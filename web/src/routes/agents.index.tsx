import { AgentsTable } from '@/components/fleet/AgentsTable';
import { useMergedAgents } from '@/lib/useMergedAgents';
import { createFileRoute, useNavigate } from '@tanstack/react-router';

export const Route = createFileRoute('/agents/')({
  component: AgentsIndexPage,
});

// Named export for direct test rendering (no router context needed).
export function AgentsIndexPage() {
  const { agents, isLoading, isError, error } = useMergedAgents();
  const navigate = useNavigate();
  return (
    <AgentsTable
      agents={agents}
      isLoading={isLoading}
      isError={isError}
      error={error}
      onRowClick={(a) => navigate({ to: '/agents/$agentId', params: { agentId: a.public_id } })}
    />
  );
}
