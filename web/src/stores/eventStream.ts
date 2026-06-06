/**
 * SSE event-stream store — Phase 3.
 *
 * Ported from web/src/lib/telemetry.ts (Svelte) → Zustand singleton.
 *
 * Design:
 *   - ONE EventSource for the whole app, regardless of how many components
 *     subscribe. connect() is idempotent; calling it while already open
 *     is a no-op (guarded by the `status` field).
 *   - Ring buffer capped at RING_BUFFER_CAP (60). Newest prepended; oldest
 *     truncated by .slice(0, cap).
 *   - Exponential backoff: 1s → 30s (cap). On first-connect with zero
 *     messages received we use a longer base (5s) to avoid rapid flicker
 *     on an unreachable endpoint — faithful to telemetry.ts.
 *   - Rate-limit awareness: when a message carries `rate_limit.remaining <= 0`
 *     we set `pausedUntil = parse(reset_at) + 1000` and skip reconnect until
 *     the pause clears.
 *   - StrictMode safety: connect() checks `status !== 'closed'` before
 *     opening a new EventSource, so the double-mount in React 19 StrictMode
 *     won't open two connections (the first mount's connect() transitions to
 *     'connecting'; the second mount's connect() sees non-'closed' and
 *     returns early).
 *
 * Event shape (inferred from telemetry.ts — not documented in OpenAPI):
 *   {
 *     id?: string
 *     event?: string         // SSE event field — used as type fallback
 *     type?: string          // primary type discriminant
 *     domain_id?: string
 *     mission_id?: string
 *     agent_id?: string
 *     status?: string
 *     payload: unknown       // inner payload object
 *     rate_limit?: { limit: number; remaining: number; reset_at: string }
 *   }
 *
 * NOTE: The /api/events/stream endpoint shape is INFERRED from telemetry.ts.
 * The backend is not documented in openapi.json. If the real shape differs,
 * update MatrixEvent and the onmessage handler accordingly.
 */

import { create } from 'zustand';

// ── Constants ─────────────────────────────────────────────────────────────────

export const RING_BUFFER_CAP = 60;
const BACKOFF_INITIAL_MS = 1000;
const BACKOFF_FIRST_CONNECT_MS = 5000; // no messages ever received → slower base
const BACKOFF_CAP_MS = 30_000;
const BACKOFF_MULTIPLIER = 1.5; // matches telemetry.ts: reconnectTimeout * 1.5

// ── Types ─────────────────────────────────────────────────────────────────────

export type RateLimit = {
  limit: number;
  remaining: number;
  reset_at: string;
};

/**
 * A single event received from the SSE stream.
 * Shape inferred from telemetry.ts — `type` preferred over `event` as the
 * type discriminant; `payload` holds the inner object.
 */
export type MatrixEvent = {
  id?: string;
  /** SSE event-type field — used as fallback when `type` is absent */
  event?: string;
  type?: string;
  domain_id?: string;
  mission_id?: string;
  agent_id?: string;
  status?: string;
  payload: unknown;
  rate_limit?: RateLimit;
  /** Wall-clock timestamp set by the client at receipt */
  receivedAt: number;
};

export type ConnectionStatus = 'closed' | 'connecting' | 'open' | 'reconnecting';

export interface EventStreamState {
  /** Ring buffer — newest first, capped at RING_BUFFER_CAP */
  events: MatrixEvent[];
  status: ConnectionStatus;
  /** Last error message, if any */
  lastError: string | null;
  /** Current rate-limit metadata from the most recent message */
  rateLimit: RateLimit | null;
  /** Reconnect attempt count (resets to 0 on a clean open) */
  reconnectCount: number;
  /** Next reconnect delay in ms (for observability/tests) */
  reconnectDelay: number;
  /** Total messages received in this session (used for backoff base selection) */
  messagesReceived: number;

  connect: () => void;
  disconnect: () => void;
  clearEvents: () => void;
}

// ── Internal mutable state (not in Zustand — timers etc. must live outside) ──

let _source: EventSource | null = null;
let _reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let _pausedUntil = 0;
let _currentDelay = BACKOFF_INITIAL_MS;
let _messagesReceived = 0; // mirrors store.messagesReceived for backoff logic

function clearReconnectTimer() {
  if (_reconnectTimer !== null) {
    clearTimeout(_reconnectTimer);
    _reconnectTimer = null;
  }
}

// ── Store ─────────────────────────────────────────────────────────────────────

export const useEventStreamStore = create<EventStreamState>((set, get) => ({
  events: [],
  status: 'closed',
  lastError: null,
  rateLimit: null,
  reconnectCount: 0,
  reconnectDelay: BACKOFF_INITIAL_MS,
  messagesReceived: 0,

  connect() {
    const { status } = get();

    // Idempotent: don't open a second connection while one is live or pending.
    if (status !== 'closed') return;

    // Rate-limit pause: if we're in a pause window, schedule re-attempt.
    if (Date.now() < _pausedUntil) {
      const remaining = _pausedUntil - Date.now();
      set({ status: 'reconnecting', reconnectDelay: remaining });
      clearReconnectTimer();
      _reconnectTimer = setTimeout(() => {
        _reconnectTimer = null;
        set({ status: 'closed' });
        get().connect();
      }, remaining);
      return;
    }

    clearReconnectTimer();
    set({ status: 'connecting' });

    let source: EventSource;
    try {
      source = new EventSource('/api/events/stream', { withCredentials: true });
    } catch (err) {
      set({
        status: 'closed',
        lastError: err instanceof Error ? err.message : 'EventSource open failed',
      });
      return;
    }

    _source = source;

    source.onopen = () => {
      // Successful open → reset backoff.
      _currentDelay = BACKOFF_INITIAL_MS;
      set({
        status: 'open',
        lastError: null,
        reconnectCount: 0,
        reconnectDelay: BACKOFF_INITIAL_MS,
      });
    };

    // The generic `onmessage` handler fires for unnamed events (event: message
    // or no event field). We use it to handle the rate-limit field only,
    // mirroring telemetry.ts which registers BOTH onmessage and addEventListener
    // ('message'). For named events (e.g. `event: task_claimed`) only the
    // addEventListener path fires.
    source.onmessage = (msg: MessageEvent<string>) => {
      _messagesReceived++;
      try {
        const parsed = JSON.parse(msg.data) as Record<string, unknown>;
        const rl = parsed.rate_limit as RateLimit | undefined;
        if (rl) {
          if (rl.remaining <= 0) {
            _pausedUntil = Date.parse(rl.reset_at) + 1000;
          }
          set({ rateLimit: rl });
        }
      } catch {
        // Ignore parse errors on the rate-limit probe.
      }
    };

    // Named 'message' listener appends to the ring buffer.
    source.addEventListener('message', (msg: MessageEvent<string>) => {
      _messagesReceived++;
      try {
        const parsed = JSON.parse(msg.data) as Record<string, unknown>;
        const event: MatrixEvent = {
          ...(parsed as Omit<MatrixEvent, 'receivedAt'>),
          receivedAt: Date.now(),
          payload: (parsed.payload ?? parsed) as unknown,
        };

        set((state) => ({
          events: [event, ...state.events].slice(0, RING_BUFFER_CAP),
          messagesReceived: state.messagesReceived + 1,
        }));

        // Check rate-limit inside the buffer handler too.
        const rl = parsed.rate_limit as RateLimit | undefined;
        if (rl) {
          if (rl.remaining <= 0) {
            _pausedUntil = Date.parse(rl.reset_at) + 1000;
          }
          set({ rateLimit: rl });
        }
      } catch {
        // Ignore malformed events.
      }
    });

    source.onerror = () => {
      set((state) => ({
        status: 'reconnecting',
        lastError: 'Connection lost',
        reconnectCount: state.reconnectCount + 1,
      }));

      source.close();
      _source = null;

      // If no messages have ever been received, use the longer initial backoff
      // to avoid rapid flicker (faithful to telemetry.ts).
      const baseTimeout = _messagesReceived === 0 ? BACKOFF_FIRST_CONNECT_MS : BACKOFF_INITIAL_MS;
      _currentDelay = Math.min(
        Math.max(_currentDelay * BACKOFF_MULTIPLIER, baseTimeout),
        BACKOFF_CAP_MS,
      );

      set({ reconnectDelay: _currentDelay });

      clearReconnectTimer();
      _reconnectTimer = setTimeout(() => {
        _reconnectTimer = null;
        // Transition back to 'closed' so the idempotent connect() guard passes.
        set({ status: 'closed' });
        get().connect();
      }, _currentDelay);
    };
  },

  disconnect() {
    clearReconnectTimer();
    _source?.close();
    _source = null;
    _pausedUntil = 0;
    _currentDelay = BACKOFF_INITIAL_MS;
    _messagesReceived = 0;
    set({
      events: [],
      status: 'closed',
      lastError: 'stream stopped',
      rateLimit: null,
      reconnectCount: 0,
      reconnectDelay: BACKOFF_INITIAL_MS,
      messagesReceived: 0,
    });
  },

  clearEvents() {
    set({ events: [] });
  },
}));
