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
 * Layout (C3 mockup conformance):
 *   - id-strip: status dot, name, mono public_id, status tag, last-seen
 *   - detail-body: detail-main (conversation, flex:1) + props rail (266px)
 *
 * ACP pane:
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

function fmtLastSeen(s: string | null | undefined): string {
  if (!s) return '—';
  const diff = Date.now() - new Date(s).getTime();
  const secs = Math.floor(diff / 1000);
  if (secs < 60) return `${secs}s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  return `${Math.floor(mins / 60)}h ago`;
}

function statusVariant(status: string): 'ok' | 'warn' | 'err' | 'default' {
  if (status === 'online' || status === 'active') return 'ok';
  if (status === 'busy') return 'warn';
  if (status === 'offline' || status === 'archived') return 'err';
  return 'default';
}

function statusColor(variant: 'ok' | 'warn' | 'err' | 'default'): string {
  if (variant === 'ok') return 'var(--ok)';
  if (variant === 'warn') return 'var(--warn)';
  if (variant === 'err') return 'var(--err)';
  return 'var(--dim)';
}

function Tag({
  variant = 'default',
  children,
}: {
  variant?: 'ok' | 'warn' | 'err' | 'accent' | 'purple' | 'default';
  children: React.ReactNode;
}) {
  const dotColor =
    variant === 'ok'
      ? 'var(--ok)'
      : variant === 'warn'
        ? 'var(--warn)'
        : variant === 'err'
          ? 'var(--err)'
          : variant === 'accent'
            ? 'var(--accent)'
            : undefined;

  return (
    <span className={`tag ${variant !== 'default' ? variant : ''}`}>
      {dotColor && (
        <span className="dot" style={{ background: dotColor, width: '5px', height: '5px' }} />
      )}
      {children}
    </span>
  );
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

// ── Properties rail ─────────────────────────────────────────────────────────

function PropRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="prop">
      <div className="k">{label}</div>
      <div className="v">{children}</div>
    </div>
  );
}

function PropsRail({ agent, nodeId }: { agent: Agent; nodeId: string | null }) {
  const meta = parseMetadata(agent.metadata ?? '{}');
  const runtime = meta.runtime ?? '—';
  const variant = statusVariant(agent.status);

  return (
    <aside
      className="props"
      style={{
        width: '266px',
        flexShrink: 0,
        borderLeft: '1px solid var(--border-subtle)',
        padding: '16px 16px 16px 20px',
        overflowY: 'auto',
      }}
    >
      <h4
        style={{
          margin: '0 0 14px',
          fontSize: '11px',
          fontWeight: 590,
          color: 'var(--dim)',
          letterSpacing: '0.02em',
        }}
      >
        PROPERTIES
      </h4>

      <PropRow label="Status">
        <Tag variant={variant}>{agent.status}</Tag>
      </PropRow>

      <PropRow label="Node">
        {nodeId ? (
          <span
            style={{ fontFamily: 'var(--mono)', fontSize: '12px', color: 'var(--text-2)' }}
            data-testid="acp-node-id"
          >
            {nodeId}
          </span>
        ) : (
          <span style={{ color: 'var(--dim)', fontSize: '12px' }}>—</span>
        )}
      </PropRow>

      <PropRow label="Runtime">
        <span style={{ fontFamily: 'var(--mono)', fontSize: '12px' }}>{runtime}</span>
      </PropRow>

      <PropRow label="Capabilities">
        {agent.capabilities ? (
          <span style={{ fontSize: '13px', color: 'var(--text-2)' }}>{agent.capabilities}</span>
        ) : (
          <span style={{ color: 'var(--dim)', fontSize: '12px' }}>—</span>
        )}
      </PropRow>

      <PropRow label="Home domain">
        <span style={{ fontSize: '13px', color: 'var(--text-2)' }}>
          {agent.home_domain_id ?? '—'}
        </span>
      </PropRow>

      <PropRow label="Source">
        <Tag variant="default">{agent.current_domain_id ? 'mesh' : 'cp'}</Tag>
      </PropRow>
    </aside>
  );
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

  // For display: hostname from metadata (human-readable).
  const displayNodeId = agent ? resolveNodeId(agent.metadata) : null;
  // For ACP attach: runtime node UUID + agent public_id (what the attach proxy + edgeplaned expect).
  const attachNodeId = agent?.runtime_node_id ?? null;
  const variant = agent ? statusVariant(agent.status) : 'default';

  return (
    <div
      className="detail"
      data-testid="agent-detail"
      style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}
    >
      {/* ── Identity strip ── */}
      <div
        className="id-strip"
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '10px',
          padding: '14px 18px',
          borderBottom: '1px solid var(--border-subtle)',
          flexShrink: 0,
        }}
      >
        <span
          className="dot"
          style={{
            background: agent ? statusColor(variant) : 'var(--dim)',
            width: '7px',
            height: '7px',
          }}
        />
        <span className="name" style={{ fontSize: '15px', fontWeight: 590, color: 'var(--text)' }}>
          {agent?.name ?? agentId}
        </span>
        <span
          className="pid"
          style={{ fontFamily: 'var(--mono)', fontSize: '12px', color: 'var(--accent)' }}
        >
          {agent?.public_id ?? agentId}
        </span>
        {agent && <Tag variant={variant}>{agent.status}</Tag>}
        <span style={{ marginLeft: 'auto', color: 'var(--dim)', fontSize: '11px' }}>
          {agent ? fmtLastSeen(agent.updated_at) : ''}
        </span>
      </div>

      {/* ── Loading ── */}
      {agentQuery.isLoading && (
        <div style={{ padding: '12px' }}>
          <p className="muted" data-testid="loading-state">
            Loading agent…
          </p>
        </div>
      )}

      {/* ── Not found ── */}
      {is404 && (
        <div className="empty-state" data-testid="not-found-state">
          <div className="empty-icon">⊘</div>
          <div className="empty-title">Agent not found</div>
          <div className="empty-body">
            No agent with id <code>{agentId}</code> is registered on this control plane.
          </div>
        </div>
      )}

      {/* ── Generic error ── */}
      {agentQuery.isError && !is404 && (
        <div style={{ padding: '12px' }}>
          <p className="error" data-testid="error-state">
            Failed to load agent — {(agentQuery.error as Error)?.message ?? 'unknown error'}
          </p>
        </div>
      )}

      {/* ── Data: two-column body ── */}
      {agent && (
        <div className="detail-body" style={{ flex: 1, minHeight: 0, display: 'flex' }}>
          {/* Left: conversation (detail-main) */}
          <div
            className="detail-main"
            style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }}
          >
            {attachNodeId && agent ? (
              <AcpPane nodeId={attachNodeId} agentId={agent.public_id} />
            ) : (
              <div
                style={{ padding: '20px 18px', color: 'var(--muted)', fontSize: '12px' }}
                data-testid="not-attachable"
              >
                <div style={{ fontWeight: 600, marginBottom: '4px' }}>Not attachable</div>
                <div>
                  This agent is not enrolled under a runtime node. Live conversation requires the
                  agent to be enrolled via the runtime API (edgeplaned registers the ACP session
                  automatically on startup).
                </div>
              </div>
            )}
          </div>

          {/* Right: properties rail */}
          <PropsRail agent={agent} nodeId={displayNodeId} />
        </div>
      )}
    </div>
  );
}
