/**
 * ACP conversation hook — Zustand-backed, StrictMode-safe.
 *
 * Design mirrors eventStream.ts:
 *   - ONE WebSocket per (nodeId, agentId) key, shared via a Zustand slice map.
 *   - connect() is idempotent: AcpAttach.open() guards the double-mount.
 *   - Coalescing logic ported from web/src/lib/components/AgentConversation.svelte:
 *       agent_message_chunk  → append to last assistant message (or create new)
 *       agent_thought_chunk  → append to last thinking item (or create new)
 *       tool_call            → push a new tool_call item (status: 'pending')
 *       tool_call_update     → find matching toolCallId, update status in place;
 *                              append a new item if the toolCallId is not found
 *       plan                 → ignored (no normalized item for plan updates)
 *       METADATA_KINDS       → suppressed entirely (no items produced)
 *       everything else      → also suppressed (forward-compat; unknown variants
 *                              don't become noise in the item list)
 *
 * Suppressed metadata variants (matching AgentConversation.svelte):
 *   user_message_chunk, available_commands_update, current_mode_update,
 *   config_option_update, session_info_update, usage_update
 *
 * NOTE: The `approval` ConversationItem kind is emitted only by the REST
 * transport (future pass). The ACP transport never produces it.
 */

import { AcpAttach } from '@/api/acp-attach';
import type { SessionNotification, SessionUpdate } from '@/api/acp-attach';
import { useEffect, useRef } from 'react';
import { create } from 'zustand';
import type { ConversationItem, TransportStatus } from './types';

// ── Metadata variants to suppress ─────────────────────────────────────────────

const METADATA_KINDS = new Set([
  'user_message_chunk',
  'available_commands_update',
  'current_mode_update',
  'config_option_update',
  'session_info_update',
  'usage_update',
]);

// ── ID generation ──────────────────────────────────────────────────────────────

let _nextId = 0;
function nextId(): string {
  return `acp-${_nextId++}`;
}

// ── Per-connection state slice ─────────────────────────────────────────────────

export interface AcpSlice {
  items: ConversationItem[];
  status: TransportStatus;
}

// Key: `${nodeId}/${agentId}`
type AcpStoreState = {
  connections: Record<string, AcpSlice>;
  _setSlice(key: string, patch: Partial<AcpSlice>): void;
  _appendItem(key: string, item: ConversationItem): void;
  _updateLastMessage(key: string, text: string): void;
  _updateLastThinking(key: string, text: string): void;
  _updateToolCall(key: string, toolCallId: string, newStatus: string): void;
};

// ── Zustand store ─────────────────────────────────────────────────────────────

export const useAcpStore = create<AcpStoreState>((set, _get) => ({
  connections: {},

  _setSlice(key, patch) {
    set((state) => ({
      connections: {
        ...state.connections,
        [key]: { ...(state.connections[key] ?? { items: [], status: 'connecting' }), ...patch },
      },
    }));
  },

  _appendItem(key, item) {
    set((state) => {
      const existing = state.connections[key] ?? { items: [], status: 'connecting' };
      return {
        connections: {
          ...state.connections,
          [key]: { ...existing, items: [...existing.items, item] },
        },
      };
    });
  },

  _updateLastMessage(key, appendText) {
    set((state) => {
      const existing = state.connections[key];
      if (!existing) return state;
      const items = [...existing.items];
      // Find the last assistant message and append text to it
      for (let i = items.length - 1; i >= 0; i--) {
        const item = items[i];
        if (item && item.kind === 'message' && item.role === 'assistant') {
          items[i] = { ...item, text: item.text + appendText, streaming: true };
          return {
            connections: {
              ...state.connections,
              [key]: { ...existing, items },
            },
          };
        }
      }
      // No existing assistant message — fall back to append (caller should have handled this)
      return state;
    });
  },

  _updateLastThinking(key, appendText) {
    set((state) => {
      const existing = state.connections[key];
      if (!existing) return state;
      const items = [...existing.items];
      for (let i = items.length - 1; i >= 0; i--) {
        const item = items[i];
        if (item && item.kind === 'thinking') {
          items[i] = { ...item, text: item.text + appendText, streaming: true };
          return {
            connections: {
              ...state.connections,
              [key]: { ...existing, items },
            },
          };
        }
      }
      return state;
    });
  },

  _updateToolCall(key, toolCallId, newStatus) {
    set((state) => {
      const existing = state.connections[key];
      if (!existing) return state;
      const items = [...existing.items];
      let found = false;
      for (let i = items.length - 1; i >= 0; i--) {
        const item = items[i];
        if (item && item.kind === 'tool_call' && item.toolCallId === toolCallId) {
          const validStatus = newStatus as Extract<
            ConversationItem,
            { kind: 'tool_call' }
          >['status'];
          items[i] = { ...item, status: validStatus };
          found = true;
          break;
        }
      }
      if (!found) return state;
      return {
        connections: {
          ...state.connections,
          [key]: { ...existing, items },
        },
      };
    });
  },
}));

// ── Frame ingestion (coalescing logic) ────────────────────────────────────────

function textOf(content: unknown): string {
  if (!content || typeof content !== 'object') return '';
  const c = content as Record<string, unknown>;
  if (typeof c.text === 'string') return c.text;
  return '';
}

function ingest(key: string, notif: SessionNotification): void {
  const store = useAcpStore.getState();
  const u = notif.update as SessionUpdate;
  const existing = store.connections[key];

  switch (u.sessionUpdate) {
    case 'agent_message_chunk': {
      const text = textOf(u.content);
      if (!text) break;
      // Check if the last item is an assistant message we can coalesce into
      const items = existing?.items ?? [];
      const last = items[items.length - 1];
      if (last && last.kind === 'message' && last.role === 'assistant') {
        store._updateLastMessage(key, text);
      } else {
        store._appendItem(key, {
          kind: 'message',
          id: nextId(),
          role: 'assistant',
          text,
          streaming: true,
        });
      }
      break;
    }

    case 'agent_thought_chunk': {
      const text = textOf(u.content);
      if (!text) break;
      const items = existing?.items ?? [];
      const last = items[items.length - 1];
      if (last && last.kind === 'thinking') {
        store._updateLastThinking(key, text);
      } else {
        store._appendItem(key, {
          kind: 'thinking',
          id: nextId(),
          text,
          streaming: true,
        });
      }
      break;
    }

    case 'tool_call': {
      const tc = u as Record<string, unknown>;
      store._appendItem(key, {
        kind: 'tool_call',
        id: nextId(),
        toolCallId: (tc.toolCallId as string | undefined) ?? '',
        title: (tc.title as string | undefined) ?? 'tool',
        status: 'pending',
        rawInput: tc.rawInput,
      });
      break;
    }

    case 'tool_call_update': {
      const tcu = u as Record<string, unknown>;
      const toolCallId = (tcu.toolCallId as string | undefined) ?? '';
      const newStatus = (tcu.status as string | undefined) ?? '';
      if (!newStatus) break;

      // Validate the status is a known value; fall back to 'pending' for unknown
      const validStatuses = new Set(['pending', 'in_progress', 'completed', 'failed', 'cancelled']);
      const safeStatus = validStatuses.has(newStatus) ? newStatus : 'pending';

      // Try to update existing tool_call item; if not found, append a new one
      const items = existing?.items ?? [];
      const found = items.some(
        (item) => item.kind === 'tool_call' && item.toolCallId === toolCallId,
      );
      if (found) {
        store._updateToolCall(key, toolCallId, safeStatus);
      } else {
        // toolCallId not seen yet — append a new item
        store._appendItem(key, {
          kind: 'tool_call',
          id: nextId(),
          toolCallId,
          title: (tcu.title as string | undefined) ?? 'tool',
          status: safeStatus as 'pending' | 'in_progress' | 'completed' | 'failed' | 'cancelled',
        });
      }
      break;
    }

    case 'plan':
      // Plan updates have no normalized item kind — suppressed.
      break;

    default:
      // Metadata variants and unknown variants: suppress.
      // METADATA_KINDS is a strict superset of the suppression list from AgentConversation.svelte.
      if (METADATA_KINDS.has(u.sessionUpdate)) break;
      // Truly unknown variants: also suppressed for noise reasons.
      // The Svelte viewer shows them as 'unknown' entries, but the normalized model
      // has no unknown item kind. Unknown variants are silently dropped.
      break;
  }
}

// ── AcpAttach instance registry ───────────────────────────────────────────────

// Keyed by nodeId/agentId — lives outside React to survive StrictMode double-mount.
const _attachRegistry: Map<string, AcpAttach> = new Map();

function getOrCreateAttach(key: string, nodeId: string, agentId: string): AcpAttach {
  const existing = _attachRegistry.get(key);
  if (existing) return existing;

  const store = useAcpStore.getState();
  store._setSlice(key, { items: [], status: 'connecting' });

  const attach = new AcpAttach(nodeId, agentId, {
    onStatus: (status) => {
      // Map AcpConnectionStatus → TransportStatus
      const mapped: TransportStatus =
        status === 'open'
          ? 'open'
          : status === 'connecting'
            ? 'connecting'
            : status === 'reconnecting'
              ? 'reconnecting'
              : status === 'error'
                ? 'error'
                : 'closed';
      useAcpStore.getState()._setSlice(key, { status: mapped });
    },
    onNotification: (notif) => ingest(key, notif),
  });

  _attachRegistry.set(key, attach);
  return attach;
}

/** Remove an attach instance from the registry and dispose it. */
function removeAttach(key: string): void {
  const attach = _attachRegistry.get(key);
  if (attach) {
    attach.dispose();
    _attachRegistry.delete(key);
  }
}

// ── Public hook ───────────────────────────────────────────────────────────────

export interface AcpConversationHandle {
  items: ConversationItem[];
  status: TransportStatus;
  send(text: string): void;
  cancel(): void;
}

/**
 * useAcpConversation — connects to the ACP attach endpoint for a given
 * (nodeId, agentId) pair and returns the live conversation state.
 *
 * StrictMode safety: AcpAttach.open() is idempotent (the _opened guard
 * in AcpAttach prevents a second WebSocket from opening on the second
 * mount). The registry ensures a single AcpAttach instance per key.
 *
 * Cleanup on unmount: the attach instance is disposed and removed from
 * the registry so the next mount gets a fresh connection.
 */
export function useAcpConversation(nodeId: string, agentId: string): AcpConversationHandle {
  const key = `${nodeId}/${agentId}`;
  // useRef keeps a stable reference to the attach so we can dispose on unmount
  const attachRef = useRef<AcpAttach | null>(null);

  const slice = useAcpStore((state) => state.connections[key]);
  const items = slice?.items ?? [];
  const status = slice?.status ?? 'connecting';

  useEffect(() => {
    const attach = getOrCreateAttach(key, nodeId, agentId);
    attachRef.current = attach;
    attach.open();

    return () => {
      // StrictMode: React calls cleanup then re-runs effect. AcpAttach.open()
      // is idempotent so the second open() after cleanup is a no-op IF we
      // don't dispose on cleanup. However, to cleanly unmount we DO dispose,
      // which means the second mount creates a fresh AcpAttach. This is the
      // correct behavior — a fresh connection on remount.
      removeAttach(key);
      attachRef.current = null;
    };
    // key is derived from nodeId + agentId; all three are listed so the
    // exhaustive-deps rule is satisfied. A change to nodeId/agentId changes
    // key, triggering cleanup + fresh connection — correct behavior.
  }, [key, nodeId, agentId]);

  const send = (text: string) => {
    attachRef.current?.sendPrompt(text);
  };

  const cancel = () => {
    attachRef.current?.sendCancel();
  };

  return { items, status, send, cancel };
}
