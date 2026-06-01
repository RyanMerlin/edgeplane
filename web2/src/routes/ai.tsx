/**
 * AI console route — Phase 5.
 *
 * Sessions sidebar + conversation pane, driven by useRestConversation.
 *
 * Behaviour parity with web/src/routes/ai/+page.svelte:
 *   - Lists sessions via GET /api/ai/sessions (polled at 30s cadence via React Query)
 *   - Auto-selects the first session on load; auto-creates one if the list is empty
 *   - New Session button: POST /api/ai/sessions with a runtime picker (CapabilitySet)
 *   - Selected session: <ConversationView> driven by useRestConversation
 *   - approval items: wired to approve/reject mutations
 *   - Backend unavailable (sessions query error): shows the unavailable state
 *
 * Deviations from the Svelte page (deliberate):
 *   - Uses SSE stream instead of 2.5s polling for the active session's live updates
 *   - Sessions list is a sidebar column rather than a top bar (better scalability)
 *   - No "show debug events" toggle (debug tool; out of scope for Phase 5)
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { ConversationView } from '@/components/conversation/ConversationView';
import { useRestConversation } from '@/lib/conversation/useRestConversation';
import { queryKeys } from '@/lib/queryKeys';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';

// ── Types ──────────────────────────────────────────────────────────────────────

type AiSession = components['schemas']['AiSession'];
type CapabilitySet = components['schemas']['CapabilitySet'];

// ── Route ──────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/ai')({
  component: AiConsolePage,
});

// ── Query functions ────────────────────────────────────────────────────────────

async function fetchSessions(): Promise<AiSession[]> {
  return unwrap(
    apiClient.GET('/api/ai/sessions', {
      params: { query: { limit: 20 } },
    }),
  );
}

async function fetchCapabilities(): Promise<CapabilitySet[]> {
  return unwrap(apiClient.GET('/api/ai/runtime-capabilities'));
}

// ── Sub-components ─────────────────────────────────────────────────────────────

function SessionRow({
  session,
  active,
  onClick,
}: {
  session: AiSession;
  active: boolean;
  onClick: () => void;
}) {
  const label = session.title || session.id;
  const statusColor =
    session.status === 'active'
      ? 'var(--ok)'
      : session.status === 'error'
        ? 'var(--err)'
        : 'var(--muted)';

  return (
    <button
      type="button"
      data-testid={`session-row-${session.id}`}
      onClick={onClick}
      style={{
        display: 'block',
        width: '100%',
        textAlign: 'left',
        padding: '7px 10px',
        background: active ? 'var(--surface-2, var(--surface))' : 'transparent',
        border: 'none',
        borderBottom: '1px solid var(--border)',
        borderLeft: active ? '3px solid var(--accent)' : '3px solid transparent',
        cursor: 'pointer',
        fontSize: '12px',
        color: 'var(--text)',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '5px',
          marginBottom: '2px',
        }}
      >
        <span
          style={{
            width: '7px',
            height: '7px',
            borderRadius: '50%',
            background: statusColor,
            flexShrink: 0,
          }}
          aria-hidden
        />
        <span
          style={{
            flex: 1,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {label}
        </span>
      </div>
      {session.runtime_kind && (
        <div style={{ fontSize: '10px', color: 'var(--dim)', paddingLeft: '12px' }}>
          {session.runtime_kind}
        </div>
      )}
    </button>
  );
}

// ── New session modal ──────────────────────────────────────────────────────────

interface NewSessionModalProps {
  capabilities: CapabilitySet[];
  onClose: () => void;
  onCreate: (runtimeKind: string, title: string) => void;
  busy: boolean;
}

function NewSessionModal({ capabilities, onClose, onCreate, busy }: NewSessionModalProps) {
  const [runtimeKind, setRuntimeKind] = useState(capabilities[0]?.runtime_kind ?? 'opencode');
  const [title, setTitle] = useState('');

  return (
    <dialog
      aria-label="New AI session"
      open
      style={{
        position: 'fixed',
        inset: 0,
        margin: 'auto',
        background: 'rgba(0,0,0,0.5)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 100,
        border: 'none',
        padding: 0,
        width: '100vw',
        height: '100vh',
        maxWidth: 'none',
        maxHeight: 'none',
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') onClose();
      }}
    >
      <div
        data-testid="new-session-modal"
        style={{
          background: 'var(--base)',
          border: '1px solid var(--border)',
          borderRadius: '6px',
          padding: '20px',
          width: '320px',
          display: 'flex',
          flexDirection: 'column',
          gap: '12px',
        }}
      >
        <div
          style={{ fontSize: '13px', fontWeight: 600, color: 'var(--text)', marginBottom: '4px' }}
        >
          New AI Session
        </div>

        <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
          <label htmlFor="new-session-title" style={{ fontSize: '11px', color: 'var(--muted)' }}>
            Title (optional)
          </label>
          <input
            id="new-session-title"
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Session title…"
            data-testid="new-session-title"
            style={{
              fontSize: '12px',
              padding: '4px 8px',
              background: 'var(--surface)',
              color: 'var(--text)',
              border: '1px solid var(--border)',
              borderRadius: '3px',
            }}
          />
        </div>

        {capabilities.length > 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
            <label
              htmlFor="new-session-runtime"
              style={{ fontSize: '11px', color: 'var(--muted)' }}
            >
              Runtime
            </label>
            <select
              id="new-session-runtime"
              value={runtimeKind}
              onChange={(e) => setRuntimeKind(e.target.value)}
              data-testid="new-session-runtime"
              style={{
                fontSize: '12px',
                padding: '4px 8px',
                background: 'var(--surface)',
                color: 'var(--text)',
                border: '1px solid var(--border)',
                borderRadius: '3px',
              }}
            >
              {capabilities.map((cap) => (
                <option key={cap.runtime_kind} value={cap.runtime_kind}>
                  {cap.display_name}
                </option>
              ))}
            </select>
          </div>
        )}

        <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end' }}>
          <button type="button" className="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            onClick={() => onCreate(runtimeKind, title)}
            disabled={busy}
            data-testid="new-session-create"
          >
            {busy ? 'Creating…' : 'Create'}
          </button>
        </div>
      </div>
    </dialog>
  );
}

// ── Active session pane ────────────────────────────────────────────────────────

function ActiveSession({ sessionId }: { sessionId: string }) {
  const { items, status, send, approve, reject } = useRestConversation(sessionId);
  const [approvalBusy, setApprovalBusy] = useState(false);

  const handleApprove = async (actionId: string) => {
    setApprovalBusy(true);
    try {
      await approve(actionId);
    } finally {
      setApprovalBusy(false);
    }
  };

  const handleReject = async (actionId: string, note: string) => {
    setApprovalBusy(true);
    try {
      await reject(actionId, note);
    } finally {
      setApprovalBusy(false);
    }
  };

  return (
    <ConversationView
      items={items}
      status={status}
      onSend={send}
      onCancel={() => {
        // REST sessions don't have an explicit cancel; no-op for now
      }}
      onApprove={handleApprove}
      onReject={handleReject}
      approvalBusy={approvalBusy}
    />
  );
}

// ── Unavailable state ──────────────────────────────────────────────────────────

function UnavailableState() {
  return (
    <div
      data-testid="ai-unavailable"
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: '8px',
        padding: '24px',
        textAlign: 'center',
        color: 'var(--muted)',
      }}
    >
      <div style={{ fontSize: '32px', color: 'var(--dim)', marginBottom: '4px' }}>&#x2298;</div>
      <div style={{ fontSize: '14px', fontWeight: 600, color: 'var(--text)' }}>
        AI Console not available
      </div>
      <div style={{ fontSize: '12px' }}>
        No AI backend is configured for this EdgePlane instance.
      </div>
      <div style={{ fontSize: '11px', color: 'var(--dim)', maxWidth: '320px' }}>
        Configure an AI provider in the server settings to enable the console.
      </div>
    </div>
  );
}

// ── Empty session pane ─────────────────────────────────────────────────────────

function EmptySessionPane() {
  return (
    <div
      data-testid="no-session-selected"
      style={{
        flex: 1,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        color: 'var(--muted)',
        fontSize: '13px',
      }}
    >
      Select a session or create a new one.
    </div>
  );
}

// ── Main page ──────────────────────────────────────────────────────────────────

export function AiConsolePage() {
  const queryClient = useQueryClient();
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [showNewModal, setShowNewModal] = useState(false);
  const [autoCreated, setAutoCreated] = useState(false);

  // Sessions list — 30s refresh (matches Svelte page cadence)
  const sessionsQuery = useQuery({
    queryKey: queryKeys.ai.sessions(),
    queryFn: fetchSessions,
    refetchInterval: 30_000,
  });

  // Runtime capabilities for the new session picker
  const capabilitiesQuery = useQuery({
    queryKey: [...queryKeys.ai.all, 'capabilities'] as const,
    queryFn: fetchCapabilities,
    staleTime: 5 * 60 * 1000,
  });

  // Create session mutation
  const createMutation = useMutation({
    mutationFn: async ({
      runtimeKind,
      title,
    }: {
      runtimeKind: string;
      title: string;
    }) =>
      unwrap(
        apiClient.POST('/api/ai/sessions', {
          body: { runtime_kind: runtimeKind, title: title || null },
        }),
      ),
    onSuccess: (session: AiSession) => {
      setActiveSessionId(session.id);
      queryClient.invalidateQueries({ queryKey: queryKeys.ai.sessions() });
      setShowNewModal(false);
    },
  });

  // Auto-select first session / auto-create when list is empty
  const sessions = sessionsQuery.data;
  if (sessions !== undefined && !sessionsQuery.isError) {
    if (sessions.length > 0 && !activeSessionId) {
      setActiveSessionId(sessions[0].id);
    } else if (sessions.length === 0 && !autoCreated && !createMutation.isPending) {
      setAutoCreated(true);
      createMutation.mutate({ runtimeKind: 'opencode', title: '' });
    }
  }

  const backendUnavailable = sessionsQuery.isError;

  const handleCreate = (runtimeKind: string, title: string) => {
    createMutation.mutate({ runtimeKind, title });
  };

  return (
    <div
      data-testid="ai-console-page"
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        overflow: 'hidden',
      }}
    >
      {/* Top bar */}
      <div
        style={{
          height: '36px',
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          gap: '10px',
          padding: '0 12px',
          background: 'var(--surface)',
          borderBottom: '1px solid var(--border)',
        }}
      >
        <span style={{ fontSize: '12px', fontWeight: 600, color: 'var(--text)' }}>Console</span>
        <span className="muted" style={{ fontSize: '11px' }}>
          Reads auto-run · writes require approval
        </span>
        <div style={{ marginLeft: 'auto' }}>
          <button
            type="button"
            className="ghost"
            onClick={() => setShowNewModal(true)}
            disabled={createMutation.isPending}
            data-testid="new-session-btn"
          >
            New Session
          </button>
        </div>
      </div>

      {backendUnavailable ? (
        <UnavailableState />
      ) : (
        <div style={{ display: 'flex', flex: 1, minHeight: 0, overflow: 'hidden' }}>
          {/* Sessions sidebar */}
          <div
            data-testid="sessions-sidebar"
            style={{
              width: '200px',
              flexShrink: 0,
              borderRight: '1px solid var(--border)',
              overflowY: 'auto',
              background: 'var(--surface)',
            }}
          >
            {sessionsQuery.isLoading && (
              <div
                style={{ padding: '10px', fontSize: '11px', color: 'var(--muted)' }}
                data-testid="sessions-loading"
              >
                Loading…
              </div>
            )}
            {!sessionsQuery.isLoading &&
              sessions?.map((s) => (
                <SessionRow
                  key={s.id}
                  session={s}
                  active={s.id === activeSessionId}
                  onClick={() => setActiveSessionId(s.id)}
                />
              ))}
            {!sessionsQuery.isLoading && sessions?.length === 0 && !createMutation.isPending && (
              <div
                style={{ padding: '10px', fontSize: '11px', color: 'var(--muted)' }}
                data-testid="no-sessions"
              >
                No sessions.
              </div>
            )}
          </div>

          {/* Conversation pane */}
          <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}>
            {activeSessionId ? <ActiveSession sessionId={activeSessionId} /> : <EmptySessionPane />}
          </div>
        </div>
      )}

      {/* New session modal */}
      {showNewModal && (
        <NewSessionModal
          capabilities={capabilitiesQuery.data ?? []}
          onClose={() => setShowNewModal(false)}
          onCreate={handleCreate}
          busy={createMutation.isPending}
        />
      )}
    </div>
  );
}
