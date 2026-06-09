import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { NodesTable } from '@/components/nodes/NodesTable';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, useNavigate } from '@tanstack/react-router';

type RuntimeNode = components['schemas']['RuntimeNode'];

export const Route = createFileRoute('/nodes/')({
  component: NodesIndexPage,
});

export function NodesIndexPage() {
  const navigate = useNavigate();

  const { data, isLoading, isError } = useQuery({
    queryKey: queryKeys.nodes.list(),
    queryFn: () => unwrap(apiClient.GET('/api/runtime/nodes', {})),
    refetchInterval: 30_000,
  });

  const nodes: RuntimeNode[] = Array.isArray(data) ? data : [];

  if (isError) {
    return (
      <div data-testid="error-state" style={{ padding: 24, color: 'var(--err)', fontSize: 13 }}>
        Failed to load nodes.
      </div>
    );
  }

  return (
    <div style={{ padding: '16px 24px' }}>
      <div style={{ marginBottom: 12 }}>
        <span
          style={{
            fontSize: 11,
            fontWeight: 590,
            color: 'var(--dim)',
            letterSpacing: '0.06em',
            textTransform: 'uppercase',
          }}
        >
          Fleet Nodes
        </span>
      </div>
      <NodesTable
        nodes={nodes}
        isLoading={isLoading}
        onRowClick={(node) => navigate({ to: '/nodes/$nodeId', params: { nodeId: node.id } })}
      />
    </div>
  );
}
