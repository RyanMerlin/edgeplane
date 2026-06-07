/**
 * Fleet dashboard — landing page (/).
 *
 * Consolidates the former Overview (per-agent conversation tabs) and Agents
 * (table) routes into one surface with two sub-views, toggled in-page:
 *   - Conversations: one tab per registered agent → live ACP <ConversationView>
 *   - Agents:        the full control-plane + mesh merge table
 *
 * Both sub-views share one agent list via useMergedAgents() (see
 * lib/useMergedAgents.ts) — no duplicated fetch/merge. The old /agents route
 * now redirects here; the row-click detail route /agents/$agentId is unchanged.
 *
 * Conversation pane: only the active tab mounts AcpPane, so at most ONE
 * WebSocket is open at a time.
 */

import { ConversationView } from '@/components/conversation/ConversationView';
import { AgentsTable } from '@/components/fleet/AgentsTable';
import { useAcpConversation } from '@/lib/conversation/useAcpConversation';
import { type MergedAgent, useMergedAgents } from '@/lib/useMergedAgents';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useEffect, useState } from 'react';

// ── Route ──────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/')({
  component: FleetDashboard,
});

type FleetView = 'console' | 'table';

// ── Helpers ────────────────────────────────────────────────────────────────────

/** Resolve the nodeId from the agent's metadata JSON string. */
function resolveNodeId(metadata: string | null | undefined): string | null {
  if (!metadata) return null;
  try {
    const parsed = JSON.parse(metadata);
    if (
      typeof parsed === 'object' &&
      parsed !== null &&
      typeof parsed.node_id === 'string' &&
      parsed.node_id.length > 0
    ) {
      return parsed.node_id;
    }
  } catch {
    // malformed JSON
  }
  return null;
}

function statusColor(status: string): string {
  switch (status) {
    case 'online':
    case 'active':
      return 'var(--ok)';
    case 'working':
    case 'busy':
      return 'var(--warn)';
    case 'error':
      return 'var(--err)';
    default:
      return 'var(--muted)';
  }
}

function fmtRelative(s: string | null | undefined): string {
  if (!s) return '—';
  const diffMs = Date.now() - new Date(s).getTime();
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 60) return `${diffSec}s ago`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  return new Date(s).toLocaleString();
}

// ── Conversation pane ────────────────────────────────────────────────────────

/**
 * AcpPane — mounts useAcpConversation only when rendered.
 * Kept in a separate component so the hook lifecycle (and WebSocket) is tied
 * to mounting. The parent conditionally renders this only for the active tab,
 * ensuring at most ONE ACP WebSocket is open at a time.
 */
function AcpPane({ nodeId, agentId }: { nodeId: string; agentId: string }) {
  const { items, status, send, cancel } = useAcpConversation(nodeId, agentId);
  return <ConversationView items={items} status={status} onSend={send} onCancel={cancel} />;
}

function AgentPane({ agent, isActive }: { agent: MergedAgent; isActive: boolean }) {
  const nodeId = resolveNodeId(agent.metadata);

  // Derive last-seen from whichever timestamp is available
  const lastSeen = agent.last_heartbeat_at ?? agent.updated_at;

  return (
    <div
      style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}
      data-testid={`agent-pane-${agent.public_id}`}
    >
      {/* Agent status header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '8px',
          padding: '6px 10px',
          background: 'var(--surface)',
          borderBottom: '1px solid var(--border)',
          flexShrink: 0,
          fontSize: '11px',
          color: 'var(--muted)',
        }}
        data-testid={`agent-status-${agent.public_id}`}
      >
        <span
          aria-label={`status: ${agent.status}`}
          style={{
            width: '7px',
            height: '7px',
            borderRadius: '50%',
            background: statusColor(agent.status),
            flexShrink: 0,
            display: 'inline-block',
          }}
        />
        <span style={{ fontFamily: 'monospace', color: 'var(--text)', fontWeight: 600 }}>
          {agent.name}
        </span>
        <span
          className={`tag ${agent.status === 'online' || agent.status === 'active' ? 'ok' : ''}`}
          data-testid={`agent-status-badge-${agent.public_id}`}
        >
          {agent.status}
        </span>
        {nodeId && (
          <span
            style={{ color: 'var(--dim)', fontSize: '10px' }}
            data-testid={`agent-node-${agent.public_id}`}
          >
            {nodeId}
          </span>
        )}
        <span style={{ marginLeft: 'auto', fontSize: '10px' }}>{fmtRelative(lastSeen)}</span>
      </div>

      {/* Conversation pane or fallback */}
      <div style={{ flex: 1, minHeight: 0 }}>
        {!nodeId && (
          <div
            style={{
              padding: '24px 16px',
              color: 'var(--muted)',
              fontSize: '13px',
              textAlign: 'center',
            }}
            data-testid={`agent-not-attachable-${agent.public_id}`}
          >
            <div style={{ fontWeight: 600, marginBottom: '4px' }}>Not attachable</div>
            <div>
              Agent <code>{agent.public_id}</code> has no <code>node_id</code> in its metadata.
            </div>
          </div>
        )}
        {nodeId && isActive && <AcpPane nodeId={nodeId} agentId={agent.public_id} />}
      </div>
    </div>
  );
}

/**
 * FleetConsole — per-agent conversation tabs + active pane.
 * Owns the active-tab selection and Ctrl/Meta+N hotkeys (scoped to this view).
 */
function FleetConsole({ agents }: { agents: MergedAgent[] }) {
  const [activeProfile, setActiveProfile] = useState<string | null>(null);

  // Derive active agent id: explicit selection or first agent as default
  const activeAgentId = activeProfile ?? (agents.length ? agents[0].public_id : null);

  // Keyboard hotkeys: Ctrl/Meta+N switches to the Nth agent tab (1-indexed)
  useEffect(() => {
    function handleKeydown(e: KeyboardEvent) {
      if (!(e.ctrlKey || e.metaKey)) return;
      const num = Number.parseInt(e.key, 10);
      if (num >= 1 && num <= agents.length) {
        e.preventDefault();
        setActiveProfile(agents[num - 1].public_id);
      }
    }
    document.addEventListener('keydown', handleKeydown);
    return () => document.removeEventListener('keydown', handleKeydown);
  }, [agents]);

  const activeAgent = agents.find((a) => a.public_id === activeAgentId) ?? null;

  if (agents.length === 0) {
    return (
      <div className="empty-state" data-testid="empty-state">
        <div className="empty-icon">⊙</div>
        <div className="empty-title">No agents registered</div>
        <div className="empty-body">
          No agents have been registered yet. Start an agent with{' '}
          <code>edgeplane agent register</code>.
        </div>
      </div>
    );
  }

  return (
    <>
      {/* Agent tabs — one per registered agent, driven by the merged list */}
      <div
        style={{
          display: 'flex',
          gap: 0,
          padding: '0 0.25rem',
          borderBottom: '1px solid var(--border)',
          flexShrink: 0,
        }}
        role="tablist"
        aria-label="Fleet agents"
        data-testid="profile-tabs"
      >
        {agents.map((a, i) => {
          const isActive = a.public_id === activeAgentId;
          return (
            <button
              key={a.public_id}
              type="button"
              role="tab"
              aria-selected={isActive}
              aria-controls={`panel-${a.public_id}`}
              id={`tab-${a.public_id}`}
              onClick={() => setActiveProfile(a.public_id)}
              title={`${a.name} (${a.status}) — Ctrl+${i + 1}`}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '0.4rem',
                padding: '0.5rem 0.75rem',
                background: 'none',
                border: 'none',
                borderBottom: isActive ? '2px solid var(--accent)' : '2px solid transparent',
                color: isActive ? 'var(--text)' : 'var(--muted)',
                fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
                fontSize: '13px',
                cursor: 'pointer',
              }}
              data-testid={`tab-${a.public_id}`}
            >
              <span
                aria-hidden="true"
                style={{
                  width: '6px',
                  height: '6px',
                  borderRadius: '50%',
                  background: statusColor(a.status),
                  flexShrink: 0,
                  display: 'inline-block',
                }}
              />
              {a.name}
            </button>
          );
        })}
      </div>

      {/* Active agent pane */}
      {activeAgent && (
        <div
          role="tabpanel"
          id={`panel-${activeAgent.public_id}`}
          aria-labelledby={`tab-${activeAgent.public_id}`}
          style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}
          data-testid="active-profile-panel"
        >
          <AgentPane key={activeAgent.public_id} agent={activeAgent} isActive />
        </div>
      )}
    </>
  );
}

// ── View toggle ────────────────────────────────────────────────────────────────

function ViewToggle({ view, onChange }: { view: FleetView; onChange: (v: FleetView) => void }) {
  const tabs: { id: FleetView; label: string }[] = [
    { id: 'console', label: 'Conversations' },
    { id: 'table', label: 'Agents' },
  ];
  return (
    <div
      style={{ display: 'flex', gap: '2px', marginLeft: 'auto' }}
      role="tablist"
      aria-label="Fleet view"
      data-testid="fleet-view-toggle"
    >
      {tabs.map((t) => {
        const active = view === t.id;
        return (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(t.id)}
            data-testid={`fleet-view-${t.id}`}
            style={{
              padding: '2px 10px',
              fontSize: '11px',
              borderRadius: '3px',
              border: '1px solid var(--border-2)',
              background: active ? 'var(--accent)' : 'var(--base)',
              color: active ? 'var(--base)' : 'var(--muted)',
              cursor: 'pointer',
            }}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

// ── Main component ─────────────────────────────────────────────────────────────

export function FleetDashboard() {
  const { agents, isLoading, isError, error } = useMergedAgents();
  const [view, setView] = useState<FleetView>('table');
  const navigate = useNavigate();

  // Fleet summary counts — derived from merged agents, no extra endpoint needed
  const onlineCount = agents.filter((a) => a.status === 'online' || a.status === 'active').length;
  const totalCount = agents.length;

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}
      data-testid="fleet-dashboard"
    >
      {/* Loading */}
      {isLoading && (
        <div style={{ padding: '12px' }}>
          <p className="muted" data-testid="loading-state">
            Loading fleet…
          </p>
        </div>
      )}

      {/* Error */}
      {isError && (
        <div style={{ padding: '12px' }}>
          <p className="error" data-testid="error-state">
            Failed to load fleet — {error?.message ?? 'unknown error'}
          </p>
        </div>
      )}

      {/* Body — show once at least one query resolves */}
      {!isLoading && !isError && (
        <>
          {/* Fleet header: summary + view toggle */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '12px',
              padding: '6px 10px',
              background: 'var(--surface)',
              borderBottom: '1px solid var(--border)',
              flexShrink: 0,
              fontSize: '11px',
              color: 'var(--muted)',
            }}
            data-testid="fleet-summary"
          >
            <span style={{ fontWeight: 600, color: 'var(--text)', fontSize: '12px' }}>Fleet</span>
            {totalCount > 0 && (
              <span data-testid="fleet-online-count">
                <span style={{ color: 'var(--ok)', fontWeight: 600 }}>{onlineCount}</span>
                {' / '}
                {totalCount} online
              </span>
            )}
            <ViewToggle view={view} onChange={setView} />
          </div>

          {/* Active sub-view */}
          {view === 'console' ? (
            <FleetConsole agents={agents} />
          ) : (
            <AgentsTable
              agents={agents}
              isLoading={false}
              isError={false}
              error={null}
              onRowClick={(a) =>
                navigate({ to: '/agents/$agentId', params: { agentId: a.public_id } })
              }
            />
          )}
        </>
      )}
    </div>
  );
}
