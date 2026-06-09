import type { components } from '@/api/schema.gen';

type RuntimeNode = components['schemas']['RuntimeNode'];

function statusColor(status: string): string {
  const v = status.toLowerCase();
  if (v === 'online') return 'var(--ok)';
  if (v === 'offline' || v === 'cordoned') return 'var(--err)';
  if (v === 'draining') return 'var(--warn)';
  return 'var(--dim)';
}

function relativeTime(iso: string | null | undefined): string {
  if (!iso) return '—';
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

interface NodesTableProps {
  nodes: RuntimeNode[];
  isLoading: boolean;
  onRowClick: (node: RuntimeNode) => void;
}

export function NodesTable({ nodes, isLoading, onRowClick }: NodesTableProps) {
  if (isLoading) {
    return (
      <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>
        Loading…
      </div>
    );
  }
  if (nodes.length === 0) {
    return (
      <div data-testid="empty-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>
        No nodes registered.
      </div>
    );
  }

  return (
    <div data-testid="nodes-table" style={{ overflowX: 'auto' }}>
      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
        <thead>
          <tr style={{ borderBottom: '1px solid var(--border)' }}>
            {['Status', 'Name', 'Tailscale FQDN', 'Version', 'Heartbeat'].map((col) => (
              <th
                key={col}
                style={{
                  padding: '8px 12px',
                  textAlign: 'left',
                  color: 'var(--dim)',
                  fontWeight: 510,
                  fontSize: 11,
                }}
              >
                {col}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {nodes.map((node) => (
            <tr
              key={node.id}
              data-testid={`node-row-${node.id}`}
              onClick={() => onRowClick(node)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') onRowClick(node);
              }}
              tabIndex={0}
              style={{ borderBottom: '1px solid var(--border-subtle)', cursor: 'pointer' }}
            >
              <td style={{ padding: '8px 12px' }}>
                <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
                  <span
                    style={{
                      width: 7,
                      height: 7,
                      borderRadius: '50%',
                      background: statusColor(node.status),
                      flexShrink: 0,
                    }}
                  />
                  <span style={{ color: statusColor(node.status), fontSize: 11 }}>
                    {node.status}
                  </span>
                </span>
              </td>
              <td style={{ padding: '8px 12px', color: 'var(--text)', fontWeight: 510 }}>
                {node.node_name}
              </td>
              <td
                style={{
                  padding: '8px 12px',
                  color: 'var(--text-2)',
                  fontFamily: 'var(--mono)',
                  fontSize: 12,
                }}
              >
                {node.tailscale_fqdn ?? '—'}
              </td>
              <td
                style={{
                  padding: '8px 12px',
                  color: 'var(--text-2)',
                  fontFamily: 'var(--mono)',
                  fontSize: 12,
                }}
              >
                {node.runtime_version}
              </td>
              <td style={{ padding: '8px 12px', color: 'var(--dim)', fontSize: 12 }}>
                {relativeTime(node.last_heartbeat_at)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
