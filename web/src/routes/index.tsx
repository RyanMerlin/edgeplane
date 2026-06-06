/**
 * Fleet dashboard — Phase 4 landing page.
 *
 * Tabs are driven dynamically from registered agents returned by the tower.
 * No hardcoded fleet profile list — one tab per agent, label = agent.name as-is.
 *
 * React changes vs. Svelte:
 *   - xterm terminal pane replaced with <ConversationView> via useAcpConversation
 *   - The Svelte "Terminal / Conversation" view toggle is dropped — ACP is the only pane
 *   - Only the active tab mounts AcpPane so at most ONE WebSocket is open at a time
 *
 * Agent data: mirrors agents.tsx merge strategy (cp agents + mesh agents, 30s refetch).
 * Fleet summary: online/total derived from the merged set — no extra backend endpoint needed.
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { ConversationView } from '@/components/conversation/ConversationView';
import { useAcpConversation } from '@/lib/conversation/useAcpConversation';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState } from 'react';

// ── Route ──────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/')({
  component: FleetDashboard,
});

// ── Types ─────────────────────────────────────────────────────────────────────

type Agent = components['schemas']['Agent'];
type NodeMeshAgent = components['schemas']['NodeMeshAgent'];
type RuntimeNode = components['schemas']['RuntimeNode'];

// ── Merged agent row (same shape as agents.tsx) ────────────────────────────────

type MergedAgent = {
  public_id: string;
  name: string;
  status: string;
  metadata?: string;
  updated_at?: string;
  last_heartbeat_at?: string | null;
};

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

// ── Query functions ────────────────────────────────────────────────────────────

async function fetchCpAgents(): Promise<Agent[]> {
  return unwrap(apiClient.GET('/api/agents'));
}

async function fetchMeshAgents(): Promise<NodeMeshAgent[]> {
  const nodes = await unwrap(apiClient.GET('/api/runtime/nodes'));
  const allMesh: NodeMeshAgent[] = [];
  await Promise.all(
    nodes.map(async (node: RuntimeNode) => {
      const agents = await unwrap(
        apiClient.GET('/api/runtime/nodes/{node_id}/agents', {
          params: { path: { node_id: node.id } },
        }),
      ).catch(() => [] as NodeMeshAgent[]);
      allMesh.push(...agents);
    }),
  );
  return allMesh;
}

// ── Merge logic (same as agents.tsx) ──────────────────────────────────────────

function mergeAgents(cpAgents: Agent[], meshAgents: NodeMeshAgent[]): MergedAgent[] {
  const byId = new Map<string, MergedAgent>();

  for (const a of cpAgents) {
    byId.set(a.public_id, {
      public_id: a.public_id,
      name: a.name,
      status: a.status,
      metadata: a.metadata,
      updated_at: a.updated_at,
    });
  }

  for (const a of meshAgents) {
    const pid = a.public_id ?? a.agent_public_id ?? a.id;
    const existing = byId.get(pid);
    if (existing) {
      existing.status = a.status;
      existing.last_heartbeat_at = a.last_heartbeat_at;
    } else {
      byId.set(pid, {
        public_id: pid,
        name: pid,
        status: a.status,
        last_heartbeat_at: a.last_heartbeat_at,
      });
    }
  }

  return Array.from(byId.values());
}

// ── Sub-components ─────────────────────────────────────────────────────────────

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

interface AgentPaneProps {
  agent: MergedAgent;
  isActive: boolean;
}

function AgentPane({ agent, isActive }: AgentPaneProps) {
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

// ── Main component ─────────────────────────────────────────────────────────────

export function FleetDashboard() {
  const [activeProfile, setActiveProfile] = useState<string | null>(null);

  const cpQuery = useQuery({
    queryKey: queryKeys.agents.list(),
    queryFn: fetchCpAgents,
    refetchInterval: 30_000,
  });

  const meshQuery = useQuery({
    queryKey: [...queryKeys.agents.all, 'mesh'] as const,
    queryFn: fetchMeshAgents,
    refetchInterval: 30_000,
  });

  const agents: MergedAgent[] =
    cpQuery.data !== undefined || meshQuery.data !== undefined
      ? mergeAgents(cpQuery.data ?? [], meshQuery.data ?? [])
      : [];

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

  const isLoading = cpQuery.isLoading && meshQuery.isLoading;
  const isError = cpQuery.isError && meshQuery.isError;

  // Fleet summary counts — derived from merged agents, no extra endpoint needed
  const onlineCount = agents.filter((a) => a.status === 'online' || a.status === 'active').length;
  const totalCount = agents.length;

  const activeAgent = agents.find((a) => a.public_id === activeAgentId) ?? null;

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
            Failed to load fleet —{' '}
            {(cpQuery.error as Error)?.message ??
              (meshQuery.error as Error)?.message ??
              'unknown error'}
          </p>
        </div>
      )}

      {/* Dashboard body — show once at least one query resolves */}
      {!isLoading && !isError && (
        <>
          {/* Fleet summary bar */}
          {agents.length > 0 && (
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
              <span data-testid="fleet-online-count">
                <span style={{ color: 'var(--ok)', fontWeight: 600 }}>{onlineCount}</span>
                {' / '}
                {totalCount} online
              </span>
            </div>
          )}

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

          {/* Empty state — no agents registered */}
          {agents.length === 0 && (
            <div className="empty-state" data-testid="empty-state">
              <div className="empty-icon">⊙</div>
              <div className="empty-title">No agents registered</div>
              <div className="empty-body">
                No agents have been registered yet. Start an agent with{' '}
                <code>edgeplane agent register</code>.
              </div>
            </div>
          )}

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
      )}
    </div>
  );
}
