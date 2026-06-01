/**
 * Fleet dashboard — Phase 4 landing page.
 *
 * Ports structure from web/src/routes/+page.svelte (Svelte home):
 *   - FLEET_PROFILES order: operator, engineer, merlinlabs, publisher, work, research
 *   - Profile-to-agent mapping via profileName() parsing "aria-<profile>-<8hex>" names
 *   - Keyboard hotkeys: Ctrl/Meta+1..6 switch tabs by index (same as Svelte bindings)
 *   - Per-profile status dot + last-seen from the agent record
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
import { useCallback, useEffect, useState } from 'react';

// ── Route ──────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/')({
  component: FleetDashboard,
});

// ── Types ─────────────────────────────────────────────────────────────────────

type Agent = components['schemas']['Agent'];
type NodeMeshAgent = components['schemas']['NodeMeshAgent'];
type RuntimeNode = components['schemas']['RuntimeNode'];

// ── Fleet profile order (ported from Svelte FLEET_PROFILES) ───────────────────

const FLEET_PROFILES = [
  'operator',
  'engineer',
  'merlinlabs',
  'publisher',
  'work',
  'research',
] as const;
type FleetProfile = (typeof FLEET_PROFILES)[number];

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

/**
 * Extract a short profile name from agent IDs like "aria-operator-e8820c0d".
 * Ported from web/src/lib/api/fleet.ts profileName().
 */
function profileName(agentId: string): string {
  const parts = agentId.split('-');
  if (parts.length >= 3 && parts[0] === 'aria') {
    return parts.slice(1, -1).join('-');
  }
  return agentId;
}

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

interface ProfilePaneProps {
  profile: FleetProfile;
  agent: MergedAgent | undefined;
  isActive: boolean;
}

function ProfilePane({ profile, agent, isActive }: ProfilePaneProps) {
  const nodeId = agent ? resolveNodeId(agent.metadata) : null;

  // Derive last-seen from whichever timestamp is available
  const lastSeen = agent?.last_heartbeat_at ?? agent?.updated_at;

  return (
    <div
      style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}
      data-testid={`profile-pane-${profile}`}
    >
      {/* Profile status header */}
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
        data-testid={`profile-status-${profile}`}
      >
        <span
          aria-label={agent ? `status: ${agent.status}` : 'not registered'}
          style={{
            width: '7px',
            height: '7px',
            borderRadius: '50%',
            background: agent ? statusColor(agent.status) : '#333',
            flexShrink: 0,
            display: 'inline-block',
          }}
        />
        <span style={{ fontFamily: 'monospace', color: 'var(--text)', fontWeight: 600 }}>
          {profile}
        </span>
        {agent ? (
          <>
            <span
              className={`tag ${agent.status === 'online' || agent.status === 'active' ? 'ok' : ''}`}
              data-testid={`profile-status-badge-${profile}`}
            >
              {agent.status}
            </span>
            {nodeId && (
              <span
                style={{ color: 'var(--dim)', fontSize: '10px' }}
                data-testid={`profile-node-${profile}`}
              >
                {nodeId}
              </span>
            )}
            <span style={{ marginLeft: 'auto', fontSize: '10px' }}>{fmtRelative(lastSeen)}</span>
          </>
        ) : (
          <span style={{ color: 'var(--dim)', fontSize: '10px' }}>not registered</span>
        )}
      </div>

      {/* Conversation pane or fallback */}
      <div style={{ flex: 1, minHeight: 0 }}>
        {!agent && (
          <div
            style={{
              padding: '24px 16px',
              color: 'var(--muted)',
              fontSize: '13px',
              textAlign: 'center',
            }}
            data-testid={`profile-unregistered-${profile}`}
          >
            <div style={{ fontWeight: 600, marginBottom: '4px' }}>{profile}</div>
            <div>Not registered in EdgePlane.</div>
            <div style={{ fontSize: '11px', marginTop: '4px', color: 'var(--dim)' }}>
              Check that edgeplaned has imported this profile.
            </div>
          </div>
        )}
        {agent && !nodeId && (
          <div
            style={{
              padding: '24px 16px',
              color: 'var(--muted)',
              fontSize: '13px',
              textAlign: 'center',
            }}
            data-testid={`profile-not-attachable-${profile}`}
          >
            <div style={{ fontWeight: 600, marginBottom: '4px' }}>Not attachable</div>
            <div>
              Agent <code>{agent.public_id}</code> has no <code>node_id</code> in its metadata.
            </div>
          </div>
        )}
        {agent && nodeId && isActive && <AcpPane nodeId={nodeId} agentId={agent.public_id} />}
      </div>
    </div>
  );
}

// ── Main component ─────────────────────────────────────────────────────────────

export function FleetDashboard() {
  const [activeProfile, setActiveProfile] = useState<FleetProfile>(FLEET_PROFILES[0]);

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

  /** Find the agent record for a fleet profile using the naming convention. */
  const agentForProfile = useCallback(
    (profile: FleetProfile): MergedAgent | undefined => {
      return agents.find(
        (a) => profileName(a.public_id) === profile || profileName(a.name) === profile,
      );
    },
    [agents],
  );

  // Keyboard hotkeys: Ctrl/Meta+1..6 switch profiles (ported from Svelte handleKeydown)
  useEffect(() => {
    function handleKeydown(e: KeyboardEvent) {
      if (!(e.ctrlKey || e.metaKey)) return;
      const num = Number.parseInt(e.key, 10);
      if (num >= 1 && num <= FLEET_PROFILES.length) {
        e.preventDefault();
        setActiveProfile(FLEET_PROFILES[num - 1]);
      }
    }
    document.addEventListener('keydown', handleKeydown);
    return () => document.removeEventListener('keydown', handleKeydown);
  }, []);

  const isLoading = cpQuery.isLoading && meshQuery.isLoading;
  const isError = cpQuery.isError && meshQuery.isError;

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

          {/* Profile tabs (session-tabs from Svelte) */}
          <div
            style={{
              display: 'flex',
              gap: 0,
              padding: '0 0.25rem',
              borderBottom: '1px solid var(--border)',
              flexShrink: 0,
            }}
            role="tablist"
            aria-label="Fleet profiles"
            data-testid="profile-tabs"
          >
            {FLEET_PROFILES.map((profile, i) => {
              const agent = agentForProfile(profile);
              const isActive = activeProfile === profile;
              const unavailable = !agent;
              return (
                <button
                  key={profile}
                  type="button"
                  role="tab"
                  aria-selected={isActive}
                  aria-controls={`panel-${profile}`}
                  id={`tab-${profile}`}
                  disabled={unavailable}
                  onClick={() => {
                    if (!unavailable) setActiveProfile(profile);
                  }}
                  title={
                    agent
                      ? `${profile} (${agent.status}) — Ctrl+${i + 1}`
                      : `${profile} — not registered`
                  }
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: '0.4rem',
                    padding: '0.5rem 0.75rem',
                    background: 'none',
                    border: 'none',
                    borderBottom: isActive ? '2px solid var(--accent)' : '2px solid transparent',
                    color: unavailable ? 'var(--dim)' : isActive ? 'var(--text)' : 'var(--muted)',
                    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
                    fontSize: '13px',
                    cursor: unavailable ? 'not-allowed' : 'pointer',
                    opacity: unavailable ? 0.35 : 1,
                  }}
                  data-testid={`tab-${profile}`}
                >
                  <span
                    aria-hidden="true"
                    style={{
                      width: '6px',
                      height: '6px',
                      borderRadius: '50%',
                      background: agent ? statusColor(agent.status) : '#333',
                      flexShrink: 0,
                      display: 'inline-block',
                    }}
                  />
                  {profile}
                </button>
              );
            })}
          </div>

          {/* Active profile pane */}
          <div
            role="tabpanel"
            id={`panel-${activeProfile}`}
            aria-labelledby={`tab-${activeProfile}`}
            style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}
            data-testid="active-profile-panel"
          >
            <ProfilePane
              key={activeProfile}
              profile={activeProfile}
              agent={agentForProfile(activeProfile)}
              isActive
            />
          </div>
        </>
      )}
    </div>
  );
}
