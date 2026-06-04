/**
 * REST conversation hook — Zustand-backed, StrictMode-safe.
 *
 * Mirrors the structure of useAcpConversation but drives the AI session REST
 * endpoints + the SSE stream endpoint instead of a WebSocket.
 *
 * Lifecycle:
 *   1. On mount: GET /api/ai/sessions/{id} → backfill items from turns/events/pending_actions.
 *   2. Open SSE stream at /api/ai/sessions/{id}/stream?after_id=<lastEventId>.
 *      Each "ai_event" SSE message carries a JSON AiEvent — fold it into the item list.
 *   3. Mutations (send/approve/reject) call the corresponding REST endpoints; the response
 *      is the updated AiSession — rebuild items from it (single reconcile path).
 *
 * StrictMode safety: an EventSource registry (keyed by sessionId) prevents a second
 * EventSource from opening on the double-mount. On unmount the registry entry is removed
 * and the EventSource is closed. The second mount creates a fresh connection — same
 * pattern as AcpAttach / the _attachRegistry in useAcpConversation.
 *
 * Event-type → item-kind mapping (ported from web/src/routes/ai/+page.svelte):
 *
 *   AiTurn (role=user|assistant)  → message item
 *   AiTurn (role=tool)            → suppressed (folded into tool_call result)
 *   AiEvent event_type:
 *     "tool_call"                 → tool_call item (status: 'pending')
 *     "tool_result"               → update matching tool_call to 'completed'/'failed';
 *                                   appends new tool_call if toolCallId unseen
 *     "approval_required"         → approval item (status: 'pending')
 *     "approval_outcome"          → update matching approval to 'executed'/'rejected'
 *     "user_message"              → suppressed (the user turn already covers it)
 *     "planner_result"            → suppressed (metadata)
 *     "session_started"           → suppressed (metadata)
 *     everything else             → suppressed (forward-compat)
 *
 *   AiPendingAction (status=pending)  → approval item (primary source; events supplement)
 *   AiPendingAction (status!=pending) → approval item with resolved status
 *
 * NOTE: The Svelte console polls (refetchInterval: 2500ms) rather than streaming. We
 * prefer the SSE stream (/api/ai/sessions/{id}/stream) for live updates because it
 * avoids repeated full-session fetches. The SSE stream emits individual AiEvent objects;
 * `after_id` is advanced as events arrive to avoid double-applying. If the stream is
 * unavailable (EventSource error and CLOSED state) we fall back to a 3-second polling
 * interval using a full GET /api/ai/sessions/{id}.
 *
 * Note on since_event_id: the typed schema for GET /api/ai/sessions/{id} has query?: never,
 * so we always fetch the full session. For polling we do the same. The SSE stream uses
 * after_id as a query param (hand-coded EventSource URL, not the typed client).
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { useEffect, useRef } from 'react';
import { create } from 'zustand';
import type { ConversationItem, TransportStatus } from './types';

// ── Schema aliases ─────────────────────────────────────────────────────────────

type AiSession = components['schemas']['AiSession'];
type AiEvent = components['schemas']['AiEvent'];
type AiTurn = components['schemas']['AiTurn'];
type AiPendingAction = components['schemas']['AiPendingAction'];

// ── Suppressed event types ─────────────────────────────────────────────────────

const SUPPRESSED_EVENTS = new Set(['user_message', 'planner_result', 'session_started']);

// ── ID generation ──────────────────────────────────────────────────────────────

let _nextId = 0;
function nextId(): string {
  return `rest-${_nextId++}`;
}

// ── Mapping helpers ────────────────────────────────────────────────────────────

function payloadOf(event: AiEvent): Record<string, unknown> {
  if (event.payload && typeof event.payload === 'object') {
    return event.payload as Record<string, unknown>;
  }
  return {};
}

function textOfContent(content: unknown): string {
  if (!content || typeof content !== 'object') return '';
  const c = content as Record<string, unknown>;
  if (typeof c.text === 'string') return c.text;
  return JSON.stringify(content);
}

/**
 * Map an AiTurn to a ConversationItem.
 * role=tool turns are suppressed — they're folded into tool_call results via events.
 */
function turnToItem(turn: AiTurn): ConversationItem | null {
  if (turn.role === 'tool') return null;
  const role = turn.role === 'assistant' ? 'assistant' : 'user';
  return {
    kind: 'message',
    id: `turn-${turn.id}`,
    role,
    text: textOfContent(turn.content),
  };
}

/**
 * Resolve an AiPendingAction status to the approval item status.
 */
function resolveApprovalStatus(status: string): 'pending' | 'executed' | 'rejected' {
  if (status === 'executed') return 'executed';
  if (status === 'rejected') return 'rejected';
  return 'pending';
}

/**
 * Map a pending_action to an approval item.
 */
function actionToItem(action: AiPendingAction): ConversationItem {
  return {
    kind: 'approval',
    id: `action-${action.id}`,
    tool: action.tool,
    args: action.args,
    reason: action.reason || 'No reason provided',
    status: resolveApprovalStatus(action.status),
  };
}

/**
 * Build the full item list from an AiSession snapshot.
 *
 * Order: mix turns and events by created_at, interleaving them chronologically.
 * Pending actions are reconciled at the end — the pending_actions array is the
 * authoritative source for approval state (events supplement ordering/history).
 */
export function sessionToItems(session: AiSession): ConversationItem[] {
  const items: ConversationItem[] = [];

  // Merge turns + events by timestamp for chronological ordering
  type Sortable =
    | { kind: 'turn'; data: AiTurn; ts: number }
    | { kind: 'event'; data: AiEvent; ts: number };

  const sortable: Sortable[] = [];

  for (const turn of session.turns ?? []) {
    sortable.push({ kind: 'turn', data: turn, ts: new Date(turn.created_at).getTime() });
  }
  for (const event of session.events ?? []) {
    sortable.push({ kind: 'event', data: event, ts: new Date(event.created_at).getTime() });
  }

  // Stable sort (turns before events at same timestamp)
  sortable.sort((a, b) => a.ts - b.ts);

  // Track tool_call items by toolCallId for result folding
  const toolCallByCallId = new Map<string, string>(); // toolCallId → item id

  for (const entry of sortable) {
    if (entry.kind === 'turn') {
      const item = turnToItem(entry.data);
      if (item) items.push(item);
      continue;
    }

    // entry.kind === 'event'
    const event = entry.data;
    if (SUPPRESSED_EVENTS.has(event.event_type)) continue;

    const payload = payloadOf(event);

    if (event.event_type === 'tool_call') {
      const toolCallId = String(payload.tool_call_id ?? payload.id ?? `tc-${event.id}`);
      const itemId = `event-${event.id}`;
      toolCallByCallId.set(toolCallId, itemId);
      items.push({
        kind: 'tool_call',
        id: itemId,
        toolCallId,
        title: String(payload.tool ?? 'tool'),
        status: 'pending',
        rawInput: payload.args,
      });
      continue;
    }

    if (event.event_type === 'tool_result') {
      const toolCallId = String(payload.tool_call_id ?? payload.id ?? '');
      const existingItemId = toolCallByCallId.get(toolCallId);
      const ok = Boolean((payload.result as { ok?: boolean } | undefined)?.ok ?? true);
      const newStatus: 'completed' | 'failed' = ok ? 'completed' : 'failed';

      if (existingItemId) {
        const idx = items.findIndex((i) => i.id === existingItemId);
        if (idx >= 0) {
          const existing = items[idx];
          if (existing && existing.kind === 'tool_call') {
            items[idx] = { ...existing, status: newStatus };
          }
        }
      } else {
        items.push({
          kind: 'tool_call',
          id: `event-${event.id}`,
          toolCallId,
          title: String(payload.tool ?? 'tool'),
          status: newStatus,
        });
      }
      continue;
    }

    if (event.event_type === 'approval_required') {
      items.push({
        kind: 'approval',
        id: `evt-approval-${event.id}`,
        tool: String(payload.tool ?? 'action'),
        args: payload.args ?? {},
        reason: String(payload.reason ?? 'Approval required'),
        status: 'pending',
      });
      continue;
    }

    if (event.event_type === 'approval_outcome') {
      const actionId = String(payload.action_id ?? '');
      const outcomeStatus = String(payload.status ?? '');
      const resolvedStatus: 'executed' | 'rejected' =
        outcomeStatus === 'rejected' ? 'rejected' : 'executed';

      const targetByActionId = `action-${actionId}`;
      const idx = items.findIndex(
        (i) =>
          i.kind === 'approval' &&
          (i.id === targetByActionId || (actionId && i.id.includes(actionId))),
      );
      if (idx >= 0) {
        const existing = items[idx];
        if (existing && existing.kind === 'approval') {
          items[idx] = { ...existing, status: resolvedStatus };
        }
      }
    }
    // All other event types: suppressed (forward-compat) — no else branch needed
  }

  // Reconcile pending_actions as the authoritative approval state.
  // De-duplicate: if an item with the same action id already exists (from events),
  // update its status; otherwise append.
  const existingApprovalIds = new Set(items.filter((i) => i.kind === 'approval').map((i) => i.id));
  for (const action of session.pending_actions ?? []) {
    const itemId = `action-${action.id}`;
    if (existingApprovalIds.has(itemId)) {
      const idx = items.findIndex((i) => i.id === itemId);
      if (idx >= 0) {
        const existing = items[idx];
        if (existing && existing.kind === 'approval') {
          items[idx] = { ...existing, status: resolveApprovalStatus(action.status) };
        }
      }
    } else {
      items.push(actionToItem(action));
    }
  }

  return items;
}

// ── Fold a single SSE AiEvent into an existing items array (incremental) ───────

export function foldEvent(items: ConversationItem[], event: AiEvent): ConversationItem[] {
  if (SUPPRESSED_EVENTS.has(event.event_type)) return items;

  const payload = payloadOf(event);
  const next = [...items];

  if (event.event_type === 'tool_call') {
    const toolCallId = String(payload.tool_call_id ?? payload.id ?? `tc-${event.id}`);
    next.push({
      kind: 'tool_call',
      id: `event-${event.id}`,
      toolCallId,
      title: String(payload.tool ?? 'tool'),
      status: 'pending',
      rawInput: payload.args,
    });
    return next;
  }

  if (event.event_type === 'tool_result') {
    const toolCallId = String(payload.tool_call_id ?? payload.id ?? '');
    const ok = Boolean((payload.result as { ok?: boolean } | undefined)?.ok ?? true);
    const newStatus: 'completed' | 'failed' = ok ? 'completed' : 'failed';
    const idx = next.findIndex((i) => i.kind === 'tool_call' && i.toolCallId === toolCallId);
    if (idx >= 0) {
      const existing = next[idx];
      if (existing && existing.kind === 'tool_call') {
        next[idx] = { ...existing, status: newStatus };
      }
    } else {
      next.push({
        kind: 'tool_call',
        id: nextId(),
        toolCallId,
        title: String(payload.tool ?? 'tool'),
        status: newStatus,
      });
    }
    return next;
  }

  if (event.event_type === 'approval_required') {
    next.push({
      kind: 'approval',
      id: `evt-approval-${event.id}`,
      tool: String(payload.tool ?? 'action'),
      args: payload.args ?? {},
      reason: String(payload.reason ?? 'Approval required'),
      status: 'pending',
    });
    return next;
  }

  if (event.event_type === 'approval_outcome') {
    const actionId = String(payload.action_id ?? '');
    const outcomeStatus = String(payload.status ?? '');
    const resolvedStatus: 'executed' | 'rejected' =
      outcomeStatus === 'rejected' ? 'rejected' : 'executed';
    const targetId = `action-${actionId}`;
    const idx = next.findIndex(
      (i) => i.kind === 'approval' && (i.id === targetId || (actionId && i.id.includes(actionId))),
    );
    if (idx >= 0) {
      const existing = next[idx];
      if (existing && existing.kind === 'approval') {
        next[idx] = { ...existing, status: resolvedStatus };
      }
    }
    return next;
  }

  // All other event types: suppressed
  return items;
}

// ── Per-session Zustand store ──────────────────────────────────────────────────

export interface RestSlice {
  items: ConversationItem[];
  status: TransportStatus;
  /** Last event id seen from SSE — used as after_id on reconnect. */
  lastEventId: number;
  /** Polling fallback timer id */
  pollTimer: ReturnType<typeof setInterval> | null;
}

type RestStoreState = {
  sessions: Record<string, RestSlice>;
  _setSlice(key: string, patch: Partial<RestSlice>): void;
  _setItems(key: string, items: ConversationItem[]): void;
};

export const useRestStore = create<RestStoreState>((set) => ({
  sessions: {},

  _setSlice(key, patch) {
    set((state) => ({
      sessions: {
        ...state.sessions,
        [key]: {
          ...(state.sessions[key] ?? {
            items: [],
            status: 'connecting' as TransportStatus,
            lastEventId: 0,
            pollTimer: null,
          }),
          ...patch,
        },
      },
    }));
  },

  _setItems(key, items) {
    set((state) => ({
      sessions: {
        ...state.sessions,
        [key]: {
          ...(state.sessions[key] ?? {
            items: [],
            status: 'connecting' as TransportStatus,
            lastEventId: 0,
            pollTimer: null,
          }),
          items,
        },
      },
    }));
  },
}));

// ── SSE EventSource registry ──────────────────────────────────────────────────
//
// Lives outside React to survive StrictMode double-mount.
// Keyed by sessionId.

interface SseEntry {
  es: EventSource;
}

const _sseRegistry: Map<string, SseEntry> = new Map();

function buildSseUrl(sessionId: string, afterId: number): string {
  const base = typeof window !== 'undefined' ? '' : 'http://localhost:8008';
  const path = `/api/ai/sessions/${encodeURIComponent(sessionId)}/stream`;
  return `${base}${path}?after_id=${afterId}`;
}

// ── SSE + polling helpers ──────────────────────────────────────────────────────

function openSse(sessionId: string): void {
  if (_sseRegistry.has(sessionId)) return; // already open — idempotent guard

  const store = useRestStore.getState();
  const afterId = store.sessions[sessionId]?.lastEventId ?? 0;
  const url = buildSseUrl(sessionId, afterId);

  let es: EventSource;
  try {
    es = new EventSource(url, { withCredentials: true });
  } catch {
    useRestStore.getState()._setSlice(sessionId, { status: 'error' });
    startPolling(sessionId);
    return;
  }

  _sseRegistry.set(sessionId, { es });

  es.onopen = () => {
    useRestStore.getState()._setSlice(sessionId, { status: 'open' });
  };

  es.addEventListener('ai_event', (ev: MessageEvent) => {
    let parsed: AiEvent;
    try {
      parsed = JSON.parse(ev.data as string) as AiEvent;
    } catch {
      return;
    }
    const current = useRestStore.getState().sessions[sessionId];
    if (!current) return;
    const nextItems = foldEvent(current.items, parsed);
    const newLastId = Math.max(current.lastEventId, parsed.id);
    useRestStore.getState()._setSlice(sessionId, {
      items: nextItems,
      lastEventId: newLastId,
    });
  });

  es.onerror = () => {
    useRestStore.getState()._setSlice(sessionId, { status: 'reconnecting' });
    // If EventSource closes (CLOSED state), fall back to polling after a brief delay
    setTimeout(() => {
      const entry = _sseRegistry.get(sessionId);
      if (entry && entry.es.readyState === EventSource.CLOSED) {
        _sseRegistry.delete(sessionId);
        startPolling(sessionId);
      }
    }, 5000);
  };
}

function closeSse(sessionId: string): void {
  const entry = _sseRegistry.get(sessionId);
  if (entry) {
    entry.es.close();
    _sseRegistry.delete(sessionId);
  }
}

function startPolling(sessionId: string): void {
  const store = useRestStore.getState();
  const existing = store.sessions[sessionId];
  if (existing?.pollTimer) return; // already polling

  const timer = setInterval(async () => {
    try {
      const session = await unwrap(
        apiClient.GET('/api/ai/sessions/{id}', {
          params: { path: { id: sessionId } },
        }),
      );
      const items = sessionToItems(session);
      const maxEventId = (session.events ?? []).reduce((max, e) => Math.max(max, e.id), 0);
      const current = useRestStore.getState().sessions[sessionId];
      useRestStore.getState()._setSlice(sessionId, {
        items,
        lastEventId: Math.max(current?.lastEventId ?? 0, maxEventId),
        status: 'open',
      });
    } catch {
      useRestStore.getState()._setSlice(sessionId, { status: 'reconnecting' });
    }
  }, 3000);

  store._setSlice(sessionId, { pollTimer: timer });
}

function stopPolling(sessionId: string): void {
  const existing = useRestStore.getState().sessions[sessionId];
  if (existing?.pollTimer) {
    clearInterval(existing.pollTimer);
    useRestStore.getState()._setSlice(sessionId, { pollTimer: null });
  }
}

function disposeSession(sessionId: string): void {
  closeSse(sessionId);
  stopPolling(sessionId);
  useRestStore.getState()._setSlice(sessionId, { status: 'closed' });
}

// ── Backfill + connect entry point ────────────────────────────────────────────

async function backfillAndConnect(sessionId: string): Promise<void> {
  useRestStore.getState()._setSlice(sessionId, {
    status: 'connecting',
    items: [],
    lastEventId: 0,
  });

  let session: AiSession;
  try {
    session = await unwrap(
      apiClient.GET('/api/ai/sessions/{id}', {
        params: { path: { id: sessionId } },
      }),
    );
  } catch {
    useRestStore.getState()._setSlice(sessionId, { status: 'error' });
    return;
  }

  const items = sessionToItems(session);
  const maxEventId = (session.events ?? []).reduce((max, e) => Math.max(max, e.id), 0);
  useRestStore.getState()._setSlice(sessionId, {
    items,
    lastEventId: maxEventId,
    status: 'connecting',
  });

  openSse(sessionId);
}

// ── Public hook ───────────────────────────────────────────────────────────────

export interface RestConversationHandle {
  items: ConversationItem[];
  status: TransportStatus;
  send(text: string): Promise<void>;
  approve(actionId: string): Promise<void>;
  reject(actionId: string, note?: string): Promise<void>;
}

/**
 * useRestConversation — connects to the AI session REST endpoints for a given
 * sessionId and returns the live conversation state.
 *
 * StrictMode safety: the SSE registry prevents a second EventSource from opening
 * on the second mount. On unmount the registry entry is removed. The second mount
 * creates a fresh connection — same pattern as _attachRegistry in useAcpConversation.
 *
 * When sessionId changes: dispose old connection, start fresh.
 */
export function useRestConversation(sessionId: string): RestConversationHandle {
  const slice = useRestStore((state) => state.sessions[sessionId]);
  const items = slice?.items ?? [];
  const status = slice?.status ?? 'connecting';

  // Keep a stable ref to the current sessionId for mutation closures
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;

  useEffect(() => {
    const id = sessionId;
    backfillAndConnect(id);

    return () => {
      disposeSession(id);
    };
  }, [sessionId]);

  const send = async (text: string): Promise<void> => {
    const id = sessionIdRef.current;
    const session = await unwrap(
      apiClient.POST('/api/ai/sessions/{id}/turns', {
        params: { path: { id } },
        body: { message: text },
      }),
    );
    useRestStore.getState()._setItems(id, sessionToItems(session));
  };

  const approve = async (actionId: string): Promise<void> => {
    const id = sessionIdRef.current;
    const session = await unwrap(
      apiClient.POST('/api/ai/sessions/{id}/actions/{action_id}/approve', {
        params: { path: { id, action_id: actionId } },
      }),
    );
    useRestStore.getState()._setItems(id, sessionToItems(session));
  };

  const reject = async (actionId: string, note = ''): Promise<void> => {
    const id = sessionIdRef.current;
    const session = await unwrap(
      apiClient.POST('/api/ai/sessions/{id}/actions/{action_id}/reject', {
        params: { path: { id, action_id: actionId }, query: { note } },
      }),
    );
    useRestStore.getState()._setItems(id, sessionToItems(session));
  };

  return { items, status, send, approve, reject };
}

// ── Re-export schema types for tests ──────────────────────────────────────────

export type { AiSession, AiEvent, AiTurn, AiPendingAction };
