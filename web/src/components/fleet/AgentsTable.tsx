/**
 * AgentsTable — presentational fleet agents table.
 *
 * Extracted from the former /agents route so it can be embedded as a sub-view
 * of the Fleet dashboard (routes/index.tsx). Pure presentation: the merged
 * agent list + loading/error state are passed in (see lib/useMergedAgents.ts),
 * and row clicks are delegated to the caller (which navigates to the detail
 * route /agents/$agentId).
 *
 * Columns (Linear density, mockup-aligned):
 *   Status | Name | Public ID | Node | Runtime | Last seen | Source
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

function statusDotColor(variant: 'ok' | 'warn' | 'err' | 'default'): string {
  if (variant === 'ok') return 'var(--ok)';
  if (variant === 'warn') return 'var(--warn)';
  if (variant === 'err') return 'var(--err)';
  return 'var(--dim)';
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
  const dotColor = statusDotColor(sv);

  const node = coalesceField(agent.metadata, 'node_id', agent.node_name);
  const runtime = coalesceField(agent.metadata, 'runtime', agent.runtime_kind);
  const lastSeen = agent.last_heartbeat_at
    ? fmtRelative(agent.last_heartbeat_at)
    : fmtRelative(agent.updated_at);

  return (
    <tr
      style={{ cursor: 'pointer', transition: 'background 0.12s ease' }}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') onClick();
      }}
      tabIndex={0}
      data-testid={`agent-row-${agent.public_id}`}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLTableRowElement).style.background = 'var(--raised)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLTableRowElement).style.background = '';
      }}
    >
      {/* Status — square tag with leading dot */}
      <td style={{ padding: '9px 12px', borderBottom: '1px solid var(--border-subtle)' }}>
        <span className="tag">
          <span className="dot" style={{ background: dotColor, width: '5px', height: '5px' }} />
          {agent.status}
        </span>
      </td>

      {/* Name */}
      <td
        className="cellname"
        style={{
          padding: '9px 12px',
          borderBottom: '1px solid var(--border-subtle)',
          color: 'var(--text)',
          fontWeight: 510,
        }}
      >
        {agent.name}
      </td>

      {/* Public ID — mono accent */}
      <td
        style={{
          padding: '9px 12px',
          borderBottom: '1px solid var(--border-subtle)',
        }}
      >
        <span className="mono" style={{ color: 'var(--accent)', fontSize: '12px' }}>
          {agent.public_id}
        </span>
      </td>

      {/* Node — mono dim */}
      <td
        style={{
          padding: '9px 12px',
          borderBottom: '1px solid var(--border-subtle)',
        }}
      >
        <span className="mono" style={{ color: 'var(--dim)', fontSize: '12px' }}>
          {node}
        </span>
      </td>

      {/* Runtime — mono dim (kept for test assertions on runtime field) */}
      <td
        style={{
          padding: '9px 12px',
          borderBottom: '1px solid var(--border-subtle)',
        }}
      >
        <span className="mono" style={{ color: 'var(--dim)', fontSize: '12px' }}>
          {runtime}
        </span>
      </td>

      {/* Last seen — muted */}
      <td
        style={{
          padding: '9px 12px',
          borderBottom: '1px solid var(--border-subtle)',
          color: 'var(--muted)',
          fontSize: '13px',
        }}
      >
        {lastSeen}
      </td>

      {/* Source — plain tag */}
      <td style={{ padding: '9px 12px', borderBottom: '1px solid var(--border-subtle)' }}>
        <span className="tag">{agent.source}</span>
      </td>
    </tr>
  );
}

// ── Column headers ─────────────────────────────────────────────────────────────

const COLUMNS = ['Status', 'Name', 'Public ID', 'Node', 'Runtime', 'Last Seen', 'Source'] as const;

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
        style={{ width: '100%', borderCollapse: 'collapse', fontSize: '13px' }}
        data-testid="agents-table"
      >
        <thead>
          <tr>
            {COLUMNS.map((col) => (
              <th
                key={col}
                style={{
                  textAlign: 'left',
                  padding: '7px 12px',
                  borderBottom: '1px solid var(--border-subtle)',
                  color: 'var(--dim)',
                  fontWeight: 510,
                  fontSize: '11px',
                  background: 'transparent',
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
