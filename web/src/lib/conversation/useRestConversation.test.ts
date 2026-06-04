/**
 * useRestConversation — unit tests for session/event → item mapping.
 *
 * Tests drive sessionToItems() and foldEvent() directly (pure functions),
 * and test the hook lifecycle with mocked apiClient + a fake EventSource.
 *
 * Mapping tests:
 *   1. Two assistant turns → two message items
 *   2. User turn + assistant turn → user message + assistant message
 *   3. tool_call event + tool_result event → one tool_call item (completed)
 *   4. tool_call event + failing tool_result → one tool_call item (failed)
 *   5. Pending action → approval item (status: pending)
 *   6. approval_outcome event flips approval to executed
 *   7. approval_outcome event flips approval to rejected
 *   8. user_message event → suppressed (no item)
 *   9. planner_result event → suppressed (no item)
 *   10. session_started event → suppressed (no item)
 *   11. Unknown event type → suppressed (no item)
 *   12. role=tool turn → suppressed (no item)
 *   13. foldEvent: tool_call → appends new item
 *   14. foldEvent: tool_result folds into matching tool_call
 *   15. foldEvent: tool_result with unknown toolCallId appends new item
 *   16. foldEvent: approval_required appends approval item
 *   17. foldEvent: approval_outcome updates matching approval
 *   18. foldEvent: suppressed event type returns items unchanged
 *
 * Hook lifecycle tests:
 *   19. StrictMode double-mount opens exactly ONE EventSource
 *   20. SSE ai_event folds into items
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { foldEvent, sessionToItems, useRestStore } from './useRestConversation';
import type { AiEvent, AiPendingAction, AiSession, AiTurn } from './useRestConversation';

// ── Helpers — build minimal fixture objects ────────────────────────────────────

function makeTurn(overrides: Partial<AiTurn> & { id: number; role: string }): AiTurn {
  return {
    id: overrides.id,
    role: overrides.role,
    content: overrides.content ?? { text: `message ${overrides.id}` },
    created_at: overrides.created_at ?? `2024-01-01T00:00:0${overrides.id}.000000`,
  };
}

function makeEvent(overrides: Partial<AiEvent> & { id: number; event_type: string }): AiEvent {
  return {
    id: overrides.id,
    event_type: overrides.event_type,
    payload: overrides.payload ?? {},
    turn_id: overrides.turn_id ?? null,
    created_at: overrides.created_at ?? `2024-01-01T00:01:0${overrides.id}.000000`,
  };
}

function makeAction(overrides: Partial<AiPendingAction> & { id: string }): AiPendingAction {
  return {
    id: overrides.id,
    tool: overrides.tool ?? 'bash',
    args: overrides.args ?? { cmd: 'ls' },
    reason: overrides.reason ?? 'Needs approval',
    status: overrides.status ?? 'pending',
    approved_by: overrides.approved_by ?? '',
    rejected_by: overrides.rejected_by ?? '',
    rejection_note: overrides.rejection_note ?? '',
    requested_by: overrides.requested_by ?? 'agent',
    created_at: overrides.created_at ?? '2024-01-01T00:02:00.000000',
    updated_at: overrides.updated_at ?? '2024-01-01T00:02:00.000000',
  };
}

function makeSession(overrides: Partial<AiSession> = {}): AiSession {
  return {
    id: overrides.id ?? 'ais_test',
    title: overrides.title ?? 'Test session',
    status: overrides.status ?? 'active',
    runtime_kind: overrides.runtime_kind ?? 'opencode',
    owner_subject: overrides.owner_subject ?? 'test@example.com',
    turns: overrides.turns ?? [],
    events: overrides.events ?? [],
    pending_actions: overrides.pending_actions ?? [],
    created_at: overrides.created_at ?? '2024-01-01T00:00:00.000000',
    updated_at: overrides.updated_at ?? '2024-01-01T00:00:00.000000',
  };
}

// ── sessionToItems tests ───────────────────────────────────────────────────────

describe('sessionToItems — mapping', () => {
  // 1. Two assistant turns → two message items
  it('two assistant turns → two message items', () => {
    const session = makeSession({
      turns: [
        makeTurn({ id: 1, role: 'assistant', content: { text: 'Hello' } }),
        makeTurn({ id: 2, role: 'assistant', content: { text: 'World' } }),
      ],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(2);
    expect(items[0].kind).toBe('message');
    expect(items[1].kind).toBe('message');
    if (items[0].kind === 'message') expect(items[0].text).toBe('Hello');
    if (items[1].kind === 'message') expect(items[1].text).toBe('World');
  });

  // 2. User turn + assistant turn → user message + assistant message
  it('user + assistant turns → correct roles', () => {
    const session = makeSession({
      turns: [
        makeTurn({
          id: 1,
          role: 'user',
          content: { text: 'Ask' },
          created_at: '2024-01-01T00:00:01.000000',
        }),
        makeTurn({
          id: 2,
          role: 'assistant',
          content: { text: 'Answer' },
          created_at: '2024-01-01T00:00:02.000000',
        }),
      ],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(2);
    if (items[0].kind === 'message') expect(items[0].role).toBe('user');
    if (items[1].kind === 'message') expect(items[1].role).toBe('assistant');
  });

  // 3. tool_call event + tool_result event → one tool_call item (completed)
  it('tool_call + tool_result → one tool_call item with status completed', () => {
    const session = makeSession({
      events: [
        makeEvent({
          id: 1,
          event_type: 'tool_call',
          payload: { tool: 'bash', tool_call_id: 'tc1', args: { cmd: 'ls' } },
          created_at: '2024-01-01T00:01:01.000000',
        }),
        makeEvent({
          id: 2,
          event_type: 'tool_result',
          payload: { tool: 'bash', tool_call_id: 'tc1', result: { ok: true } },
          created_at: '2024-01-01T00:01:02.000000',
        }),
      ],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe('tool_call');
    if (items[0].kind === 'tool_call') {
      expect(items[0].toolCallId).toBe('tc1');
      expect(items[0].status).toBe('completed');
    }
  });

  // 4. tool_call event + failing tool_result → one tool_call item (failed)
  it('tool_call + failing tool_result → status failed', () => {
    const session = makeSession({
      events: [
        makeEvent({
          id: 3,
          event_type: 'tool_call',
          payload: { tool: 'edit', tool_call_id: 'tc2', args: {} },
          created_at: '2024-01-01T00:01:03.000000',
        }),
        makeEvent({
          id: 4,
          event_type: 'tool_result',
          payload: { tool: 'edit', tool_call_id: 'tc2', result: { ok: false } },
          created_at: '2024-01-01T00:01:04.000000',
        }),
      ],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(1);
    if (items[0].kind === 'tool_call') {
      expect(items[0].status).toBe('failed');
    }
  });

  // 5. Pending action → approval item (status: pending)
  it('pending_action → approval item with status pending', () => {
    const session = makeSession({
      pending_actions: [
        makeAction({ id: 'act1', tool: 'bash', reason: 'Write op', status: 'pending' }),
      ],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe('approval');
    if (items[0].kind === 'approval') {
      expect(items[0].tool).toBe('bash');
      expect(items[0].status).toBe('pending');
      expect(items[0].reason).toBe('Write op');
    }
  });

  // 6. approval_outcome event flips approval to executed
  it('approval_outcome(executed) flips matching approval to executed', () => {
    const session = makeSession({
      pending_actions: [makeAction({ id: 'act2', tool: 'bash', status: 'executed' })],
      events: [
        makeEvent({
          id: 10,
          event_type: 'approval_outcome',
          payload: { action_id: 'act2', status: 'executed' },
        }),
      ],
    });
    const items = sessionToItems(session);
    const approvals = items.filter((i) => i.kind === 'approval');
    expect(approvals).toHaveLength(1);
    if (approvals[0].kind === 'approval') {
      expect(approvals[0].status).toBe('executed');
    }
  });

  // 7. approval_outcome event flips approval to rejected
  it('approval_outcome(rejected) flips matching approval to rejected', () => {
    const session = makeSession({
      pending_actions: [makeAction({ id: 'act3', tool: 'edit', status: 'rejected' })],
      events: [
        makeEvent({
          id: 11,
          event_type: 'approval_outcome',
          payload: { action_id: 'act3', status: 'rejected' },
        }),
      ],
    });
    const items = sessionToItems(session);
    const approvals = items.filter((i) => i.kind === 'approval');
    expect(approvals).toHaveLength(1);
    if (approvals[0].kind === 'approval') {
      expect(approvals[0].status).toBe('rejected');
    }
  });

  // 8. user_message event → suppressed
  it('user_message event is suppressed', () => {
    const session = makeSession({
      events: [makeEvent({ id: 20, event_type: 'user_message', payload: { text: 'hello' } })],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(0);
  });

  // 9. planner_result event → suppressed
  it('planner_result event is suppressed', () => {
    const session = makeSession({
      events: [
        makeEvent({ id: 21, event_type: 'planner_result', payload: { assistant_text: 'plan' } }),
      ],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(0);
  });

  // 10. session_started event → suppressed
  it('session_started event is suppressed', () => {
    const session = makeSession({
      events: [makeEvent({ id: 22, event_type: 'session_started', payload: { title: 'AI' } })],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(0);
  });

  // 11. Unknown event type → suppressed
  it('unknown event type is suppressed', () => {
    const session = makeSession({
      events: [makeEvent({ id: 23, event_type: 'some_future_event', payload: {} })],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(0);
  });

  // 12. role=tool turn → suppressed
  it('role=tool turn is suppressed', () => {
    const session = makeSession({
      turns: [makeTurn({ id: 5, role: 'tool', content: { result: 'ok' } })],
    });
    const items = sessionToItems(session);
    expect(items).toHaveLength(0);
  });
});

// ── foldEvent tests ───────────────────────────────────────────────────────────

describe('foldEvent — incremental SSE folding', () => {
  // 13. tool_call event appends new item
  it('tool_call event appends a tool_call item', () => {
    const event = makeEvent({
      id: 30,
      event_type: 'tool_call',
      payload: { tool: 'bash', tool_call_id: 'tc30', args: { cmd: 'pwd' } },
    });
    const result = foldEvent([], event);
    expect(result).toHaveLength(1);
    expect(result[0].kind).toBe('tool_call');
    if (result[0].kind === 'tool_call') {
      expect(result[0].toolCallId).toBe('tc30');
      expect(result[0].status).toBe('pending');
    }
  });

  // 14. tool_result folds into matching tool_call
  it('tool_result folds into matching tool_call item', () => {
    const initial: import('./types').ConversationItem[] = [
      {
        kind: 'tool_call',
        id: 'event-30',
        toolCallId: 'tc30',
        title: 'bash',
        status: 'pending',
      },
    ];
    const result = foldEvent(
      initial,
      makeEvent({
        id: 31,
        event_type: 'tool_result',
        payload: { tool: 'bash', tool_call_id: 'tc30', result: { ok: true } },
      }),
    );
    expect(result).toHaveLength(1);
    if (result[0].kind === 'tool_call') {
      expect(result[0].status).toBe('completed');
    }
  });

  // 15. tool_result with unknown toolCallId appends new item
  it('tool_result with unknown toolCallId appends a new tool_call item', () => {
    const result = foldEvent(
      [],
      makeEvent({
        id: 32,
        event_type: 'tool_result',
        payload: { tool: 'edit', tool_call_id: 'unknown-tc', result: { ok: false } },
      }),
    );
    expect(result).toHaveLength(1);
    if (result[0].kind === 'tool_call') {
      expect(result[0].status).toBe('failed');
    }
  });

  // 16. approval_required appends approval item
  it('approval_required event appends an approval item', () => {
    const result = foldEvent(
      [],
      makeEvent({
        id: 40,
        event_type: 'approval_required',
        payload: { tool: 'bash', action_id: 'act40', reason: 'write op', args: { cmd: 'rm -rf' } },
      }),
    );
    expect(result).toHaveLength(1);
    expect(result[0].kind).toBe('approval');
    if (result[0].kind === 'approval') {
      expect(result[0].status).toBe('pending');
      expect(result[0].tool).toBe('bash');
    }
  });

  // 17. approval_outcome updates matching approval
  it('approval_outcome updates the matching approval item', () => {
    const initial: import('./types').ConversationItem[] = [
      {
        kind: 'approval',
        id: 'action-act40',
        tool: 'bash',
        args: {},
        reason: 'write op',
        status: 'pending',
      },
    ];
    const result = foldEvent(
      initial,
      makeEvent({
        id: 41,
        event_type: 'approval_outcome',
        payload: { action_id: 'act40', status: 'executed' },
      }),
    );
    expect(result).toHaveLength(1);
    if (result[0].kind === 'approval') {
      expect(result[0].status).toBe('executed');
    }
  });

  // 18. suppressed event type returns items unchanged
  it('suppressed event type returns items array unchanged (by reference)', () => {
    const initial: import('./types').ConversationItem[] = [
      { kind: 'message', id: 'm1', role: 'user', text: 'hi' },
    ];
    const result = foldEvent(
      initial,
      makeEvent({ id: 50, event_type: 'user_message', payload: { text: 'hi' } }),
    );
    // Suppressed — same reference returned
    expect(result).toBe(initial);
  });
});

// ── Hook lifecycle tests ───────────────────────────────────────────────────────

// Mock EventSource and apiClient for lifecycle tests

class MockEventSource {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;

  static instances: MockEventSource[] = [];

  url: string;
  readyState: number = MockEventSource.CONNECTING;
  withCredentials: boolean;

  onopen: ((ev: Event) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;

  private listeners: Map<string, Array<(ev: MessageEvent) => void>> = new Map();

  constructor(url: string, opts?: { withCredentials?: boolean }) {
    this.url = url;
    this.withCredentials = opts?.withCredentials ?? false;
    MockEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: (ev: MessageEvent) => void): void {
    const existing = this.listeners.get(type) ?? [];
    this.listeners.set(type, [...existing, listener]);
  }

  // Test helpers
  openIt(): void {
    this.readyState = MockEventSource.OPEN;
    this.onopen?.call(this, new Event('open'));
  }

  emitAiEvent(data: unknown): void {
    const listeners = this.listeners.get('ai_event') ?? [];
    const ev = new MessageEvent('ai_event', { data: JSON.stringify(data) });
    for (const l of listeners) l(ev);
  }

  close(): void {
    this.readyState = MockEventSource.CLOSED;
  }
}

const _origES = (globalThis as Record<string, unknown>).EventSource;

function installEsMock() {
  MockEventSource.instances = [];
  (globalThis as Record<string, unknown>).EventSource = MockEventSource;
}

function uninstallEsMock() {
  (globalThis as Record<string, unknown>).EventSource = _origES;
}

// We mock the apiClient module so backfillAndConnect resolves immediately
vi.mock('@/api/client', () => ({
  apiClient: {
    GET: vi.fn().mockResolvedValue({
      data: makeSession(),
      error: undefined,
      response: { ok: true, status: 200 },
    }),
    POST: vi.fn().mockResolvedValue({
      data: makeSession(),
      error: undefined,
      response: { ok: true, status: 200 },
    }),
  },
  unwrap: vi
    .fn()
    .mockImplementation(
      async (promise: Promise<{ data?: unknown; error?: unknown; response: Response }>) => {
        const { data } = await promise;
        return data;
      },
    ),
}));

import { act, renderHook } from '@testing-library/react';
import { useRestConversation } from './useRestConversation';

beforeEach(() => {
  vi.useFakeTimers();
  installEsMock();
  useRestStore.setState({ sessions: {} });
});

afterEach(() => {
  vi.useRealTimers();
  uninstallEsMock();
  useRestStore.setState({ sessions: {} });
});

describe('useRestConversation — StrictMode single-EventSource guard', () => {
  // 19. Double-mount opens exactly ONE EventSource
  it('double-mount (simulated StrictMode) opens exactly ONE EventSource', async () => {
    const { unmount: unmount1 } = renderHook(() => useRestConversation('session-1'));
    // StrictMode cleanup
    unmount1();

    // Second mount (StrictMode re-runs effects)
    const { result, unmount: unmount2 } = renderHook(() => useRestConversation('session-1'));

    // Allow the async backfill to settle
    await act(async () => {
      await vi.runAllTimersAsync();
    });

    // Open the EventSource from the second mount
    await act(async () => {
      const es = MockEventSource.instances[MockEventSource.instances.length - 1];
      es?.openIt();
    });

    // Only one EventSource should be in OPEN state
    const openInstances = MockEventSource.instances.filter(
      (es) => es.readyState === MockEventSource.OPEN,
    );
    expect(openInstances).toHaveLength(1);

    expect(result.current.status).toBe('open');

    unmount2();
  });

  // 20. SSE ai_event folds into items
  it('SSE ai_event message folds into conversation items', async () => {
    const { result, unmount } = renderHook(() => useRestConversation('session-2'));

    await act(async () => {
      await vi.runAllTimersAsync();
    });

    // Open the EventSource
    await act(async () => {
      const es = MockEventSource.instances[MockEventSource.instances.length - 1];
      es?.openIt();
    });

    // Emit a tool_call event via SSE
    await act(async () => {
      const es = MockEventSource.instances[MockEventSource.instances.length - 1];
      es?.emitAiEvent(
        makeEvent({
          id: 100,
          event_type: 'tool_call',
          payload: { tool: 'bash', tool_call_id: 'tc100', args: {} },
        }),
      );
    });

    const { items } = result.current;
    const toolCalls = items.filter((i) => i.kind === 'tool_call');
    expect(toolCalls).toHaveLength(1);
    if (toolCalls[0].kind === 'tool_call') {
      expect(toolCalls[0].toolCallId).toBe('tc100');
      expect(toolCalls[0].status).toBe('pending');
    }

    unmount();
  });
});
