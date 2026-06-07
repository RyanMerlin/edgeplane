import { Link, useParams, useRouterState } from '@tanstack/react-router';
import { type Crumb, buildCrumbs } from './breadcrumbs';

export function CrumbTrail({ crumbs }: { crumbs: Crumb[] }) {
  if (crumbs.length === 0) return null;
  return (
    <nav
      className="breadcrumbs"
      data-testid="breadcrumbs"
      aria-label="Breadcrumb"
      style={{
        fontSize: 13,
        fontWeight: 510,
        color: 'var(--text-2)',
        display: 'flex',
        alignItems: 'center',
      }}
    >
      {crumbs.map((c, i) => (
        <span key={c.label} style={{ display: 'flex', alignItems: 'center' }}>
          {i > 0 && (
            <span
              style={{
                color: 'var(--dim)',
                margin: '0 7px',
                fontSize: 13,
              }}
            >
              ›
            </span>
          )}
          {c.to ? (
            <Link
              to={c.to}
              style={{
                color: 'var(--muted)',
                textDecoration: 'none',
                transition: 'color .12s ease',
              }}
              onMouseEnter={(e) => {
                (e.currentTarget as HTMLElement).style.color = 'var(--text-2)';
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLElement).style.color = 'var(--muted)';
              }}
            >
              {c.label}
            </Link>
          ) : (
            <span style={{ color: 'var(--text)' }}>{c.label}</span>
          )}
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
