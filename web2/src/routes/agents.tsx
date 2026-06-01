/**
 * Agents list screen — Phase 1 React migration.
 *
 * Data sources:
 *   - GET /api/agents        (typed via schema.gen.ts — control-plane agents)
 *   - GET /api/runtime/nodes (typed via schema.gen.ts — runtime nodes for mesh merge)
 *   - GET /api/runtime/nodes/{node_id}/agents (NodeMeshAgent list per node)
 *
 * Merge strategy: mirrors the Svelte page in web/src/routes/agents/+page.svelte.
 *   - Control-plane agents are the primary source (name, capabilities, status).
 *   - Mesh agents augment with live status and add mesh-only agents if missing.
 *   - Keyed by public_id; mesh status wins on conflict.
 *   - Result sorted alphabetically by name.
 *
 * Cadence: refetchInterval 30s (matches Svelte page + Svelte fleet.ts CACHE_TTL_MS).
 *
 * Navigation: row click → /agents/$agentId (detail).
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, useNavigate } from '@tanstack/react-router';

// ── Generated schema types ─────────────────────────────────────────────────────

type Agent = components['schemas']['Agent'];
type NodeMeshAgent = components['schemas']['NodeMeshAgent'];
type RuntimeNode = components['schemas']['RuntimeNode'];

// ── Route ──────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/agents')({
  component: AgentsPage,
});

// ── Merged agent row ───────────────────────────────────────────────────────────

/** Source of truth for this row. */
type AgentSource = 'controlplane' | 'mesh' | 'both';

type MergedAgent = {
  public_id: string;
  name: string;
  status: string;
  capabilities: string;
  source: AgentSource;
  /** Present when source includes controlplane */
  metadata?: string;
  /** Domain name when joined server-side (control-plane only) */
  domain_name?: string | null;
  updated_at?: string;
  /** runtime_kind from mesh layer */
  runtime_kind?: string;
  /** last_heartbeat_at from mesh layer */
  last_heartbeat_at?: string | null;
};

// ── Helpers ────────────────────────────────────────────────────────────────────

function fmtDate(s: string | null | undefined): string {
  if (!s) return '—';
  return new Date(s).toLocaleString();
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
  return fmtDate(s);
}

function parseCapabilities(raw: string | undefined | unknown): string {
  if (!raw) return '';
  if (typeof raw === 'string') return raw;
  if (Array.isArray(raw)) return raw.join(', ');
  return String(raw);
}

function metadataField(metadata: string | undefined, key: string): string {
  if (!metadata) return '—';
  try {
    const parsed = JSON.parse(metadata);
    return typeof parsed === 'object' && parsed !== null && key in parsed
      ? String(parsed[key])
      : '—';
  } catch {
    return '—';
  }
}

function statusVariant(status: string): 'ok' | 'warn' | 'err' | 'default' {
  if (status === 'online' || status === 'active') return 'ok';
  if (status === 'busy') return 'warn';
  if (status === 'offline' || status === 'archived') return 'err';
  return 'default';
}

function sourceVariant(source: AgentSource): string {
  if (source === 'both') return 'ok';
  if (source === 'mesh') return 'accent';
  return '';
}

// Inline tag — mirrors app.css `.tag` classes from governance.tsx
function Tag({
  variant = 'default',
  children,
}: {
  variant?: 'ok' | 'warn' | 'err' | 'accent' | 'purple' | 'default';
  children: React.ReactNode;
}) {
  return <span className={`tag ${variant !== 'default' ? variant : ''}`}>{children}</span>;
}

// ── Merge logic ────────────────────────────────────────────────────────────────

/** Merge control-plane + mesh agents (same strategy as Svelte fleet page). */
function mergeAgents(cpAgents: Agent[], meshAgents: NodeMeshAgent[]): MergedAgent[] {
  const byId = new Map<string, MergedAgent>();

  for (const a of cpAgents) {
    byId.set(a.public_id, {
      public_id: a.public_id,
      name: a.name,
      status: a.status,
      capabilities: a.capabilities,
      source: 'controlplane',
      metadata: a.metadata,
      updated_at: a.updated_at,
    });
  }

  for (const a of meshAgents) {
    const pid = a.public_id ?? a.agent_public_id ?? a.id;
    const existing = byId.get(pid);
    if (existing) {
      // Mesh status wins; keep controlplane capabilities/metadata
      existing.status = a.status;
      existing.source = 'both';
      existing.runtime_kind = a.runtime_kind;
      existing.last_heartbeat_at = a.last_heartbeat_at;
    } else {
      byId.set(pid, {
        public_id: pid,
        name: pid,
        status: a.status,
        capabilities: parseCapabilities(a.capabilities),
        source: 'mesh',
        runtime_kind: a.runtime_kind,
        last_heartbeat_at: a.last_heartbeat_at,
        domain_name: a.domain_name,
      });
    }
  }

  return Array.from(byId.values()).sort((a, b) => a.name.localeCompare(b.name));
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

// ── Sub-components ─────────────────────────────────────────────────────────────

function AgentRow({
  agent,
  onClick,
}: {
  agent: MergedAgent;
  onClick: () => void;
}) {
  const sv = statusVariant(agent.status);
  const srcClass = sourceVariant(agent.source);

  return (
    <tr
      style={{ cursor: 'pointer' }}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') onClick();
      }}
      tabIndex={0}
      data-testid={`agent-row-${agent.public_id}`}
    >
      <td>
        <Tag variant={sv}>{agent.status}</Tag>
      </td>
      <td style={{ fontFamily: 'monospace', fontSize: '11px', color: 'var(--accent)' }}>
        {agent.public_id}
      </td>
      <td>{agent.name}</td>
      <td className="caps" style={{ fontSize: '11px', color: 'var(--muted)' }}>
        {agent.capabilities || '—'}
      </td>
      <td style={{ fontSize: '10px', color: 'var(--dim)' }}>
        {agent.source === 'controlplane' || agent.source === 'both'
          ? metadataField(agent.metadata, 'runtime')
          : (agent.runtime_kind ?? '—')}
      </td>
      <td style={{ fontSize: '10px', color: 'var(--dim)' }}>
        {agent.source === 'controlplane' || agent.source === 'both'
          ? metadataField(agent.metadata, 'node_id')
          : '—'}
      </td>
      <td style={{ fontSize: '10px', color: 'var(--muted)' }}>
        {agent.last_heartbeat_at
          ? fmtRelative(agent.last_heartbeat_at)
          : fmtRelative(agent.updated_at)}
      </td>
      <td>
        <span className={srcClass ? `tag ${srcClass}` : 'tag'}>{agent.source}</span>
      </td>
    </tr>
  );
}

// ── Main page ──────────────────────────────────────────────────────────────────

// Named export for direct use in tests (avoids router context requirement)
export function AgentsPage() {
  const navigate = useNavigate();

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

  const isLoading = cpQuery.isLoading && meshQuery.isLoading;
  const isError = cpQuery.isError && meshQuery.isError;

  return (
    <div className="gov-page">
      {/* Top bar */}
      <div className="gov-bar">
        <span className="gov-title">Agents</span>
        <span className="muted" style={{ fontSize: '11px' }}>
          Registered fleet agents — click a row for details
        </span>
      </div>

      {/* Loading */}
      {isLoading && (
        <div style={{ padding: '12px' }}>
          <p className="muted" data-testid="loading-state">
            Loading agents…
          </p>
        </div>
      )}

      {/* Error */}
      {isError && (
        <div style={{ padding: '12px' }}>
          <p className="error" data-testid="error-state">
            Failed to load agents —{' '}
            {(cpQuery.error as Error)?.message ??
              (meshQuery.error as Error)?.message ??
              'unknown error'}
          </p>
        </div>
      )}

      {/* Table */}
      {!isLoading && !isError && agents.length > 0 && (
        <div style={{ flex: 1, overflow: 'auto' }}>
          <table
            style={{ width: '100%', borderCollapse: 'collapse', fontSize: '12px' }}
            data-testid="agents-table"
          >
            <thead>
              <tr>
                {[
                  'Status',
                  'Public ID',
                  'Name',
                  'Capabilities',
                  'Runtime',
                  'Node',
                  'Last Seen',
                  'Source',
                ].map((col) => (
                  <th
                    key={col}
                    style={{
                      textAlign: 'left',
                      padding: '4px 10px',
                      borderBottom: '1px solid var(--border)',
                      color: 'var(--dim)',
                      fontWeight: 400,
                      fontSize: '10px',
                      textTransform: 'uppercase',
                      letterSpacing: '0.05em',
                      background: 'var(--surface)',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {col}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {agents.map((agent) => (
                <AgentRow
                  key={agent.public_id}
                  agent={agent}
                  onClick={() =>
                    navigate({ to: '/agents/$agentId', params: { agentId: agent.public_id } })
                  }
                />
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Empty state */}
      {!isLoading && !isError && agents.length === 0 && (
        <div className="empty-state" data-testid="empty-state">
          <div className="empty-icon">⊙</div>
          <div className="empty-title">No agents registered</div>
          <div className="empty-body">
            No control-plane agents have been registered yet. Start an agent with{' '}
            <code>edgeplane agent register</code>.
          </div>
        </div>
      )}
    </div>
  );
}
