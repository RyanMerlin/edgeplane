/**
 * Feed screen — Phase 3 SSE migration.
 *
 * Data source:  SSE /api/events/stream via useEventStream() → Zustand store
 *
 * Parity with web/src/routes/feed/+page.svelte:
 *   - Text filter across agent_id, mission_id, event type, and payload summary
 *   - Chip filter: All | Errors | Governance | Artifacts | Tasks | Heartbeat
 *   - Alerts-only toggle (errors + governance + overlaps)
 *   - Rate counter (events received in last 60s)
 *   - Connection status indicator (LIVE / offline)
 *   - Detail panel on row click (payload, agent, domain, mission, status)
 *   - Status bar: total count, error/governance/warn counters
 *
 * NOTE: EventSource /api/events/stream shape is INFERRED from telemetry.ts.
 */

import { useEventStream } from '@/lib/useEventStream';
import type { MatrixEvent } from '@/stores/eventStream';
import { createFileRoute } from '@tanstack/react-router';
import { useMemo, useState } from 'react';

// ── Route ─────────────────────────────────────────────────────────────────────

export const Route = createFileRoute('/feed')({
  component: FeedPage,
});

// ── Types ─────────────────────────────────────────────────────────────────────

type ChipFilter = 'all' | 'errors' | 'governance' | 'artifacts' | 'tasks' | 'heartbeat';

// ── Helpers ───────────────────────────────────────────────────────────────────

function eventType(e: MatrixEvent): string {
  return e.type ?? e.event ?? '';
}

function alertClass(e: MatrixEvent): '' | 'a-err' | 'a-warn' | 'a-gov' {
  const t = eventType(e);
  if (t === 'step_error') return 'a-err';
  if (t === 'overlap_detected') return 'a-warn';
  if (t === 'governance') return 'a-gov';
  return '';
}

function typeClass(e: MatrixEvent): string {
  const t = eventType(e);
  if (t === 'step_started') return 'ty-start';
  if (t === 'step_finished') return 'ty-finish';
  if (t === 'step_error') return 'ty-err';
  if (t === 'governance') return 'ty-gov';
  if (t === 'artifact') return 'ty-art';
  if (t === 'heartbeat') return 'ty-hb';
  if (t === 'task_claimed') return 'ty-claim';
  if (t === 'task_finished') return 'ty-done';
  if (t === 'overlap_detected') return 'ty-warn';
  return '';
}

function summaryOf(payload: unknown): string {
  if (!payload || typeof payload !== 'object') return String(payload ?? '');
  const p = payload as Record<string, unknown>;
  return (
    String(p.summary ?? p.message ?? p.title ?? p.name ?? '').slice(0, 140) ||
    JSON.stringify(payload).slice(0, 140)
  );
}

function agentOf(e: MatrixEvent): string {
  if (e.agent_id) return e.agent_id;
  if (e.payload && typeof e.payload === 'object') {
    const p = e.payload as Record<string, unknown>;
    return String(p.agent ?? p.agent_id ?? '');
  }
  return '';
}

function contextOf(e: MatrixEvent): string {
  if (e.mission_id) return e.mission_id;
  if (e.payload && typeof e.payload === 'object') {
    const p = e.payload as Record<string, unknown>;
    return String(p.context ?? p.task_id ?? '');
  }
  return '';
}

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString('en-US', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

function fmtTimeFull(ts: number): string {
  const d = new Date(ts);
  return `${d.toLocaleTimeString('en-US', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })}.${String(d.getMilliseconds()).padStart(3, '0')}`;
}

function chipMatchesEvent(chip: ChipFilter, e: MatrixEvent): boolean {
  if (chip === 'all') return true;
  const t = eventType(e);
  if (chip === 'errors') return t === 'step_error' || t === 'overlap_detected';
  if (chip === 'governance') return t === 'governance';
  if (chip === 'artifacts') return t === 'artifact';
  if (chip === 'tasks') return t === 'task_claimed' || t === 'task_finished';
  if (chip === 'heartbeat') return t === 'heartbeat';
  return true;
}

// ── Sub-components ────────────────────────────────────────────────────────────

function ChipButton({
  label,
  active,
  variant,
  onClick,
  'data-testid': testId,
}: {
  label: string;
  active: boolean;
  variant?: 'err' | 'gov';
  onClick: () => void;
  'data-testid'?: string;
}) {
  let className = 'chip';
  if (active) {
    if (variant === 'err') className += ' on-err';
    else if (variant === 'gov') className += ' on-gov';
    else className += ' on';
  }
  return (
    <button type="button" className={className} onClick={onClick} data-testid={testId}>
      {label}
    </button>
  );
}

function DetailPanel({ event }: { event: MatrixEvent | null }) {
  if (!event) {
    return (
      <div id="detail-panel" data-testid="detail-panel">
        <div className="dp-empty">
          <span className="dim">Select a row to inspect</span>
        </div>
      </div>
    );
  }

  const ac = alertClass(event);
  const label = eventType(event);
  const agent = agentOf(event);
  const ctx = contextOf(event);
  const payload = event.payload;

  const alertIcon =
    ac === 'a-err' ? (
      <span className="err">⚠</span>
    ) : ac === 'a-gov' ? (
      <span className="purple">⬡</span>
    ) : ac === 'a-warn' ? (
      <span className="warn">⚠</span>
    ) : (
      <span className="dim">○</span>
    );

  return (
    <div id="detail-panel" data-testid="detail-panel">
      <div className="dp-hdr">
        {alertIcon}
        <span className="t">{label || 'event'}</span>
        <span className="dim">·</span>
        <span>{fmtTime(event.receivedAt)}</span>
      </div>
      <div className="dp-body">
        <div className="kv">
          <span className="kk">Time</span>
          <span className="dim">{fmtTimeFull(event.receivedAt)}</span>
        </div>
        {agent && (
          <div className="kv">
            <span className="kk">Agent</span>
            <span>{agent}</span>
          </div>
        )}
        {event.domain_id && (
          <div className="kv">
            <span className="kk">Domain</span>
            <span className="muted">{event.domain_id}</span>
          </div>
        )}
        {event.mission_id && (
          <div className="kv">
            <span className="kk">Mission</span>
            <span className="muted">{event.mission_id}</span>
          </div>
        )}
        {ctx && ctx !== event.mission_id && (
          <div className="kv">
            <span className="kk">Context</span>
            <span className="muted">{ctx}</span>
          </div>
        )}

        {!!payload && typeof payload === 'object' && Object.keys(payload as object).length > 0 && (
          <>
            <div className="d-sep" />
            <div className="d-sec">Payload</div>
            <div className="payload-block">
              {Object.entries(payload as Record<string, unknown>).map(([k, v]) => (
                <div key={k}>
                  <span className="pk">{k}</span>{' '}
                  <span
                    className={`pv${k === 'error' || (k === 'message' && ac === 'a-err') ? ' pv-err' : ''}`}
                  >
                    {typeof v === 'string' ? v : JSON.stringify(v)}
                  </span>
                </div>
              ))}
            </div>
          </>
        )}

        {event.status && (
          <>
            <div className="d-sep" />
            <div className="d-sec">Status</div>
            <div className="kv">
              <span className="kk">Status</span>
              <span>{event.status}</span>
            </div>
          </>
        )}

        {event.agent_id && (
          <>
            <div className="d-sep" />
            <div className="d-sec">Agent Context</div>
            <div className="ctx-log">
              <div>
                <span className="muted">{fmtTime(event.receivedAt)}</span> {label} —{' '}
                {summaryOf(payload)}
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function FeedPage() {
  const { events, status, lastError, rateLimit } = useEventStream();

  const [filterText, setFilterText] = useState('');
  const [activeChip, setActiveChip] = useState<ChipFilter>('all');
  const [alertsOnly, setAlertsOnly] = useState(false);
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);

  const isLive = status === 'open';

  // Rate: events received in last 60s (matches Svelte recentRate derived store)
  const recentRate = useMemo(() => {
    const cutoff = Date.now() - 60_000;
    return events.filter((e) => e.receivedAt > cutoff).length;
  }, [events]);

  const filteredEvents = useMemo(() => {
    let list = events;

    if (activeChip !== 'all') {
      list = list.filter((e) => chipMatchesEvent(activeChip, e));
    }

    if (alertsOnly) {
      list = list.filter((e) => alertClass(e) !== '');
    }

    if (filterText.trim()) {
      const q = filterText.toLowerCase();
      list = list.filter(
        (e) =>
          (e.agent_id ?? '').toLowerCase().includes(q) ||
          (e.mission_id ?? '').toLowerCase().includes(q) ||
          eventType(e).toLowerCase().includes(q) ||
          summaryOf(e.payload).toLowerCase().includes(q),
      );
    }

    return list;
  }, [events, activeChip, alertsOnly, filterText]);

  const errorCount = useMemo(
    () => events.filter((e) => alertClass(e) === 'a-err').length,
    [events],
  );
  const govCount = useMemo(() => events.filter((e) => alertClass(e) === 'a-gov').length, [events]);
  const warnCount = useMemo(
    () => events.filter((e) => alertClass(e) === 'a-warn').length,
    [events],
  );

  const selectedEvent = selectedIdx !== null ? (filteredEvents[selectedIdx] ?? null) : null;

  function handleRowClick(i: number) {
    setSelectedIdx((prev) => (prev === i ? null : i));
  }

  function handleFilterChange(text: string) {
    setFilterText(text);
    setSelectedIdx(null);
  }

  function handleChipClick(chip: ChipFilter) {
    setActiveChip((prev) => {
      const next = prev === chip && chip !== 'all' ? 'all' : chip;
      setSelectedIdx(null);
      return next;
    });
  }

  return (
    <div
      className="feed-page"
      style={{ display: 'flex', flexDirection: 'column', height: '100%', overflow: 'hidden' }}
    >
      {/* Filter bar */}
      <div id="filterbar" data-testid="filterbar">
        <div className={`fi${filterText.length > 0 ? ' focused' : ''}`}>
          <span className="dim">/</span>
          <input
            type="text"
            placeholder="filter events..."
            value={filterText}
            onChange={(e) => handleFilterChange(e.target.value)}
            data-testid="filter-input"
          />
        </div>

        <span className="fsep">|</span>

        <ChipButton
          label="All"
          active={activeChip === 'all'}
          onClick={() => handleChipClick('all')}
          data-testid="chip-all"
        />
        <ChipButton
          label="Errors"
          active={activeChip === 'errors'}
          variant="err"
          onClick={() => handleChipClick('errors')}
          data-testid="chip-errors"
        />
        <ChipButton
          label="Governance"
          active={activeChip === 'governance'}
          variant="gov"
          onClick={() => handleChipClick('governance')}
          data-testid="chip-governance"
        />
        <ChipButton
          label="Artifacts"
          active={activeChip === 'artifacts'}
          onClick={() => handleChipClick('artifacts')}
          data-testid="chip-artifacts"
        />
        <ChipButton
          label="Tasks"
          active={activeChip === 'tasks'}
          onClick={() => handleChipClick('tasks')}
          data-testid="chip-tasks"
        />
        <ChipButton
          label="Heartbeat"
          active={activeChip === 'heartbeat'}
          onClick={() => handleChipClick('heartbeat')}
          data-testid="chip-heartbeat"
        />

        <span className="fsep">|</span>

        <button
          type="button"
          className={`fi alerts-toggle${alertsOnly ? ' active-warn' : ''}`}
          onClick={() => setAlertsOnly((v) => !v)}
          data-testid="alerts-toggle"
        >
          <span className={`alert-dot${alertsOnly ? ' on' : ''}`} />
          <span>Alerts only</span>
        </button>

        <div className="fr">
          <span data-testid="event-count">{events.length} events</span>
          <span className="dim">·</span>
          <span className="ok" style={{ fontSize: '11px' }} data-testid="rate-counter">
            rate {recentRate}/60
          </span>
          {rateLimit && (
            <>
              <span className="dim">·</span>
              <span className="dim" style={{ fontSize: '11px' }}>
                rl {rateLimit.remaining}/{rateLimit.limit}
              </span>
            </>
          )}
          {isLive ? (
            <>
              <span className="ok live" data-testid="status-live">
                ●
              </span>
              <span className="ok" style={{ fontWeight: 700, fontSize: '11px' }}>
                LIVE
              </span>
            </>
          ) : (
            <span className="dim" data-testid="status-offline">
              ○ {status === 'reconnecting' ? 'reconnecting…' : 'offline'}
            </span>
          )}
        </div>
      </div>

      {/* Error banner */}
      {lastError && !isLive && (
        <div
          style={{ padding: '4px 12px', borderBottom: '1px solid var(--border)' }}
          data-testid="error-banner"
        >
          <span className="error" style={{ fontSize: '11px' }}>
            ✗ {lastError}
          </span>
        </div>
      )}

      {/* Content */}
      <div id="content" data-testid="content-area">
        {/* Feed list */}
        <div id="feed-list">
          <div id="feed-hdr">
            <span>Time</span>
            <span>Agent</span>
            <span>Context</span>
            <span>Event</span>
            <span>Detail</span>
          </div>

          <div id="feed" data-testid="feed-list">
            {filteredEvents.length === 0 ? (
              <div className="feed-empty" data-testid="feed-empty">
                <span className="dim">
                  {filterText || activeChip !== 'all'
                    ? 'No matching events.'
                    : 'Waiting for events…'}
                </span>
              </div>
            ) : (
              filteredEvents.map((evt, i) => {
                const ac = alertClass(evt);
                const tc = typeClass(evt);
                const agent = agentOf(evt);
                const ctx = contextOf(evt);
                const summary = summaryOf(evt.payload);
                const label = eventType(evt);
                const isSelected = selectedIdx === i;

                return (
                  <button
                    type="button"
                    key={`${evt.receivedAt}-${i}`}
                    className={`f-row ${ac}${isSelected ? ' sel' : ''}`}
                    onClick={() => handleRowClick(i)}
                    data-testid="feed-row"
                  >
                    <span
                      className={`f-time${ac === 'a-err' ? ' err' : ac === 'a-warn' ? ' warn' : ''}`}
                    >
                      {fmtTime(evt.receivedAt)}
                    </span>
                    <span
                      className={`f-agent${ac === 'a-err' ? ' err' : ac === 'a-warn' ? ' warn' : ''}`}
                    >
                      {agent || '—'}
                    </span>
                    <span
                      className={`f-ctx${ac === 'a-err' ? ' err' : ac === 'a-warn' ? ' warn' : ''}`}
                    >
                      {ctx || '—'}
                    </span>
                    <span className={`f-type ${tc}`}>{label || 'event'}</span>
                    <span
                      className={[
                        'f-msg',
                        ac === 'a-err' || ac === 'a-warn' ? (ac === 'a-err' ? 'err' : 'warn') : '',
                        label === 'task_finished' ? 'ok' : '',
                        label === 'artifact' ? 'purple-txt' : '',
                      ]
                        .filter(Boolean)
                        .join(' ')}
                    >
                      {summary}
                    </span>
                  </button>
                );
              })
            )}
          </div>
        </div>

        {/* Detail panel */}
        <DetailPanel event={selectedEvent} />
      </div>

      {/* Status bar */}
      <div className="feed-statusbar" data-testid="feed-statusbar">
        <span className="muted">{events.length} events</span>
        <span className="dim">·</span>
        {errorCount > 0 ? (
          <span className="err" data-testid="error-count">
            {errorCount} error{errorCount === 1 ? '' : 's'}
          </span>
        ) : (
          <span className="dim">0 errors</span>
        )}
        {govCount > 0 && (
          <span className="warn" data-testid="gov-count">
            {govCount} governance
          </span>
        )}
        {warnCount > 0 && (
          <span className="warn" data-testid="warn-count">
            {warnCount} overlap
          </span>
        )}
        <div className="feed-statusbar-right">
          <span>/ filter</span>
          <span className="dim">·</span>
          <span>click row to inspect</span>
          {isLive && (
            <>
              <span className="dim">·</span>
              <span className="ok live">●</span>
              <span className="ok">LIVE</span>
            </>
          )}
        </div>
      </div>

      <style>{`
        /* Feed page layout */
        .feed-page {
          display: flex;
          flex-direction: column;
          height: 100%;
          overflow: hidden;
        }

        #filterbar {
          height: 34px;
          flex-shrink: 0;
          display: flex;
          align-items: center;
          flex-wrap: nowrap;
          background: var(--surface);
          border-bottom: 1px solid var(--border);
          padding: 0 12px;
          gap: 6px;
        }
        .fi {
          display: flex;
          align-items: center;
          gap: 5px;
          background: var(--base);
          border: 1px solid var(--border-2);
          border-radius: 3px;
          padding: 2px 8px;
          font-size: 11px;
          color: var(--muted);
        }
        .fi.focused { border-color: var(--accent); }
        .fi input {
          background: transparent;
          border: none;
          outline: none;
          color: var(--text);
          font-family: inherit;
          font-size: 11px;
          width: 20ch;
          padding: 0;
        }
        .fi input::placeholder { color: var(--dim); }
        .fsep { color: var(--border); font-size: 14px; margin: 0 2px; }
        .chip.on-err { border-color: var(--err-border); color: var(--err); background: var(--err-bg); }
        .chip.on-gov { border-color: var(--purple-border); color: var(--purple); background: var(--purple-bg); }
        .alerts-toggle {
          background: var(--base);
          border: 1px solid var(--border-2);
          border-radius: 3px;
          padding: 2px 8px;
          font-size: 11px;
          color: var(--muted);
          display: flex;
          align-items: center;
          gap: 5px;
          cursor: pointer;
        }
        .alerts-toggle.active-warn { border-color: var(--warn-border); color: var(--warn); background: var(--warn-bg); }
        .alert-dot {
          width: 7px;
          height: 7px;
          border-radius: 50%;
          background: var(--dim);
          display: inline-block;
          flex-shrink: 0;
        }
        .alert-dot.on { background: var(--warn); }
        .fr {
          margin-left: auto;
          display: flex;
          align-items: center;
          gap: 10px;
          font-size: 11px;
          color: var(--dim);
          white-space: nowrap;
        }

        /* Content area */
        #content {
          flex: 1;
          display: flex;
          overflow: hidden;
          min-height: 0;
        }

        /* Feed list */
        #feed-list {
          flex: 1;
          display: flex;
          flex-direction: column;
          overflow: hidden;
          min-width: 0;
        }
        #feed-hdr {
          display: grid;
          grid-template-columns: 68px 140px 160px 120px 1fr;
          gap: 0 6px;
          padding: 3px 12px;
          background: var(--surface);
          border-bottom: 1px solid var(--border-2);
          color: var(--dim);
          font-size: 10px;
          text-transform: uppercase;
          letter-spacing: 0.05em;
          flex-shrink: 0;
        }
        #feed {
          flex: 1;
          overflow-y: auto;
        }
        .feed-empty { padding: 24px 12px; font-size: 12px; }
        .f-row {
          display: grid;
          grid-template-columns: 68px 140px 160px 120px 1fr;
          gap: 0 6px;
          padding: 3px 12px;
          border-bottom: 1px solid var(--border);
          align-items: baseline;
          cursor: pointer;
        }
        .f-row:hover { background: var(--surface); }
        .f-row.sel { background: var(--surface-2); }
        .f-row.a-err { border-left: 2px solid var(--err); background: #120a0a; }
        .f-row.a-warn { border-left: 2px solid var(--warn); background: #110f00; }
        .f-row.a-gov { border-left: 2px solid var(--purple); background: #0a0f1a; }
        .f-row.sel.a-err  { background: #1f1010; }
        .f-row.sel.a-warn { background: #1f1a0a; }
        .f-row.sel.a-gov  { background: #141c30; }
        .f-time  { color: var(--dim); font-size: 11px; }
        .f-agent { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .f-ctx   { color: var(--dim); font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .f-type  { font-size: 11px; font-weight: 600; white-space: nowrap; }
        .f-msg   { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .f-msg.purple-txt { color: var(--purple); }
        .ty-start  { color: var(--accent); }
        .ty-finish { color: var(--ok); }
        .ty-err    { color: var(--err); font-weight: 700; }
        .ty-gov    { color: var(--purple); font-weight: 700; }
        .ty-art    { color: var(--purple); }
        .ty-hb     { color: var(--border-2); }
        .ty-claim  { color: var(--accent); }
        .ty-done   { color: var(--ok); font-weight: 700; }
        .ty-warn   { color: var(--warn); font-weight: 700; }

        /* Detail panel */
        #detail-panel {
          width: 380px;
          flex-shrink: 0;
          border-left: 1px solid var(--border);
          display: flex;
          flex-direction: column;
          overflow: hidden;
        }
        .dp-hdr {
          height: 28px;
          flex-shrink: 0;
          display: flex;
          align-items: center;
          gap: 8px;
          background: var(--surface);
          border-bottom: 1px solid var(--border);
          padding: 0 12px;
          font-size: 11px;
          color: var(--muted);
        }
        .dp-hdr .t { color: var(--text); font-size: 12px; }
        .dp-body { flex: 1; overflow-y: auto; padding: 10px 12px; }
        .dp-empty {
          flex: 1;
          display: flex;
          align-items: center;
          justify-content: center;
          font-size: 12px;
        }
        .d-sep { border-top: 1px solid var(--border); margin: 8px 0 6px; }
        .d-sec {
          font-size: 10px;
          color: var(--dim);
          text-transform: uppercase;
          letter-spacing: 0.08em;
          margin-bottom: 6px;
        }
        .payload-block {
          background: var(--surface);
          border: 1px solid var(--border-2);
          padding: 7px 10px;
          font-size: 11px;
          line-height: 1.7;
          font-family: inherit;
          margin-bottom: 6px;
          white-space: pre-wrap;
          word-break: break-all;
        }
        .pk { color: var(--accent); }
        .pv { color: var(--text); }
        .pv-err { color: var(--err); }
        .ctx-log { font-size: 11px; color: var(--dim); line-height: 1.9; }

        /* Status bar */
        .feed-statusbar {
          height: 22px;
          flex-shrink: 0;
          display: flex;
          align-items: center;
          gap: 8px;
          padding: 0 12px;
          background: var(--surface);
          border-top: 1px solid var(--border);
          font-size: 11px;
          color: var(--dim);
        }
        .feed-statusbar-right {
          margin-left: auto;
          display: flex;
          align-items: center;
          gap: 8px;
          font-size: 11px;
          color: var(--dim);
        }
        .live { font-size: 10px; }
        .kv { display: flex; gap: 8px; font-size: 11px; margin-bottom: 3px; }
        .kk { color: var(--dim); min-width: 60px; }
      `}</style>
    </div>
  );
}
