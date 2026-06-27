import { useAuthStore } from '@/stores/auth';
import { Outlet, createFileRoute, redirect } from '@tanstack/react-router';

export const Route = createFileRoute('/admin')({
  beforeLoad: () => {
    if (!useAuthStore.getState().isAdmin) throw redirect({ to: '/' });
  },
  component: () => <Outlet />,
});
