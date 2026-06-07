import { Outlet, createFileRoute } from '@tanstack/react-router';

// Layout for the Agents section. The list lives in agents.index.tsx; the detail
// in agents.$agentId.tsx renders through this Outlet.
export const Route = createFileRoute('/agents')({
  component: () => <Outlet />,
});
