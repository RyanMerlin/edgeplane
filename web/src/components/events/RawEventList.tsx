/**
 * RawEventList — raw SSE event stream view (formerly the /matrix route).
 *
 * Now rendered as the "Raw" sub-view of the Feed route (routes/feed.tsx). The
 * event-stream slice is passed in as props so Feed's single useEventStream()
 * subscription drives both its curated view and this raw view — no second
 * EventSource. The old /matrix route redirects to /feed.
 *
 * Features (unchanged from the Matrix page):
 *   - Per-type filter (select from observed types)
 *   - Rate counter (events received in last 60s)
 *   - Connection status indicator + rate-limit display
 *   - Per-event JSON payload expander
 *   - Color-coding by event type
 *   - "Show more" pagination + Clear action
 */

import type { ConnectionStatus, MatrixEvent, RateLimit } from '@/stores/eventStream';
import { useMemo, useState } from 'react';

// ── Constants ─────────────────────────────────────────────────────────────────

const PAGE_SIZE = 50;

// ── Helpers ───────────────────────────────────────────────────────────────────

function eventType(e: MatrixEvent): string {
  return e.type ?? e.event ?? '';
}

function summaryOf(payload: unknown): string {
  if (!payload || typeof payload !== 'object') return String(payload ?? '');
  const p = payload as Record<string, unknown>;
  return (
    String(p.summary ?? p.message ?? p.title ?? p.name ?? '').slice(0, 120) ||
    JSON.stringify(payload).slice(0, 120)
  );
}

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString();
}

/** Type color class — mirrors Svelte typeColor() */
function typeColorClass(t: string): string {
  const v = t.toLowerCase();
  if (v.includes('domain')) return 'type-domain';
  if (v.includes('mission')) return 'type-mission';
  if (v.includes('task')) return 'type-task';
  if (v.includes('agent')) return 'type-agent';
  if (v.includes('error') || v.includes('fail')) return 'type-error';
  return '';
}

/** Tag variant for the event-type badge — mirrors Svelte typeTagClass() */
function typeTagVariant(t: string): 'err' | 'purple' | 'accent' | 'ok' | 'dim' {
  const v = t.toLowerCase();
  if (v.includes('domain')) return 'err';
  if (v.includes('mission')) return 'purple';
  if (v.includes('task')) return 'accent';
  if (v.includes('agent')) return 'ok';
  if (v.includes('error') || v.includes('fail')) return 'err';
  return 'dim';
}

/** Tag variant for status badge — mirrors Svelte statusTagClass() */
function statusTagVariant(s: string | undefined): 'ok' | 'accent' | 'err' | 'dim' {
  const v = String(s ?? '').toLowerCase();
  if (v === 'done' || v === 'completed' || v === 'ok') return 'ok';
  if (v === 'in_progress' || v === 'running') return 'accent';
  if (v === 'blocked' || v === 'failed' || v === 'error') return 'err';
  return 'dim';
}

function Tag({
  variant,
  children,
}: {
  variant: 'ok' | 'err' | 'accent' | 'purple' | 'dim';
  children: React.ReactNode;
}) {
  return <span className={`tag ${variant}`}>{children}</span>;
}

// ── Component ─────────────────────────────────────────────────────────────────

export interface RawEventListProps {
  events: MatrixEvent[];
  status: ConnectionStatus;
  lastError: string | null;
  rateLimit: RateLimit | null;
  clearEvents: () => void;
}

export function RawEventList({
  events,
  status,
  lastError,
  rateLimit,
  clearEvents,
}: RawEventListProps) {
  const [filterType, setFilterType] = useState('');
  const [maxVisible, setMaxVisible] = useState(PAGE_SIZE);

  const isLive = status === 'open';

  const recentRate = useMemo(() => {
    const cutoff = Date.now() - 60_000;
    return events.filter((e) => e.receivedAt > cutoff).length;
  }, [events]);

  // All unique event types observed so far (sorted)
  const knownTypes = useMemo(
    () => [...new Set(events.map((e) => eventType(e)).filter(Boolean))].sort(),
    [events],
  );

  const visibleEvents = useMemo(() => {
    const filtered = filterType ? events.filter((e) => eventType(e) === filterType) : events;
    return filtered.slice(0, maxVisible);
  }, [events, filterType, maxVisible]);

  const totalFiltered = useMemo(() => {
    if (!filterType) return events.length;
    return events.filter((e) => eventType(e) === filterType).length;
  }, [events, filterType]);

  return (
    <div className="matrix-page" data-testid="raw-event-list">
      {/* Filter bar */}
      <div className="matrix-bar" data-testid="matrix-bar">
        <span className={isLive ? 'ok' : 'dim'} data-testid="matrix-status-dot">
          {isLive ? '●' : '○'}
        </span>
        <span className="muted" style={{ fontSize: '11px' }} data-testid="matrix-status-label">
          {isLive ? 'live' : status === 'reconnecting' ? 'reconnecting…' : 'offline'}
        </span>
        <span
          className="dim"
          style={{ fontSize: '11px', marginLeft: '4px' }}
          data-testid="matrix-counts"
        >
          {events.length} events · {recentRate}/min
        </span>
        {rateLimit && (
          <span className="dim" style={{ fontSize: '11px' }} data-testid="rate-limit-display">
            · rl {rateLimit.remaining}/{rateLimit.limit}
          </span>
        )}

        <div style={{ marginLeft: 'auto', display: 'flex', gap: '6px', alignItems: 'center' }}>
          <select
            value={filterType}
            onChange={(e) => {
              setFilterType(e.target.value);
              setMaxVisible(PAGE_SIZE);
            }}
            style={{ fontSize: '11px', padding: '2px 5px' }}
            data-testid="type-filter"
          >
            <option value="">All types</option>
            {knownTypes.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
          <button type="button" className="ghost" onClick={clearEvents} data-testid="clear-btn">
            Clear
          </button>
        </div>
      </div>

      {/* Error banner */}
      {lastError && !isLive && (
        <div
          style={{ padding: '6px 12px', borderBottom: '1px solid var(--border)' }}
          data-testid="matrix-error-banner"
        >
          <p className="error" style={{ margin: 0, fontSize: '11px' }}>
            ✗ {lastError}
          </p>
        </div>
      )}

      {/* Event list */}
      <div className="matrix-list" data-testid="matrix-list">
        {visibleEvents.length === 0 ? (
          <div className="matrix-empty" data-testid="matrix-empty">
            <p className="muted">
              {filterType ? `No "${filterType}" events yet.` : 'Waiting for events…'}
            </p>
          </div>
        ) : (
          <>
            {visibleEvents.map((evt, i) => {
              const label = eventType(evt);
              const summary = summaryOf(evt.payload);
              const tc = typeColorClass(label);
              const tagVariant = typeTagVariant(label);
              const hasPayload: boolean =
                !!evt.payload &&
                typeof evt.payload === 'object' &&
                Object.keys(evt.payload as object).length > 0;

              return (
                <div
                  key={`${evt.receivedAt}-${i}`}
                  className={`event-row${tc ? ` ${tc}` : ''}`}
                  data-testid="matrix-row"
                >
                  <div className="event-time">{fmtTime(evt.receivedAt)}</div>

                  <div className="event-label">
                    <Tag variant={tagVariant}>{label || 'event'}</Tag>
                  </div>

                  <div className="event-meta">
                    {evt.status && <Tag variant={statusTagVariant(evt.status)}>{evt.status}</Tag>}
                    {evt.mission_id && (
                      <span className="dim" style={{ fontSize: '10px' }}>
                        m:{evt.mission_id.slice(0, 8)}
                      </span>
                    )}
                    {evt.agent_id && (
                      <span className="dim" style={{ fontSize: '10px' }}>
                        a:{evt.agent_id.slice(0, 8)}
                      </span>
                    )}
                    {evt.domain_id && (
                      <span className="dim" style={{ fontSize: '10px' }}>
                        d:{evt.domain_id.slice(0, 8)}
                      </span>
                    )}
                  </div>

                  <div className="event-summary-col">
                    {summary && (
                      <span
                        className="muted"
                        style={{
                          fontSize: '11px',
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                      >
                        {summary}
                      </span>
                    )}
                  </div>

                  <div className="event-detail-col">
                    {hasPayload && (
                      <details data-testid="payload-expander">
                        <summary style={{ fontSize: '11px', cursor: 'pointer' }}>payload</summary>
                        <pre style={{ fontSize: '11px', marginTop: '3px' }}>
                          {JSON.stringify(evt.payload, null, 2)}
                        </pre>
                      </details>
                    )}
                  </div>
                </div>
              );
            })}

            {totalFiltered > maxVisible && (
              <div
                style={{
                  padding: '8px',
                  textAlign: 'center',
                  borderTop: '1px solid var(--border)',
                }}
                data-testid="show-more"
              >
                <button
                  type="button"
                  className="ghost"
                  onClick={() => setMaxVisible((n) => n + PAGE_SIZE)}
                >
                  Show more ({totalFiltered - maxVisible} remaining)
                </button>
              </div>
            )}
          </>
        )}
      </div>

      <style>{`
        /* ── Raw event list layout ────────────────────────────────────────── */
        .matrix-page {
          display: flex;
          flex-direction: column;
          height: 100%;
          overflow: hidden;
        }

        /* Toolbar strip — same density as other pane-header bars */
        .matrix-bar {
          height: 32px;
          flex-shrink: 0;
          display: flex;
          align-items: center;
          gap: 8px;
          padding: 0 12px;
          background: var(--surface);
          border-bottom: 1px solid var(--border);
        }

        /* Scrollable list */
        .matrix-list {
          flex: 1;
          overflow-y: auto;
          min-height: 0;
        }
        .matrix-empty {
          display: flex;
          align-items: center;
          justify-content: center;
          height: 100%;
          min-height: 80px;
        }

        /* Event rows */
        .event-row {
          display: grid;
          grid-template-columns: 72px 140px 1fr 2fr auto;
          gap: 8px;
          align-items: center;
          padding: 4px 10px;
          border-bottom: 1px solid var(--border-subtle);
          font-size: 12px;
        }
        .event-row:hover { background: var(--raised); }

        /* Timestamp — mono tabular */
        .event-time {
          font-size: 10px;
          color: var(--dim);
          white-space: nowrap;
          font-family: var(--mono);
          font-variant-numeric: tabular-nums;
        }
        .event-label { display: flex; align-items: center; gap: 4px; }
        .event-meta  { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
        .event-summary-col { overflow: hidden; display: flex; align-items: center; }
        .event-detail-col  { display: flex; align-items: center; }

        /* Error-type row accent — token-based, no hardcoded color */
        .event-row.type-error { background: var(--err-bg); }
      `}</style>
    </div>
  );
}
