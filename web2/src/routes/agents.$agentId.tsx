/**
 * Agent detail screen — Phase 1 React migration.
 *
 * Data source: GET /api/agents/{agent_id} (typed via schema.gen.ts)
 * Route param: agentId — public_id string (e.g. `aria-operator-e8820c0d`) or
 *              numeric id; the API accepts both (per AgentIdent in the backend).
 *
 * 404 from the API is rendered as a distinct not-found affordance.
 * Back-link returns to /agents list.
 *
 * Cadence: refetchInterval 30s (matches Svelte list cadence; detail is live).
 *
 * Svelte detail note: the Svelte page (web/src/routes/agents/[agentId]/+page.svelte)
 * primarily renders an AgentConversation ACP pane — not a metadata grid. The React
 * implementation renders the full agent record instead; the ACP conversation pane is
 * out of scope for Phase 1 (requires xterm.js wiring not yet ported).
 */

import { apiClient } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { Link, createFileRoute } from '@tanstack/react-router';

// ── Generated schema types ─────────────────────────────────────────────────────

type Agent = components['schemas']['Agent'];

// ── Route ──────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/agents/$agentId')({
  component: AgentDetailPage,
});

// ── Helpers ────────────────────────────────────────────────────────────────────

function fmtDate(s: string | null | undefined): string {
  if (!s) return '—';
  return new Date(s).toLocaleString();
}

function statusVariant(status: string): 'ok' | 'warn' | 'err' | 'default' {
  if (status === 'online' || status === 'active') return 'ok';
  if (status === 'busy') return 'warn';
  if (status === 'offline' || status === 'archived') return 'err';
  return 'default';
}

function Tag({
  variant = 'default',
  children,
}: {
  variant?: 'ok' | 'warn' | 'err' | 'accent' | 'purple' | 'default';
  children: React.ReactNode;
}) {
  return <span className={`tag ${variant !== 'default' ? variant : ''}`}>{children}</span>;
}

/** Parse the `metadata` JSON string into a display-friendly object. */
function parseMetadata(raw: string): Record<string, string> {
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) {
      return Object.fromEntries(
        Object.entries(parsed).map(([k, v]) => [k, typeof v === 'string' ? v : JSON.stringify(v)]),
      );
    }
  } catch {
    // not valid JSON — treat as opaque string
  }
  return { raw };
}

// ── Sub-components ─────────────────────────────────────────────────────────────

function AgentMetaDl({ agent }: { agent: Agent }) {
  return (
    <dl className="policy-meta">
      <dt>Public ID</dt>
      <dd style={{ fontFamily: 'monospace', fontSize: '11px', color: 'var(--accent)' }}>
        {agent.public_id}
      </dd>

      <dt>Name</dt>
      <dd>{agent.name}</dd>

      <dt>Status</dt>
      <dd>
        <Tag variant={statusVariant(agent.status)}>{agent.status}</Tag>
      </dd>

      <dt>Capabilities</dt>
      <dd style={{ fontSize: '11px', color: 'var(--muted)' }}>{agent.capabilities || '—'}</dd>

      <dt>Home Domain</dt>
      <dd style={{ fontSize: '11px', color: 'var(--dim)' }}>{agent.home_domain_id ?? '—'}</dd>

      <dt>Current Domain</dt>
      <dd style={{ fontSize: '11px', color: 'var(--dim)' }}>{agent.current_domain_id ?? '—'}</dd>

      <dt>Created</dt>
      <dd>{fmtDate(agent.created_at)}</dd>

      <dt>Updated</dt>
      <dd>{fmtDate(agent.updated_at)}</dd>

      <dt>Internal ID</dt>
      <dd style={{ fontSize: '11px', color: 'var(--dim)' }}>{agent.id}</dd>
    </dl>
  );
}

function MetadataBlock({ metadata }: { metadata: string }) {
  const parsed = parseMetadata(metadata);
  const entries = Object.entries(parsed);
  if (entries.length === 0) return null;

  return (
    <div style={{ marginTop: '12px' }}>
      <p className="section-label">Runtime Metadata</p>
      <table className="action-table">
        <tbody>
          {entries.map(([k, v]) => (
            <tr key={k}>
              <td className="dim" style={{ fontSize: '11px', whiteSpace: 'nowrap' }}>
                {k}
              </td>
              <td style={{ fontSize: '11px', wordBreak: 'break-all' }}>{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ── Main page ──────────────────────────────────────────────────────────────────

// Named export for direct use in tests
export function AgentDetailPage() {
  const { agentId } = Route.useParams();

  const agentQuery = useQuery({
    queryKey: queryKeys.agents.detail(agentId),
    queryFn: async () => {
      const { data, error, response } = await apiClient.GET('/api/agents/{agent_id}', {
        params: { path: { agent_id: agentId } },
      });
      if (response.status === 404) {
        // Distinct not-found signal — throw with a sentinel so the UI
        // can show a targeted affordance instead of a generic error.
        const notFound = Object.assign(new Error('agent not found'), { status: 404 });
        throw notFound;
      }
      if (error !== undefined || !response.ok) {
        throw new Error(`Request failed: ${response.status}`);
      }
      return data as Agent;
    },
    refetchInterval: 30_000,
  });

  const agent = agentQuery.data;
  const is404 =
    agentQuery.isError &&
    typeof agentQuery.error === 'object' &&
    agentQuery.error !== null &&
    'status' in agentQuery.error &&
    (agentQuery.error as { status: number }).status === 404;

  return (
    <div className="gov-page">
      {/* Top bar */}
      <div className="gov-bar">
        <Link to="/agents" className="ghost" style={{ fontSize: '11px', marginRight: '8px' }}>
          ← Agents
        </Link>
        <span className="gov-title" style={{ fontFamily: 'monospace', color: 'var(--accent)' }}>
          {agentId}
        </span>
        <span className="muted" style={{ fontSize: '11px' }}>
          Agent detail
        </span>
      </div>

      {/* Loading */}
      {agentQuery.isLoading && (
        <div style={{ padding: '12px' }}>
          <p className="muted" data-testid="loading-state">
            Loading agent…
          </p>
        </div>
      )}

      {/* Not found */}
      {is404 && (
        <div className="empty-state" data-testid="not-found-state">
          <div className="empty-icon">⊘</div>
          <div className="empty-title">Agent not found</div>
          <div className="empty-body">
            No agent with id <code>{agentId}</code> is registered on this control plane.
          </div>
        </div>
      )}

      {/* Generic error */}
      {agentQuery.isError && !is404 && (
        <div style={{ padding: '12px' }}>
          <p className="error" data-testid="error-state">
            Failed to load agent — {(agentQuery.error as Error)?.message ?? 'unknown error'}
          </p>
        </div>
      )}

      {/* Data */}
      {agent && (
        <div className="pane-row" style={{ flex: 1, minHeight: 0 }}>
          <div className="pane" style={{ flex: 1, minWidth: 0 }}>
            <div className="pane-header">
              <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                <span className="pane-title">{agent.name}</span>
                <Tag variant={statusVariant(agent.status)}>{agent.status}</Tag>
              </div>
            </div>
            <div className="pane-body" style={{ padding: '10px' }}>
              <AgentMetaDl agent={agent} />
              <MetadataBlock metadata={agent.metadata} />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
