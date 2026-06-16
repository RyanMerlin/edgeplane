/**
 * RawEventList — unit tests.
 *
 * Migrated from the former routes/matrix.test.tsx. RawEventList is now a pure
 * presentational component (the raw "Raw" sub-view of /feed), so these tests
 * pass the event-stream slice as props directly — no useEventStream mock and no
 * router context needed.
 */

import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ConnectionStatus, MatrixEvent, RateLimit } from '@/stores/eventStream';
import { RawEventList } from './RawEventList';

// ── Fixtures ──────────────────────────────────────────────────────────────────

function makeEvent(overrides: Partial<MatrixEvent> = {}): MatrixEvent {
  return {
    type: 'heartbeat',
    payload: {},
    receivedAt: Date.now(),
    ...overrides,
  };
}

const NOW = Date.now();

const sampleEvents: MatrixEvent[] = [
  makeEvent({
    type: 'task_claimed',
    agent_id: 'aria-001',
    mission_id: 'm-abc',
    payload: { name: 'T1' },
    receivedAt: NOW - 100,
  }),
  makeEvent({
    type: 'step_error',
    agent_id: 'aria-002',
    payload: { error: 'fail' },
    receivedAt: NOW - 200,
  }),
  makeEvent({ type: 'heartbeat', payload: {}, receivedAt: NOW - 400 }),
  makeEvent({ type: 'artifact', payload: { name: 'my-artifact' }, receivedAt: NOW - 500 }),
];

interface RenderOpts {
  events?: MatrixEvent[];
  status?: ConnectionStatus;
  lastError?: string | null;
  rateLimit?: RateLimit | null;
  clearEvents?: () => void;
}

function renderRaw(opts: RenderOpts = {}) {
  const clearEvents = opts.clearEvents ?? vi.fn();
  render(
    <RawEventList
      events={opts.events ?? sampleEvents}
      status={opts.status ?? 'open'}
      lastError={opts.lastError ?? null}
      rateLimit={opts.rateLimit ?? null}
      clearEvents={clearEvents}
    />,
  );
  return { clearEvents };
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('RawEventList', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders all events from props', () => {
    renderRaw();
    const rows = screen.getAllByTestId('matrix-row');
    expect(rows).toHaveLength(sampleEvents.length);
  });

  it('shows empty state when no events', () => {
    renderRaw({ events: [] });
    expect(screen.getByTestId('matrix-empty')).toBeInTheDocument();
    expect(screen.getByText('Waiting for events…')).toBeInTheDocument();
  });

  it('per-type filter narrows the list', () => {
    renderRaw();
    fireEvent.change(screen.getByTestId('type-filter'), { target: { value: 'task_claimed' } });
    const rows = screen.getAllByTestId('matrix-row');
    expect(rows).toHaveLength(1);
  });

  it('populates type filter dropdown with observed event types', () => {
    renderRaw();
    const select = screen.getByTestId('type-filter');
    const options = Array.from(select.querySelectorAll('option')).map((o) => o.textContent);
    expect(options).toContain('task_claimed');
    expect(options).toContain('step_error');
    expect(options).toContain('heartbeat');
    expect(options).toContain('artifact');
  });

  it('shows JSON payload expander for events with non-empty payload', () => {
    renderRaw();
    const expanders = screen.getAllByTestId('payload-expander');
    expect(expanders.length).toBeGreaterThan(0);
  });

  it('does NOT show payload expander for empty payload', () => {
    renderRaw({ events: [makeEvent({ type: 'heartbeat', payload: {} })] });
    expect(screen.queryAllByTestId('payload-expander')).toHaveLength(0);
  });

  it('shows LIVE indicator when connected', () => {
    renderRaw();
    expect(screen.getByTestId('matrix-status-dot')).toHaveTextContent('●');
    expect(screen.getByTestId('matrix-status-label')).toHaveTextContent('live');
  });

  it('shows offline indicator when disconnected', () => {
    renderRaw({ status: 'closed' });
    expect(screen.getByTestId('matrix-status-dot')).toHaveTextContent('○');
    expect(screen.getByTestId('matrix-status-label')).toHaveTextContent('offline');
  });

  it('shows reconnecting label when status is reconnecting', () => {
    renderRaw({ status: 'reconnecting' });
    expect(screen.getByTestId('matrix-status-label')).toHaveTextContent('reconnecting…');
  });

  it('shows error banner when lastError is set and not live', () => {
    renderRaw({ status: 'reconnecting', lastError: 'Connection lost' });
    expect(screen.getByTestId('matrix-error-banner')).toBeInTheDocument();
    expect(screen.getByText(/Connection lost/)).toBeInTheDocument();
  });

  it('shows rate-limit display when rateLimit present', () => {
    renderRaw({ rateLimit: { limit: 100, remaining: 30, reset_at: new Date().toISOString() } });
    expect(screen.getByTestId('rate-limit-display')).toHaveTextContent('rl 30/100');
  });

  it('clear button calls clearEvents', () => {
    const { clearEvents } = renderRaw();
    fireEvent.click(screen.getByTestId('clear-btn'));
    expect(clearEvents).toHaveBeenCalledOnce();
  });

  it('shows "Show more" when event count exceeds PAGE_SIZE', () => {
    const manyEvents = Array.from({ length: 55 }, (_, i) =>
      makeEvent({ type: 'heartbeat', payload: { seq: i }, receivedAt: NOW - i * 10 }),
    );
    renderRaw({ events: manyEvents });
    expect(screen.getByTestId('show-more')).toBeInTheDocument();
    expect(screen.getByText(/Show more \(5 remaining\)/)).toBeInTheDocument();
  });

  it('does NOT show "Show more" when event count is within PAGE_SIZE', () => {
    renderRaw();
    expect(screen.queryByTestId('show-more')).not.toBeInTheDocument();
  });

  it('displays event count in the bar', () => {
    renderRaw();
    expect(screen.getByTestId('matrix-counts')).toHaveTextContent(`${sampleEvents.length} events`);
  });
});
