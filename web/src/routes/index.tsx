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
  if (diffSec < 60) return `${diffSec}s`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h`;
  return new Date(ts).toLocaleString();
}

/** Map event type string to a status dot color */
function eventDotColor(type: string | undefined): string {
  if (!type) return 'var(--dim)';
  const t = type.toLowerCase();
  if (t.includes('finish') || t.includes('complet') || t.includes('heartbeat'))
    return 'var(--warn)';
  if (t.includes('claim') || t.includes('creat') || t.includes('start')) return 'var(--accent)';
  if (t.includes('ok') || t.includes('success') || t.includes('done')) return 'var(--ok)';
  if (t.includes('fail') || t.includes('err') || t.includes('reject')) return 'var(--err)';
  if (t.includes('govern') || t.includes('policy')) return 'var(--purple)';
  return 'var(--dim)';
}

// ── Fleet card ─────────────────────────────────────────────────────────────────

function FleetCard() {
  const { agents, isLoading } = useMergedAgents();

  const onlineCount = agents.filter((a) => a.status === 'online' || a.status === 'active').length;
  const totalCount = agents.length;

  return (
    <div
      style={{
        background: 'var(--card)',
        border: '1px solid var(--border-subtle)',
        borderRadius: '10px',
        padding: '16px',
      }}
    >
      <h3
        style={{
          margin: '0 0 10px',
          fontSize: '11px',
          fontWeight: 590,
          color: 'var(--dim)',
          letterSpacing: '0.02em',
          textTransform: 'uppercase',
        }}
      >
        Fleet
      </h3>
      <div style={{ fontSize: '28px', fontWeight: 590, letterSpacing: '-0.02em', lineHeight: 1.2 }}>
        {isLoading ? (
          <span style={{ fontSize: '13px', color: 'var(--muted)' }}>Loading…</span>
        ) : (
          <>
            <span data-testid="dash-fleet-online" style={{ color: 'var(--ok)' }}>
              {onlineCount}
            </span>
            <span
              style={{
                fontSize: '14px',
                fontWeight: 400,
                color: 'var(--muted)',
                marginLeft: '4px',
              }}
            >
              / {totalCount} online
            </span>
          </>
        )}
      </div>
      <div style={{ marginTop: '10px' }}>
        <Link
          to="/agents"
          style={{ fontSize: '12.5px', color: 'var(--accent)', display: 'inline-block' }}
        >
          Agents →
        </Link>
      </div>
    </div>
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
    <div
      style={{
        background: 'var(--card)',
        border: '1px solid var(--border-subtle)',
        borderRadius: '10px',
        padding: '16px',
      }}
    >
      <h3
        style={{
          margin: '0 0 10px',
          fontSize: '11px',
          fontWeight: 590,
          color: 'var(--dim)',
          letterSpacing: '0.02em',
          textTransform: 'uppercase',
        }}
      >
        Work
      </h3>
      <div style={{ fontSize: '28px', fontWeight: 590, letterSpacing: '-0.02em', lineHeight: 1.2 }}>
        {treeQuery.isLoading ? (
          <span style={{ fontSize: '13px', color: 'var(--muted)' }}>Loading…</span>
        ) : (
          <>
            <span data-testid="dash-work-domains">{domainCount}</span>
            <span
              style={{
                fontSize: '14px',
                fontWeight: 400,
                color: 'var(--muted)',
                marginLeft: '4px',
              }}
            >
              {domainCount === 1 ? 'domain' : 'domains'}
              {missionCount > 0 &&
                ` · ${missionCount} ${missionCount === 1 ? 'mission' : 'missions'}`}
            </span>
          </>
        )}
      </div>
      <div style={{ marginTop: '10px' }}>
        <Link
          to="/domains"
          style={{ fontSize: '12.5px', color: 'var(--accent)', display: 'inline-block' }}
        >
          Domains →
        </Link>
      </div>
    </div>
  );
}

// ── Recent activity ────────────────────────────────────────────────────────────

function ActivityStrip() {
  const { events } = useEventStream();
  const recent = events.slice(0, 8);

  return (
    <>
      <div
        style={{
          fontSize: '11px',
          fontWeight: 590,
          color: 'var(--dim)',
          letterSpacing: '0.02em',
          textTransform: 'uppercase',
          margin: '22px 0 8px',
        }}
      >
        Recent Activity
      </div>
      <div
        style={{
          background: 'var(--card)',
          border: '1px solid var(--border-subtle)',
          borderRadius: '10px',
          padding: '6px 16px',
        }}
      >
        {recent.length === 0 ? (
          <p
            className="muted"
            style={{ fontSize: '12px', margin: '8px 0' }}
            data-testid="activity-empty"
          >
            No recent events.
          </p>
        ) : (
          <div data-testid="activity-list">
            {recent.map((ev, i) => {
              const evType = ev.type ?? ev.event ?? 'event';
              const dotColor = eventDotColor(evType);
              return (
                <div
                  key={ev.id ?? i}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: '10px',
                    padding: '7px 0',
                    borderBottom: i < recent.length - 1 ? '1px solid var(--border-subtle)' : 'none',
                    fontSize: '13px',
                  }}
                >
                  <span className="tag">
                    <span
                      className="dot"
                      style={{ background: dotColor, width: '5px', height: '5px' }}
                    />
                    {evType}
                  </span>
                  <span
                    style={{
                      color: 'var(--muted)',
                      fontSize: '13px',
                      minWidth: 0,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {ev.payload && typeof ev.payload === 'object'
                      ? JSON.stringify(ev.payload).slice(0, 80)
                      : ''}
                  </span>
                  <span
                    style={{
                      marginLeft: 'auto',
                      color: 'var(--dim)',
                      fontSize: '11px',
                      fontFamily: 'var(--mono)',
                      flexShrink: 0,
                    }}
                  >
                    {fmtRelative(ev.receivedAt)}
                  </span>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </>
  );
}

// ── Main component ─────────────────────────────────────────────────────────────

export function Dashboard() {
  return (
    <div
      data-testid="dashboard"
      style={{
        padding: '20px 18px',
        height: '100%',
        overflowY: 'auto',
        boxSizing: 'border-box',
      }}
    >
      {/* Summary cards */}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(260px, 1fr))',
          gap: '14px',
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
