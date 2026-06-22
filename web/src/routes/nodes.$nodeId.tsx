import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { DeleteNodeButton } from '@/components/nodes/DeleteNodeButton';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { Link, createFileRoute, useParams } from '@tanstack/react-router';
import { Fragment } from 'react';

type Agent = components['schemas']['Agent'];
type RuntimeNode = components['schemas']['RuntimeNode'];

export const Route = createFileRoute('/nodes/$nodeId')({
  component: NodeDetailPage,
});

export function NodeDetailPage() {
  const { nodeId } = useParams({ from: '/nodes/$nodeId' });

  const { data: node, isLoading } = useQuery({
    queryKey: queryKeys.nodes.detail(nodeId),
    queryFn: async (): Promise<RuntimeNode> => {
      const res = await fetch(`/api/runtime/nodes/${nodeId}`, { credentials: 'include' });
      if (!res.ok) throw new Error(`node fetch failed: ${res.status}`);
      return res.json();
    },
  });

  const { data: allAgents } = useQuery({
    queryKey: queryKeys.agents.list(),
    queryFn: () => unwrap(apiClient.GET('/api/agents', {})),
  });

  const nodeAgents = ((allAgents ?? []) as Agent[]).filter((a) => {
    try {
      const meta = JSON.parse(a.metadata ?? '{}');
      return meta.node_id === node?.node_name;
    } catch {
      return false;
    }
  });

  if (isLoading || !node) {
    return (
      <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>
        Loading…
      </div>
    );
  }

  const capacityEntries = (() => {
    try {
      return Object.entries(node.capacity as Record<string, unknown>);
    } catch {
      return [];
    }
  })();

  return (
    <div style={{ padding: '16px 24px' }}>
      <div style={{ fontSize: 11, color: 'var(--dim)', marginBottom: 12 }}>
        <Link to="/nodes" style={{ color: 'var(--dim)', textDecoration: 'none' }}>
          Nodes
        </Link>
        {' › '}
        <span style={{ color: 'var(--text)' }}>{node.node_name}</span>
      </div>

      <div data-testid="node-detail-header" style={{ marginBottom: 24 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
          <span style={{ fontSize: 18, fontWeight: 590, color: 'var(--text)' }}>
            {node.node_name}
          </span>
          <span
            style={{
              fontSize: 11,
              color: 'var(--ok)',
              background: 'rgba(87,208,138,0.12)',
              padding: '1px 7px',
              borderRadius: 3,
            }}
          >
            {node.status}
          </span>
          <span
            style={{
              marginLeft: 'auto',
              fontSize: 11,
              color: 'var(--dim)',
              fontFamily: 'var(--mono)',
            }}
          >
            v{node.runtime_version}
          </span>
        </div>
        <dl
          style={{
            display: 'grid',
            gridTemplateColumns: 'max-content 1fr',
            gap: '5px 16px',
            fontSize: 13,
          }}
        >
          <dt style={{ color: 'var(--dim)' }}>Hostname</dt>
          <dd style={{ margin: 0, color: 'var(--text)' }}>{node.hostname}</dd>
          {node.tailscale_fqdn && (
            <>
              <dt style={{ color: 'var(--dim)' }}>Tailscale FQDN</dt>
              <dd
                style={{ margin: 0, color: 'var(--text)', fontFamily: 'var(--mono)', fontSize: 12 }}
              >
                {node.tailscale_fqdn}
              </dd>
            </>
          )}
          {node.tailscale_ip && (
            <>
              <dt style={{ color: 'var(--dim)' }}>Tailscale IP</dt>
              <dd
                style={{ margin: 0, color: 'var(--text)', fontFamily: 'var(--mono)', fontSize: 12 }}
              >
                {node.tailscale_ip}
              </dd>
            </>
          )}
          <dt style={{ color: 'var(--dim)' }}>Trust Tier</dt>
          <dd style={{ margin: 0, color: 'var(--accent)' }}>{node.trust_tier}</dd>
        </dl>
      </div>

      <div data-testid="node-agents-section" style={{ marginBottom: 24 }}>
        <div
          style={{
            fontSize: 11,
            fontWeight: 590,
            color: 'var(--dim)',
            letterSpacing: '0.06em',
            textTransform: 'uppercase',
            marginBottom: 8,
          }}
        >
          Agents ({nodeAgents.length})
        </div>
        {nodeAgents.length === 0 ? (
          <div style={{ color: 'var(--dim)', fontSize: 13 }}>No agents on this node.</div>
        ) : (
          nodeAgents.map((a) => (
            <div
              key={a.public_id}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '7px 10px',
                borderRadius: 5,
                background: 'var(--raised)',
                marginBottom: 2,
                fontSize: 13,
              }}
            >
              <span style={{ color: 'var(--text)', fontWeight: 510 }}>{a.name}</span>
              <span
                style={{
                  fontSize: 11,
                  color: 'var(--ok)',
                  background: 'rgba(87,208,138,0.12)',
                  padding: '1px 5px',
                  borderRadius: 3,
                }}
              >
                {a.status}
              </span>
              <Link
                to="/agents/$agentId"
                params={{ agentId: a.public_id }}
                style={{
                  marginLeft: 'auto',
                  fontSize: 11,
                  color: 'var(--accent)',
                  textDecoration: 'none',
                }}
              >
                View →
              </Link>
            </div>
          ))
        )}
      </div>

      {capacityEntries.length > 0 && (
        <div>
          <div
            style={{
              fontSize: 11,
              fontWeight: 590,
              color: 'var(--dim)',
              letterSpacing: '0.06em',
              textTransform: 'uppercase',
              marginBottom: 8,
            }}
          >
            Capacity
          </div>
          <dl
            style={{
              display: 'grid',
              gridTemplateColumns: 'max-content 1fr',
              gap: '5px 16px',
              fontSize: 13,
            }}
          >
            {capacityEntries.map(([k, v]) => (
              <Fragment key={k}>
                <dt style={{ color: 'var(--dim)' }}>{k}</dt>
                <dd
                  style={{
                    margin: 0,
                    color: 'var(--text)',
                    fontFamily: 'var(--mono)',
                    fontSize: 12,
                  }}
                >
                  {String(v)}
                </dd>
              </Fragment>
            ))}
          </dl>
        </div>
      )}

      <div data-testid="node-danger-zone" style={{ marginTop: 24 }}>
        <div
          style={{
            fontSize: 11,
            fontWeight: 590,
            color: 'var(--dim)',
            letterSpacing: '0.06em',
            textTransform: 'uppercase',
            marginBottom: 8,
          }}
        >
          Danger Zone
        </div>
        <DeleteNodeButton nodeId={node.id} nodeName={node.node_name} />
      </div>
    </div>
  );
}
