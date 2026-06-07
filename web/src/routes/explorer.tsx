import { createFileRoute, redirect } from '@tanstack/react-router';

export const Route = createFileRoute('/explorer')({
  beforeLoad: () => {
    throw redirect({ to: '/domains' });
  },
});
