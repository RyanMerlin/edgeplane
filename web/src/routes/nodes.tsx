import { Outlet, createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/nodes')({
  component: () => <Outlet />,
});
