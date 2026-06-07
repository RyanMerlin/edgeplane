/**
 * Dashboard — landing page (/).
 *
 * Pivot between the two nav spines:
 *   - Fleet card: online/total agent summary → links to /agents
 *   - Work card: domain + mission count → links to /domains
 *   - Recent activity: last ~8 SSE events (read-only)
 *
 * Data sources:
 *   - useMergedAgents() — fleet agent list with status
 *   - GET /api/explorer/tree — domain + mission counts
 *   - useEventStream() — live event ring buffer
 */

import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useEventStream } from '@/lib/useEventStream';
import { useMergedAgents } from '@/lib/useMergedAgents';
import { useQuery } from '@tanstack/react-query';
import { Link, createFileRoute } from '@tanstack/react-router';

// ── Route ──────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/')({
  component: Dashboard,
});

// ── Types ──────────────────────────────────────────────────────────────────────

type ExplorerTreeResponse = components['schemas']['ExplorerTreeResponse'];

// ── Helpers ────────────────────────────────────────────────────────────────────

function fmtRelative(ts: number): string {
  const diffMs = Date.now() - ts;
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 60) return `${diffSec}s ago`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  return new Date(ts).toLocaleString();
}

// ── Card shell ─────────────────────────────────────────────────────────────────

function Card({
  heading,
  children,
}: {
  heading: string;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        background: 'var(--surface)',
        border: '1px solid var(--border)',
        borderRadius: '6px',
        padding: '14px 16px',
        display: 'flex',
        flexDirection: 'column',
        gap: '8px',
        minWidth: 0,
      }}
    >
      <div
        style={{
          fontSize: '11px',
          fontWeight: 700,
          color: 'var(--muted)',
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
        }}
      >
        {heading}
      </div>
      {children}
    </div>
  );
}

// ── Fleet card ─────────────────────────────────────────────────────────────────

function FleetCard() {
  const { agents, isLoading } = useMergedAgents();

  const onlineCount = agents.filter((a) => a.status === 'online' || a.status === 'active').length;
  const totalCount = agents.length;

  return (
    <Card heading="Fleet">
      <div style={{ fontSize: '24px', fontWeight: 700, fontFamily: 'monospace', lineHeight: 1.2 }}>
        {isLoading ? (
          <span className="muted" style={{ fontSize: '13px' }}>
            Loading…
          </span>
        ) : (
          <>
            <span data-testid="dash-fleet-online" style={{ color: 'var(--ok)' }}>
              {onlineCount}
            </span>
            <span style={{ color: 'var(--muted)', fontSize: '14px', fontWeight: 400 }}>
              {' / '}
              {totalCount} online
            </span>
          </>
        )}
      </div>
      <div style={{ marginTop: '4px' }}>
        <Link
          to="/agents"
          style={{
            fontSize: '12px',
            color: 'var(--accent)',
            textDecoration: 'none',
          }}
        >
          Agents →
        </Link>
      </div>
    </Card>
  );
}

// ── Work card ──────────────────────────────────────────────────────────────────

function WorkCard() {
  const treeQuery = useQuery<ExplorerTreeResponse>({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => unwrap(apiClient.GET('/api/explorer/tree')),
    refetchInterval: 60_000,
  });

  const domains = treeQuery.data?.domains ?? [];
  const domainCount = domains.length;
  const missionCount = domains.reduce((sum, d) => sum + d.missions.length, 0);

  return (
    <Card heading="Work">
      <div style={{ fontSize: '24px', fontWeight: 700, fontFamily: 'monospace', lineHeight: 1.2 }}>
        {treeQuery.isLoading ? (
          <span className="muted" style={{ fontSize: '13px' }}>
            Loading…
          </span>
        ) : (
          <>
            <span data-testid="dash-work-domains" style={{ color: 'var(--text)' }}>
              {domainCount}
            </span>
            <span style={{ color: 'var(--muted)', fontSize: '14px', fontWeight: 400 }}>
              {' '}
              {domainCount === 1 ? 'domain' : 'domains'}
              {missionCount > 0 && (
                <>
                  {', '}
                  {missionCount} {missionCount === 1 ? 'mission' : 'missions'}
                </>
              )}
            </span>
          </>
        )}
      </div>
      <div style={{ marginTop: '4px' }}>
        <Link
          to="/domains"
          style={{
            fontSize: '12px',
            color: 'var(--accent)',
            textDecoration: 'none',
          }}
        >
          Domains →
        </Link>
      </div>
    </Card>
  );
}

// ── Recent activity ────────────────────────────────────────────────────────────

function ActivityStrip() {
  const { events } = useEventStream();
  const recent = events.slice(0, 8);

  return (
    <Card heading="Recent activity">
      {recent.length === 0 ? (
        <p className="muted" style={{ fontSize: '12px', margin: 0 }} data-testid="activity-empty">
          No recent events.
        </p>
      ) : (
        <ul
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: '4px',
          }}
          data-testid="activity-list"
        >
          {recent.map((ev, i) => (
            <li
              key={ev.id ?? i}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                fontSize: '12px',
              }}
            >
              <span
                className="tag"
                style={{ fontFamily: 'monospace', fontSize: '11px', flexShrink: 0 }}
              >
                {ev.type ?? ev.event ?? 'event'}
              </span>
              <span className="dim" style={{ fontSize: '11px', marginLeft: 'auto', flexShrink: 0 }}>
                {fmtRelative(ev.receivedAt)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

// ── Main component ─────────────────────────────────────────────────────────────

export function Dashboard() {
  return (
    <div
      data-testid="dashboard"
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: '16px',
        padding: '20px',
        height: '100%',
        overflowY: 'auto',
        boxSizing: 'border-box',
      }}
    >
      {/* Top row: Fleet + Work side by side */}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
          gap: '16px',
        }}
      >
        <FleetCard />
        <WorkCard />
      </div>

      {/* Recent activity */}
      <ActivityStrip />
    </div>
  );
}
