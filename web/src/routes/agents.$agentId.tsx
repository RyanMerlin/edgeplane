/**
 * Agent detail screen — Phase 4: ACP conversation pane.
 *
 * Data source: GET /api/agents/{agent_id} (typed via schema.gen.ts)
 * Route param: agentId — public_id string (e.g. `aria-operator-e8820c0d`) or
 *              numeric id; the API accepts both (per AgentIdent in the backend).
 *
 * 404 from the API is rendered as a distinct not-found affordance.
 * Up-navigation is via the shell breadcrumb (Agents › <id>), not an in-page link.
 *
 * ACP pane (Phase 4):
 *   - nodeId resolved from agent.metadata.node_id (JSON-parsed)
 *   - If no nodeId is resolvable, a "not attachable" state is shown instead of
 *     a broken WebSocket.
 *   - Uses ConversationView + useAcpConversation hook (transport-agnostic shell).
 */

import { apiClient } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { ConversationView } from '@/components/conversation/ConversationView';
import { useAcpConversation } from '@/lib/conversation/useAcpConversation';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';

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

// ── Helpers ────────────────────────────────────────────────────────────────────

/**
 * Resolve the nodeId used for the ACP attach endpoint.
 *
 * The agent record carries a `metadata` JSON string. We look for `node_id`
 * inside it (set during agent registration via `register_agent` with
 * metadata="{\"node_id\":\"...\"}"`).
 *
 * Returns null when metadata is absent, malformed, or doesn't contain node_id.
 * A null nodeId triggers the "not attachable" affordance rather than a broken socket.
 */
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
    // malformed JSON — not attachable
  }
  return null;
}

// ── ACP pane — inner component so the hook only runs when nodeId is known ─────

function AcpPane({ nodeId, agentId }: { nodeId: string; agentId: string }) {
  const { items, status, send, cancel } = useAcpConversation(nodeId, agentId);
  return <ConversationView items={items} status={status} onSend={send} onCancel={cancel} />;
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

  const nodeId = agent ? resolveNodeId(agent.metadata) : null;

  return (
    <div
      className="gov-page"
      data-testid="agent-detail"
      style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}
    >
      {/* Top bar */}
      <div className="gov-bar">
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
        <div className="pane-row" style={{ flex: 1, minHeight: 0, display: 'flex', gap: '0' }}>
          {/* Left pane: metadata */}
          <div
            className="pane"
            style={{ width: '320px', flexShrink: 0, minWidth: 0, overflowY: 'auto' }}
          >
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

          {/* Right pane: ACP conversation or not-attachable affordance */}
          <div
            className="pane"
            style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}
          >
            <div className="pane-header">
              <span className="pane-title">Live conversation</span>
              {nodeId && (
                <span
                  className="muted"
                  style={{ fontSize: '10px', marginLeft: '8px' }}
                  data-testid="acp-node-id"
                >
                  {nodeId}
                </span>
              )}
            </div>
            <div
              className="pane-body"
              style={{
                flex: 1,
                minHeight: 0,
                padding: 0,
                display: 'flex',
                flexDirection: 'column',
              }}
            >
              {nodeId ? (
                <AcpPane nodeId={nodeId} agentId={agentId} />
              ) : (
                <div
                  style={{ padding: '20px 12px', color: 'var(--muted)', fontSize: '12px' }}
                  data-testid="not-attachable"
                >
                  <div style={{ fontWeight: 600, marginBottom: '4px' }}>Not attachable</div>
                  <div>
                    This agent does not have a <code>node_id</code> in its metadata. To enable the
                    live conversation pane, register the agent with{' '}
                    <code>{`metadata='{"node_id":"<node>"}'`}</code>.
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
