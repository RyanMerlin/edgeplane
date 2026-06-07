/**
 * AgentsTable — presentational fleet agents table.
 *
 * Extracted from the former /agents route so it can be embedded as a sub-view
 * of the Fleet dashboard (routes/index.tsx). Pure presentation: the merged
 * agent list + loading/error state are passed in (see lib/useMergedAgents.ts),
 * and row clicks are delegated to the caller (which navigates to the detail
 * route /agents/$agentId).
 */

import type { MergedAgent } from '@/lib/useMergedAgents';

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

/**
 * Resolve a display value preferring the agent's self-reported metadata, then
 * falling back to a value derived from the live mesh topology. Either source
 * may be absent (controlplane agents register with empty `{}` metadata; mesh
 * rows exist only once an agent is enrolled on a node), so we coalesce both.
 */
function coalesceField(
  metadata: string | undefined,
  key: string,
  fallback: string | null | undefined,
): string {
  const fromMeta = metadataField(metadata, key);
  if (fromMeta !== '—') return fromMeta;
  return fallback ?? '—';
}

function statusVariant(status: string): 'ok' | 'warn' | 'err' | 'default' {
  if (status === 'online' || status === 'active') return 'ok';
  if (status === 'busy') return 'warn';
  if (status === 'offline' || status === 'archived') return 'err';
  return 'default';
}

function sourceVariant(source: MergedAgent['source']): string {
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

// ── Row ────────────────────────────────────────────────────────────────────────

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
        {coalesceField(agent.metadata, 'runtime', agent.runtime_kind)}
      </td>
      <td style={{ fontSize: '10px', color: 'var(--dim)' }}>
        {coalesceField(agent.metadata, 'node_id', agent.node_name)}
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

// ── Table ────────────────────────────────────────────────────────────────────

export interface AgentsTableProps {
  agents: MergedAgent[];
  isLoading: boolean;
  isError: boolean;
  error: Error | null;
  onRowClick: (agent: MergedAgent) => void;
}

export function AgentsTable({ agents, isLoading, isError, error, onRowClick }: AgentsTableProps) {
  if (isLoading) {
    return (
      <div style={{ padding: '12px' }}>
        <p className="muted" data-testid="loading-state">
          Loading agents…
        </p>
      </div>
    );
  }

  if (isError) {
    return (
      <div style={{ padding: '12px' }}>
        <p className="error" data-testid="error-state">
          Failed to load agents — {error?.message ?? 'unknown error'}
        </p>
      </div>
    );
  }

  if (agents.length === 0) {
    return (
      <div className="empty-state" data-testid="empty-state">
        <div className="empty-icon">⊙</div>
        <div className="empty-title">No agents registered</div>
        <div className="empty-body">
          No control-plane agents have been registered yet. Start an agent with{' '}
          <code>edgeplane agent register</code>.
        </div>
      </div>
    );
  }

  // Alphabetical by name — deterministic row order (display concern; the merge
  // hook preserves registration order).
  const rows = [...agents].sort((a, b) => a.name.localeCompare(b.name));

  return (
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
          {rows.map((agent) => (
            <AgentRow key={agent.public_id} agent={agent} onClick={() => onRowClick(agent)} />
          ))}
        </tbody>
      </table>
    </div>
  );
}
