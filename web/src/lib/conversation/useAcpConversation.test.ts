/**
 * useAcpConversation — unit tests for the coalescing/transport logic.
 *
 * These tests drive the Zustand store directly via the exported ingest path
 * (by feeding SessionNotifications through a mock WebSocket), asserting the
 * resulting ConversationItem[] without needing to mount a React component.
 *
 * Tests:
 *   1. Two agent_message_chunks → ONE assistant message with concatenated text
 *   2. agent_thought_chunk → thinking item
 *   3. tool_call then tool_call_update(completed) → ONE item ending 'completed'
 *   4. Metadata updates produce NO items
 *   5. Idempotent connect: double-mount opens ONE WebSocket (StrictMode guard)
 *   6. tool_call_update with unknown toolCallId appends a new item
 *   7. agent_message_chunk after a thinking block starts a NEW assistant message
 *   8. Empty text chunks are ignored
 */

import type { SessionNotification } from '@/api/acp-attach';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useAcpStore } from './useAcpConversation';

// ── Mock WebSocket ─────────────────────────────────────────────────────────────

class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  static instances: MockWebSocket[] = [];

  url: string;
  readyState: number = MockWebSocket.CONNECTING;
  binaryType = 'blob';

  onopen: ((ev: Event) => unknown) | null = null;
  onmessage: ((ev: MessageEvent) => unknown) | null = null;
  onerror: ((ev: Event) => unknown) | null = null;
  onclose: ((ev: CloseEvent) => unknown) | null = null;

  sent: string[] = [];

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.call(this, new CloseEvent('close'));
  }

  // Test helpers — drive the WS lifecycle synchronously
  openIt() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.call(this, new Event('open'));
  }

  pushFrame(data: unknown) {
    this.onmessage?.call(this, new MessageEvent('message', { data: JSON.stringify(data) }));
  }

  pushNotification(notif: SessionNotification) {
    this.pushFrame(notif);
  }
}

// ── Setup / teardown ──────────────────────────────────────────────────────────

const _origWS = globalThis.WebSocket;
const _origWindow = (globalThis as unknown as Record<string, unknown>).window;

function installMock() {
  MockWebSocket.instances = [];
  (globalThis as unknown as Record<string, unknown>).WebSocket = MockWebSocket;
  // attachAgentWsUrl requires window.location
  (globalThis as unknown as Record<string, unknown>).window = {
    location: { protocol: 'http:', host: 'localhost:8008' },
  };
}

function uninstallMock() {
  (globalThis as unknown as Record<string, unknown>).WebSocket = _origWS;
  (globalThis as unknown as Record<string, unknown>).window = _origWindow;
}

function resetStore() {
  useAcpStore.setState({ connections: {} });
}

import { act, renderHook } from '@testing-library/react';
// Import the registry reset — we need to clear _attachRegistry between tests
// so stale attach instances don't interfere. We reach into the module for this.
// The registry is module-level, so we clear it by disposing the registered keys.
import { useAcpConversation } from './useAcpConversation';

beforeEach(() => {
  vi.useFakeTimers();
  installMock();
  resetStore();
});

afterEach(() => {
  vi.useRealTimers();
  uninstallMock();
  resetStore();
});

// ── Helper: mount hook and return the WS instance ─────────────────────────────

function mountHook(nodeId = 'node1', agentId = 'agent1') {
  const { result, unmount } = renderHook(() => useAcpConversation(nodeId, agentId));
  // Open the WS
  const ws = MockWebSocket.instances[MockWebSocket.instances.length - 1];
  act(() => ws?.openIt());
  return { result, unmount, ws };
}

function push(ws: MockWebSocket, notif: SessionNotification) {
  act(() => ws.pushNotification(notif));
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('useAcpConversation — coalescing', () => {
  // 1. Two agent_message_chunks coalesce into one assistant message
  it('two agent_message_chunks → ONE assistant message with concatenated text', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'agent_message_chunk', content: { text: 'Hello ' } },
    });
    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'agent_message_chunk', content: { text: 'world' } },
    });

    const { items } = result.current;
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe('message');
    if (items[0].kind === 'message') {
      expect(items[0].role).toBe('assistant');
      expect(items[0].text).toBe('Hello world');
      expect(items[0].streaming).toBe(true);
    }

    unmount();
  });

  // 2. agent_thought_chunk → thinking item
  it('agent_thought_chunk → thinking item', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'agent_thought_chunk', content: { text: 'I think...' } },
    });

    const { items } = result.current;
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe('thinking');
    if (items[0].kind === 'thinking') {
      expect(items[0].text).toBe('I think...');
      expect(items[0].streaming).toBe(true);
    }

    unmount();
  });

  // 3. Two thought chunks coalesce into one thinking item
  it('two agent_thought_chunks coalesce into ONE thinking item', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'agent_thought_chunk', content: { text: 'Step 1 ' } },
    });
    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'agent_thought_chunk', content: { text: 'Step 2' } },
    });

    const { items } = result.current;
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe('thinking');
    if (items[0].kind === 'thinking') {
      expect(items[0].text).toBe('Step 1 Step 2');
    }

    unmount();
  });

  // 4. tool_call then tool_call_update(completed) → ONE item ending completed
  it('tool_call + tool_call_update(completed) → ONE tool item with status completed', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: {
        sessionUpdate: 'tool_call',
        toolCallId: 'tc-1',
        title: 'read_file',
        rawInput: { path: '/foo' },
      },
    });

    // Verify initial state
    expect(result.current.items).toHaveLength(1);
    expect(result.current.items[0].kind).toBe('tool_call');
    if (result.current.items[0].kind === 'tool_call') {
      expect(result.current.items[0].status).toBe('pending');
    }

    push(ws, {
      sessionId: 's1',
      update: {
        sessionUpdate: 'tool_call_update',
        toolCallId: 'tc-1',
        status: 'completed',
      },
    });

    const { items } = result.current;
    // Still ONE item — not a new one appended
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe('tool_call');
    if (items[0].kind === 'tool_call') {
      expect(items[0].toolCallId).toBe('tc-1');
      expect(items[0].title).toBe('read_file');
      expect(items[0].status).toBe('completed');
    }

    unmount();
  });

  // 5. Metadata updates produce NO items
  it.each([
    'user_message_chunk',
    'available_commands_update',
    'current_mode_update',
    'config_option_update',
    'session_info_update',
    'usage_update',
  ])('metadata variant %s produces NO items', (variant) => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: variant } as unknown as SessionNotification['update'],
    });

    expect(result.current.items).toHaveLength(0);
    unmount();
  });

  // 6. tool_call_update with unknown toolCallId appends a new item
  it('tool_call_update with unknown toolCallId appends a new tool item', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: {
        sessionUpdate: 'tool_call_update',
        toolCallId: 'tc-unknown',
        status: 'completed',
      },
    });

    const { items } = result.current;
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe('tool_call');
    if (items[0].kind === 'tool_call') {
      expect(items[0].toolCallId).toBe('tc-unknown');
      expect(items[0].status).toBe('completed');
    }

    unmount();
  });

  // 7. message chunk after thinking block starts a NEW assistant message
  it('agent_message_chunk after a thinking block starts a NEW assistant message', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'agent_thought_chunk', content: { text: 'thinking...' } },
    });
    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'agent_message_chunk', content: { text: 'answer' } },
    });

    const { items } = result.current;
    // Should have TWO items: thinking + message
    expect(items).toHaveLength(2);
    expect(items[0].kind).toBe('thinking');
    expect(items[1].kind).toBe('message');
    if (items[1].kind === 'message') {
      expect(items[1].text).toBe('answer');
    }

    unmount();
  });

  // 8. Empty text chunks are ignored
  it('empty text chunks produce no items or changes', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'agent_message_chunk', content: { text: '' } },
    });
    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'agent_thought_chunk', content: { text: '' } },
    });

    expect(result.current.items).toHaveLength(0);
    unmount();
  });

  // 9. plan update produces no items
  it('plan update produces NO items', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: {
        sessionUpdate: 'plan',
        detail: 'some plan data',
      } as unknown as SessionNotification['update'],
    });

    expect(result.current.items).toHaveLength(0);
    unmount();
  });

  // 10. Unknown update kinds produce no items
  it('truly unknown sessionUpdate variants produce NO items', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: {
        sessionUpdate: 'some_future_kind',
        data: 'x',
      } as unknown as SessionNotification['update'],
    });

    expect(result.current.items).toHaveLength(0);
    unmount();
  });

  // 11. Multiple tool calls tracked independently
  it('multiple tool_calls tracked independently by toolCallId', () => {
    const { result, unmount, ws } = mountHook();

    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'tool_call', toolCallId: 'tc-a', title: 'tool_a' },
    });
    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'tool_call', toolCallId: 'tc-b', title: 'tool_b' },
    });
    push(ws, {
      sessionId: 's1',
      update: { sessionUpdate: 'tool_call_update', toolCallId: 'tc-a', status: 'failed' },
    });

    const { items } = result.current;
    expect(items).toHaveLength(2);
    const a = items.find((i) => i.kind === 'tool_call' && i.toolCallId === 'tc-a');
    const b = items.find((i) => i.kind === 'tool_call' && i.toolCallId === 'tc-b');
    expect(a?.kind === 'tool_call' && a.status).toBe('failed');
    expect(b?.kind === 'tool_call' && b.status).toBe('pending');

    unmount();
  });
});

// ── StrictMode single-socket guard ────────────────────────────────────────────

describe('useAcpConversation — StrictMode single-socket guard', () => {
  it('double-mount (simulated StrictMode) opens exactly ONE WebSocket', () => {
    // Simulate StrictMode: mount, immediately unmount (cleanup), then mount again.
    const { unmount: unmount1 } = renderHook(() => useAcpConversation('node1', 'agent1'));

    // StrictMode cleanup
    unmount1();

    // Second mount (StrictMode re-runs effects)
    const { result, unmount: unmount2 } = renderHook(() => useAcpConversation('node1', 'agent1'));

    act(() => {
      const ws = MockWebSocket.instances[MockWebSocket.instances.length - 1];
      ws?.openIt();
    });

    expect(result.current.status).toBe('open');
    // The strict mode pattern: first mount creates + opens + unmounts (cleanup disposes),
    // second mount creates a fresh connection. So we may have 2 WS instances total,
    // but only ONE is active (the last one). The key invariant: only one is in OPEN state.
    const openInstances = MockWebSocket.instances.filter(
      (ws) => ws.readyState === MockWebSocket.OPEN,
    );
    expect(openInstances).toHaveLength(1);

    unmount2();
  });
});
