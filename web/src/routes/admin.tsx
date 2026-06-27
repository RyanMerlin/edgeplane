import { Outlet, createFileRoute, redirect } from '@tanstack/react-router';
import { useAuthStore } from '@/stores/auth';

export const Route = createFileRoute('/admin')({
  beforeLoad: () => {
    if (!useAuthStore.getState().isAdmin) throw redirect({ to: '/' });
  },
  component: () => <Outlet />,
});
