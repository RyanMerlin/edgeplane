/**
 * Feed screen — unit tests (Phase 3).
 *
 * Mocking strategy: seed the Zustand event-stream store directly before render.
 * The useEventStream hook calls connect() on mount — we mock the store so
 * connect() is a no-op (no real EventSource opened in tests).
 *
 * Tests:
 *   - Events render from a seeded store
 *   - Text filter narrows the list
 *   - Chip filters narrow the list
 *   - Alerts-only toggle shows only alert events
 *   - Empty state renders when no events
 *   - Disconnected state shows offline indicator
 *   - Rate counter is displayed
 *   - Status bar error/governance/warn counters
 *   - Row click selects the detail panel
 */

import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// ── Mocks ─────────────────────────────────────────────────────────────────────

// Mock TanStack Router (no router context needed for component tests)
vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    createFileRoute: (_path: string) => (opts: { component: React.ComponentType }) => ({
      ...opts,
      id: _path,
    }),
  };
});

// Mock useEventStream so it returns seeded state without opening a real EventSource.
// Each test overrides this via seedStore().
let _mockStoreState = {
  events: [] as import('@/stores/eventStream').MatrixEvent[],
  status: 'open' as import('@/stores/eventStream').ConnectionStatus,
  lastError: null as string | null,
  rateLimit: null as import('@/stores/eventStream').RateLimit | null,
  reconnectCount: 0,
  reconnectDelay: 1000,
  messagesReceived: 0,
  clearEvents: vi.fn(),
  isLive: true,
};

vi.mock('@/lib/useEventStream', () => ({
  useEventStream: () => _mockStoreState,
}));

// ── Import component AFTER mocks ──────────────────────────────────────────────

import { FeedPage } from './feed';

// ── Fixtures ──────────────────────────────────────────────────────────────────

import type { MatrixEvent } from '@/stores/eventStream';

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
    type: 'task_finished',
    agent_id: 'aria-001',
    mission_id: 'm-abc',
    payload: { name: 'task done' },
    receivedAt: NOW - 100,
  }),
  makeEvent({
    type: 'step_error',
    agent_id: 'aria-002',
    payload: { error: 'boom' },
    receivedAt: NOW - 200,
  }),
  makeEvent({ type: 'governance', payload: { rule: 'no-delete' }, receivedAt: NOW - 300 }),
  makeEvent({ type: 'overlap_detected', payload: { detail: 'conflict' }, receivedAt: NOW - 400 }),
  makeEvent({ type: 'heartbeat', payload: {}, receivedAt: NOW - 500 }),
  makeEvent({
    type: 'artifact',
    agent_id: 'aria-003',
    payload: { name: 'my-artifact' },
    receivedAt: NOW - 600,
  }),
];

function seedStore(overrides: Partial<typeof _mockStoreState> = {}) {
  _mockStoreState = {
    events: sampleEvents,
    status: 'open',
    lastError: null,
    rateLimit: null,
    reconnectCount: 0,
    reconnectDelay: 1000,
    messagesReceived: sampleEvents.length,
    clearEvents: vi.fn(),
    isLive: true,
    ...overrides,
  };
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('FeedPage', () => {
  beforeEach(() => {
    seedStore();
  });

  it('renders events from the seeded store', () => {
    render(<FeedPage />);
    const rows = screen.getAllByTestId('feed-row');
    expect(rows).toHaveLength(sampleEvents.length);
  });

  it('shows empty state when no events', () => {
    seedStore({ events: [] });
    render(<FeedPage />);
    expect(screen.getByTestId('feed-empty')).toBeInTheDocument();
    expect(screen.getByText('Waiting for events…')).toBeInTheDocument();
  });

  it('shows "No matching events." when filter returns empty', () => {
    render(<FeedPage />);
    const input = screen.getByTestId('filter-input');
    fireEvent.change(input, { target: { value: 'zzz-no-match' } });
    expect(screen.getByTestId('feed-empty')).toBeInTheDocument();
    expect(screen.getByText('No matching events.')).toBeInTheDocument();
  });

  it('text filter narrows the event list', () => {
    render(<FeedPage />);
    const input = screen.getByTestId('filter-input');
    // Filter by agent
    fireEvent.change(input, { target: { value: 'aria-001' } });
    const rows = screen.getAllByTestId('feed-row');
    expect(rows).toHaveLength(1);
  });

  it('Errors chip filter shows only error-class events', () => {
    render(<FeedPage />);
    fireEvent.click(screen.getByTestId('chip-errors'));
    // step_error + overlap_detected = 2
    const rows = screen.getAllByTestId('feed-row');
    expect(rows).toHaveLength(2);
  });

  it('Governance chip filter shows only governance events', () => {
    render(<FeedPage />);
    fireEvent.click(screen.getByTestId('chip-governance'));
    const rows = screen.getAllByTestId('feed-row');
    expect(rows).toHaveLength(1);
  });

  it('Artifacts chip filter shows only artifact events', () => {
    render(<FeedPage />);
    fireEvent.click(screen.getByTestId('chip-artifacts'));
    const rows = screen.getAllByTestId('feed-row');
    expect(rows).toHaveLength(1);
  });

  it('Heartbeat chip filter shows only heartbeat events', () => {
    render(<FeedPage />);
    fireEvent.click(screen.getByTestId('chip-heartbeat'));
    const rows = screen.getAllByTestId('feed-row');
    expect(rows).toHaveLength(1);
  });

  it('Alerts-only toggle shows only error/governance/warn events', () => {
    render(<FeedPage />);
    fireEvent.click(screen.getByTestId('alerts-toggle'));
    // step_error (a-err) + governance (a-gov) + overlap_detected (a-warn) = 3
    const rows = screen.getAllByTestId('feed-row');
    expect(rows).toHaveLength(3);
  });

  it('shows LIVE indicator when connected', () => {
    render(<FeedPage />);
    expect(screen.getByTestId('status-live')).toBeInTheDocument();
  });

  it('shows offline indicator when disconnected', () => {
    seedStore({ status: 'closed', isLive: false });
    render(<FeedPage />);
    expect(screen.queryByTestId('status-live')).not.toBeInTheDocument();
    expect(screen.getByTestId('status-offline')).toBeInTheDocument();
  });

  it('shows error banner when lastError is set and not live', () => {
    seedStore({ status: 'reconnecting', isLive: false, lastError: 'Connection lost' });
    render(<FeedPage />);
    expect(screen.getByTestId('error-banner')).toBeInTheDocument();
    expect(screen.getByText(/Connection lost/)).toBeInTheDocument();
  });

  it('displays event count', () => {
    render(<FeedPage />);
    expect(screen.getByTestId('event-count')).toHaveTextContent(`${sampleEvents.length} events`);
  });

  it('displays rate counter', () => {
    render(<FeedPage />);
    expect(screen.getByTestId('rate-counter')).toBeInTheDocument();
  });

  it('shows error count in status bar', () => {
    render(<FeedPage />);
    // step_error = 1 error
    expect(screen.getByTestId('error-count')).toHaveTextContent('1 error');
  });

  it('shows governance count in status bar', () => {
    render(<FeedPage />);
    expect(screen.getByTestId('gov-count')).toHaveTextContent('1 governance');
  });

  it('shows warn count in status bar', () => {
    render(<FeedPage />);
    expect(screen.getByTestId('warn-count')).toHaveTextContent('1 overlap');
  });

  it('clicking a row populates the detail panel with event type in the header', () => {
    render(<FeedPage />);
    const rows = screen.getAllByTestId('feed-row');
    fireEvent.click(rows[0]);
    // Detail panel should now show event type in the dp-hdr .t span
    expect(screen.getByTestId('detail-panel')).toBeInTheDocument();
    // Use getAllByText since the type label appears both in the row and in the panel header
    const typeLabels = screen.getAllByText('task_finished');
    expect(typeLabels.length).toBeGreaterThanOrEqual(2); // row + detail panel
  });

  it('detail panel shows "Select a row to inspect" initially', () => {
    render(<FeedPage />);
    expect(screen.getByTestId('detail-panel')).toHaveTextContent('Select a row to inspect');
  });

  it('rateLimit display shown when rateLimit present', () => {
    seedStore({
      rateLimit: { limit: 100, remaining: 50, reset_at: new Date().toISOString() },
    });
    render(<FeedPage />);
    expect(screen.getByText(/rl 50\/100/)).toBeInTheDocument();
  });
});
