import { Link, useParams, useRouterState } from '@tanstack/react-router';
import { type Crumb, buildCrumbs } from './breadcrumbs';

export function CrumbTrail({ crumbs }: { crumbs: Crumb[] }) {
  if (crumbs.length === 0) return null;
  return (
    <nav className="breadcrumbs" data-testid="breadcrumbs" aria-label="Breadcrumb">
      {crumbs.map((c, i) => (
        <span key={c.label}>
          {i > 0 && (
            <span className="dim" style={{ margin: '0 6px' }}>
              ›
            </span>
          )}
          {c.to ? <Link to={c.to}>{c.label}</Link> : <span>{c.label}</span>}
        </span>
      ))}
    </nav>
  );
}

export default function Breadcrumbs() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const params = useParams({ strict: false }) as Record<string, string>;
  return <CrumbTrail crumbs={buildCrumbs(pathname, params)} />;
}
