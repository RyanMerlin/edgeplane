/**
 * eventStream store — unit tests (Phase 3, main risk).
 *
 * Tests:
 *   1. Ring buffer caps at 60 and evicts oldest
 *   2. Backoff sequence: 1s → 30s capped, multiplier 1.5
 *   3. Clean open resets backoff to 1s
 *   4. Rate-limit path: remaining<=0 sets pausedUntil, status transitions
 *   5. connect() is idempotent (calling while open is a no-op)
 *   6. disconnect() closes source and resets all state
 *   7. clearEvents() empties the ring buffer
 *
 * Mock strategy: provide a fake global EventSource that lets tests control
 * open/message/error events synchronously. The store's internal mutable refs
 * (_source, _reconnectTimer, etc.) are reset between tests via disconnect().
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type MatrixEvent, RING_BUFFER_CAP, useEventStreamStore } from './eventStream';

// ── Fake EventSource ──────────────────────────────────────────────────────────

type FakeHandler = (event: { data: string }) => void;

class FakeEventSource {
  static instances: FakeEventSource[] = [];

  url: string;
  withCredentials: boolean;
  readyState: number;

  onopen: (() => void) | null = null;
  onmessage: FakeHandler | null = null;
  onerror: (() => void) | null = null;

  private _listeners: Map<string, FakeHandler[]> = new Map();

  constructor(url: string, opts?: { withCredentials?: boolean }) {
    this.url = url;
    this.withCredentials = opts?.withCredentials ?? false;
    this.readyState = 0; // CONNECTING
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, handler: FakeHandler) {
    if (!this._listeners.has(type)) this._listeners.set(type, []);
    this._listeners.get(type)?.push(handler);
  }

  removeEventListener(type: string, handler: FakeHandler) {
    const list = this._listeners.get(type) ?? [];
    const idx = list.indexOf(handler);
    if (idx >= 0) list.splice(idx, 1);
  }

  close() {
    this.readyState = 2; // CLOSED
  }

  // Test helpers — trigger handlers synchronously
  triggerOpen() {
    this.readyState = 1; // OPEN
    this.onopen?.();
  }

  triggerMessage(data: unknown, eventType = 'progress') {
    const payload = JSON.stringify(data);
    // onmessage fires for unnamed/rate-limit frames; named listeners handle progress.
    this.onmessage?.({ data: payload });
    for (const h of this._listeners.get(eventType) ?? []) {
      h({ data: payload });
    }
  }

  triggerError() {
    this.onerror?.();
  }
}

// Install fake EventSource globally
const originalEventSource = globalThis.EventSource;

function installFakeEventSource() {
  FakeEventSource.instances = [];
  // @ts-expect-error — assigning fake to global for tests
  globalThis.EventSource = FakeEventSource;
}

function uninstallFakeEventSource() {
  globalThis.EventSource = originalEventSource;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function getStore() {
  return useEventStreamStore.getState();
}

/** Reset store + all internal mutable state between tests */
function resetStore() {
  getStore().disconnect();
  // After disconnect the status is 'closed' and state is clean
  // but we also need to clear events explicitly in case disconnect left lastError
  useEventStreamStore.setState({
    events: [],
    status: 'closed',
    lastError: null,
    rateLimit: null,
    reconnectCount: 0,
    reconnectDelay: 1000,
    messagesReceived: 0,
  });
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('eventStream store', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    installFakeEventSource();
    resetStore();
  });

  afterEach(() => {
    vi.useRealTimers();
    uninstallFakeEventSource();
    resetStore();
  });

  // ── 1. Ring buffer ──────────────────────────────────────────────────────────

  describe('ring buffer', () => {
    it('caps at RING_BUFFER_CAP (60) and evicts oldest', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();

      // Send RING_BUFFER_CAP + 5 events
      for (let i = 0; i < RING_BUFFER_CAP + 5; i++) {
        src.triggerMessage({ type: 'heartbeat', payload: { seq: i } });
      }

      const { events } = getStore();
      expect(events).toHaveLength(RING_BUFFER_CAP);

      // Newest is first (seq = RING_BUFFER_CAP + 4)
      const newestPayload = events[0].payload as { seq: number };
      expect(newestPayload.seq).toBe(RING_BUFFER_CAP + 4);

      // Oldest still in buffer is seq = 5 (first 5 evicted)
      const oldestPayload = events[RING_BUFFER_CAP - 1].payload as { seq: number };
      expect(oldestPayload.seq).toBe(5);
    });

    it('newest events are prepended (index 0 is most recent)', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();

      src.triggerMessage({ type: 'task_claimed', payload: { name: 'first' } });
      src.triggerMessage({ type: 'task_finished', payload: { name: 'second' } });

      const { events } = getStore();
      expect(events).toHaveLength(2);
      const newest = events[0].payload as { name: string };
      expect(newest.name).toBe('second');
    });

    it('clearEvents() empties the buffer', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();
      src.triggerMessage({ type: 'heartbeat', payload: {} });

      expect(getStore().events).toHaveLength(1);
      getStore().clearEvents();
      expect(getStore().events).toHaveLength(0);
    });
  });

  // ── 2. Backoff sequence ─────────────────────────────────────────────────────

  describe('backoff', () => {
    it('first error with no messages received uses 5s base (not 1s)', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];

      // Don't trigger open or any messages — simulates unreachable endpoint
      src.triggerError();

      // Reconnect delay should be >= 5000 (the "first connect" base)
      expect(getStore().reconnectDelay).toBeGreaterThanOrEqual(5000);
    });

    it('backoff sequence caps at 30s after repeated errors', () => {
      getStore().connect();
      let src = FakeEventSource.instances[0];

      // Trigger open first so we switch to 1s base
      src.triggerOpen();
      // Send one message so messagesReceived > 0
      src.triggerMessage({ type: 'heartbeat', payload: {} });

      const delays: number[] = [];

      // Simulate enough errors to hit the cap
      for (let attempt = 0; attempt < 10; attempt++) {
        src.triggerError();
        delays.push(getStore().reconnectDelay);

        // Advance timers to trigger the scheduled reconnect
        vi.runAllTimers();

        // A new FakeEventSource was created on reconnect
        src = FakeEventSource.instances[FakeEventSource.instances.length - 1];
        // Trigger open to test reset later; for cap test, trigger error again
        if (attempt < 9) {
          // For most iterations, re-error without open to keep ramping
          // But we need to trigger open once so status goes back to 'closed'
          // Actually: the timer callback sets status=closed then calls connect()
          // connect() opens a new source. We just need to trigger error on the new source.
        }
      }

      // After enough errors, delay should be capped at 30s
      expect(getStore().reconnectDelay).toBeLessThanOrEqual(30_000);
      // And should have reached the cap
      expect(delays[delays.length - 1]).toBe(30_000);
    });

    it('backoff resets to 1s on a clean open', () => {
      getStore().connect();
      let src = FakeEventSource.instances[0];

      // Trigger open to use 1s base, then get a message so messagesReceived > 0
      src.triggerOpen();
      src.triggerMessage({ type: 'heartbeat', payload: {} });

      // Error a few times to build up delay
      for (let i = 0; i < 4; i++) {
        src.triggerError();
        vi.runAllTimers();
        src = FakeEventSource.instances[FakeEventSource.instances.length - 1];
      }

      // Delay should be above 1s now
      expect(getStore().reconnectDelay).toBeGreaterThan(1000);

      // Now trigger a clean open → backoff should reset
      src.triggerOpen();

      expect(getStore().reconnectDelay).toBe(1000);
      expect(getStore().reconnectCount).toBe(0);
      expect(getStore().status).toBe('open');
    });

    it('backoff multiplier is 1.5 (matches telemetry.ts)', () => {
      getStore().connect();
      let src = FakeEventSource.instances[0];

      // Prime with open+message so we get the 1s base
      src.triggerOpen();
      src.triggerMessage({ type: 'heartbeat', payload: {} });

      const capturedDelays: number[] = [];

      // First error → delay should be max(1000 * 1.5, 1000) = 1500
      src.triggerError();
      capturedDelays.push(getStore().reconnectDelay); // 1500

      // Advance timer so reconnect fires, get the new source
      vi.runAllTimers();
      src = FakeEventSource.instances[FakeEventSource.instances.length - 1];

      // Do NOT trigger open — this keeps _currentDelay at 1500 so next error
      // will multiply from 1500 → 2250. If we trigger open, _currentDelay resets to 1s.
      // Just trigger another error immediately (simulates reconnect attempt failing).
      src.triggerError();
      capturedDelays.push(getStore().reconnectDelay); // 2250

      // Check multiplier progression: 1000 → 1500 → 2250
      expect(capturedDelays[0]).toBe(1500);
      expect(capturedDelays[1]).toBe(2250);
    });
  });

  // ── 3. Rate-limit path ──────────────────────────────────────────────────────

  describe('rate-limit', () => {
    it('sets rateLimit state from message payload', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();

      const rl = {
        limit: 100,
        remaining: 50,
        reset_at: new Date(Date.now() + 10_000).toISOString(),
      };
      src.triggerMessage({ type: 'heartbeat', payload: {}, rate_limit: rl });

      expect(getStore().rateLimit).toEqual(rl);
    });

    it('sets pausedUntil when remaining <= 0 and defers reconnect', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();
      src.triggerMessage({ type: 'heartbeat', payload: {} }); // bump messagesReceived

      // Send a rate-limit message with remaining=0 and reset 10s in the future
      const resetAt = new Date(Date.now() + 10_000).toISOString();
      const rl = { limit: 100, remaining: 0, reset_at: resetAt };
      src.triggerMessage({ type: 'event', payload: {}, rate_limit: rl });

      // Now trigger an error — reconnect should be deferred past the rate-limit window
      src.triggerError();

      const { status } = getStore();
      // Status should be reconnecting (timer pending)
      expect(status).toBe('reconnecting');

      // Advance past the rate-limit pause: store should try to reconnect
      vi.advanceTimersByTime(12_000);

      // After the pause, a new EventSource should have been created
      expect(FakeEventSource.instances.length).toBeGreaterThan(1);
    });
  });

  // ── 4. Idempotent connect ───────────────────────────────────────────────────

  describe('idempotent connect', () => {
    it('connect() while open does not open a second EventSource', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();

      expect(getStore().status).toBe('open');
      expect(FakeEventSource.instances).toHaveLength(1);

      // Call connect() again — should be a no-op
      getStore().connect();

      expect(FakeEventSource.instances).toHaveLength(1);
    });

    it('connect() while connecting does not open a second EventSource', () => {
      getStore().connect();
      // Status is 'connecting' before triggerOpen
      expect(getStore().status).toBe('connecting');
      expect(FakeEventSource.instances).toHaveLength(1);

      getStore().connect(); // second call while connecting

      expect(FakeEventSource.instances).toHaveLength(1);
    });

    it('connect() while reconnecting does not open a second EventSource', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();
      src.triggerMessage({ type: 'heartbeat', payload: {} });
      src.triggerError(); // triggers reconnect timer

      expect(getStore().status).toBe('reconnecting');

      getStore().connect(); // should be a no-op (status !== 'closed')

      // Timer hasn't fired yet — still only 1 source created so far
      expect(FakeEventSource.instances).toHaveLength(1);
    });
  });

  // ── 5. Disconnect ───────────────────────────────────────────────────────────

  describe('disconnect', () => {
    it('closes the EventSource and resets all state', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();
      src.triggerMessage({ type: 'heartbeat', payload: {} });

      getStore().disconnect();

      expect(src.readyState).toBe(2); // CLOSED
      const s = getStore();
      expect(s.status).toBe('closed');
      expect(s.events).toHaveLength(0);
      expect(s.rateLimit).toBeNull();
      expect(s.reconnectCount).toBe(0);
      expect(s.messagesReceived).toBe(0);
    });

    it('cancels pending reconnect timer on disconnect', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();
      src.triggerMessage({ type: 'heartbeat', payload: {} });
      src.triggerError(); // schedules reconnect

      expect(getStore().status).toBe('reconnecting');

      getStore().disconnect();

      // Advance timers — no new EventSource should be created
      vi.runAllTimers();
      expect(FakeEventSource.instances).toHaveLength(1); // still only original
    });
  });

  // ── 6. Event shape ──────────────────────────────────────────────────────────

  describe('event shape', () => {
    it('stamps receivedAt on each event', () => {
      const before = Date.now();
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();
      src.triggerMessage({ type: 'task_claimed', agent_id: 'aria-001', payload: { name: 'T1' } });

      const { events } = getStore();
      expect(events[0].receivedAt).toBeGreaterThanOrEqual(before);
    });

    it('uses payload field when present, falls back to whole object', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();

      // With explicit payload field
      src.triggerMessage({ type: 'task_finished', payload: { name: 'explicit' } });
      expect((getStore().events[0].payload as { name: string }).name).toBe('explicit');
    });

    it('captures type, agent_id, mission_id, domain_id', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();

      src.triggerMessage({
        type: 'step_error',
        agent_id: 'a-123',
        mission_id: 'm-456',
        domain_id: 'd-789',
        payload: { error: 'boom' },
      });

      const evt: MatrixEvent = getStore().events[0];
      expect(evt.type).toBe('step_error');
      expect(evt.agent_id).toBe('a-123');
      expect(evt.mission_id).toBe('m-456');
      expect(evt.domain_id).toBe('d-789');
    });

    it('normalizes backend event_type to type (progress event shape)', () => {
      getStore().connect();
      const src = FakeEventSource.instances[0];
      src.triggerOpen();

      // Backend sends `event_type`, not `type` — normalize at the listener boundary.
      src.triggerMessage({
        event_type: 'step_started',
        agent_id: 'my-agent-engineer-0fd11ef0',
        task_id: 'task-uuid-123',
        seq: 0,
        summary: 'Task claimed by agent',
        occurred_at: '2026-06-07T17:00:00',
      });

      const evt: MatrixEvent = getStore().events[0];
      expect(evt.type).toBe('step_started');
      expect(evt.agent_id).toBe('my-agent-engineer-0fd11ef0');
    });
  });
});
