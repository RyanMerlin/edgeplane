/**
 * /agents — consolidated into the Fleet dashboard (/).
 *
 * The agents table is now the "Agents" sub-view of the Fleet route
 * (routes/index.tsx, via components/fleet/AgentsTable.tsx), so the bare
 * /agents path redirects there. This route is retained only as the layout
 * parent for the detail route /agents/$agentId, which still renders through
 * the <Outlet/> below.
 */

import { Outlet, createFileRoute, redirect } from '@tanstack/react-router';

export const Route = createFileRoute('/agents')({
  beforeLoad: ({ location }) => {
    // Redirect only the bare /agents path; let /agents/$agentId fall through
    // to the Outlet so the detail screen still renders.
    if (location.pathname.replace(/\/+$/, '') === '/agents') {
      throw redirect({ to: '/' });
    }
  },
  component: () => <Outlet />,
});
