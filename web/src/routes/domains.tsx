import { Outlet, createFileRoute } from '@tanstack/react-router';

// Domains section layout — list at domains.index.tsx, entity pages at domains.$domainId.tsx
export const Route = createFileRoute('/domains')({
  component: () => <Outlet />,
});
