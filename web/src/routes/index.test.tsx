/**
 * Dashboard (/) — unit tests.
 *
 * The Dashboard landing page shows three regions:
 *   - Fleet card: online/total agent count + link to /agents
 *   - Work card: domain/mission count + link to /domains
 *   - Recent activity: last ~8 events from the SSE stream (or empty state)
 *
 * Mocking strategy:
 *   - vi.mock('@/api/client') — controls apiClient.GET / unwrap
 *   - vi.mock('@/lib/useEventStream') — no real EventSource in jsdom
 *   - vi.mock('@tanstack/react-router') — no router context required
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ── Mocks (must be hoisted before component imports) ──────────────────────────

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    createFileRoute: (_path: string) => (opts: { component: React.ComponentType }) => ({
      ...opts,
      id: _path,
    }),
    Link: ({
      to,
      children,
    }: {
      to: string;
      children: React.ReactNode;
      [key: string]: unknown;
    }) => <a href={to}>{children}</a>,
  };
});

vi.mock('@/api/client', () => ({
  apiClient: {
    GET: vi.fn(),
    POST: vi.fn().mockResolvedValue({ data: { ok: true } }),
    use: vi.fn(),
  },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

// Mock useEventStream — fixture events for the activity strip
const fixtureEvents = [
  {
    id: 'ev-1',
    type: 'task.created',
    event: 'task.created',
    payload: {},
    receivedAt: Date.now() - 60_000,
  },
  {
    id: 'ev-2',
    type: 'agent.heartbeat',
    event: 'agent.heartbeat',
    payload: {},
    receivedAt: Date.now() - 30_000,
  },
];

vi.mock('@/lib/useEventStream', () => ({
  useEventStream: vi.fn(() => ({
    events: fixtureEvents,
    status: 'open',
    isLive: true,
    lastError: null,
    rateLimit: null,
    reconnectCount: 0,
    reconnectDelay: 0,
    messagesReceived: 2,
    clearEvents: vi.fn(),
  })),
}));

// ── Component import (after mocks) ────────────────────────────────────────────

import { Dashboard } from './index';

// ── Fixtures ──────────────────────────────────────────────────────────────────

const sampleCpAgents = [
  {
    id: 1,
    public_id: 'aria-engineer-f1a2b3c4',
    name: 'aria-engineer-f1a2b3c4',
    status: 'online',
    capabilities: 'mc-development',
    metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
    home_domain_id: 'dom-abc',
    current_domain_id: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-05-31T10:00:00Z',
  },
  {
    id: 2,
    public_id: 'worker-bot-99',
    name: 'worker-bot-99',
    status: 'online',
    capabilities: 'batch-processing',
    metadata: JSON.stringify({ runtime: 'custom', node_id: 'kai' }),
    home_domain_id: 'dom-def',
    current_domain_id: null,
    created_at: '2026-01-02T00:00:00Z',
    updated_at: '2026-05-31T09:00:00Z',
  },
  {
    id: 3,
    public_id: 'analytics-agent-77',
    name: 'analytics-agent-77',
    status: 'offline',
    capabilities: 'reporting',
    metadata: JSON.stringify({ runtime: 'claude-code' }),
    home_domain_id: 'dom-ghi',
    current_domain_id: null,
    created_at: '2026-01-03T00:00:00Z',
    updated_at: '2026-05-31T08:00:00Z',
  },
];

// Matches the real ExplorerTreeResponse shape from schema.gen.ts
// domains[].missions[] = ExplorerMissionNode[]
const sampleTree = {
  domain_count: 1,
  mission_count: 2,
  task_count: 5,
  generated_at: '2026-06-07T10:00:00Z',
  domains: [
    {
      id: 'd1',
      name: 'Apollo',
      description: 'Primary domain',
      status: 'active',
      owners: 'aria-operator',
      tags: null,
      visibility: 'public',
      mission_count: 2,
      task_count: 5,
      missions: [
        {
          id: 'm1',
          name: 'Mission Alpha',
          description: 'First mission',
          domain_id: 'd1',
          status: 'in_progress',
          owners: 'aria-operator',
          tags: null,
          task_count: 3,
          task_status_counts: {},
          recent_tasks: [],
          updated_at: '2026-06-07T09:00:00Z',
        },
        {
          id: 'm2',
          name: 'Mission Beta',
          description: 'Second mission',
          domain_id: 'd1',
          status: 'proposed',
          owners: 'aria-operator',
          tags: null,
          task_count: 2,
          task_status_counts: {},
          recent_tasks: [],
          updated_at: '2026-06-07T08:00:00Z',
        },
      ],
      updated_at: '2026-06-07T09:00:00Z',
    },
  ],
  unassigned_missions: [],
};

// ── Test helpers ──────────────────────────────────────────────────────────────

function renderWith(component: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{component}</QueryClientProvider>);
}

function makeQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
}

/**
 * Set up the three queries that Dashboard triggers:
 *   1. GET /api/agents → sampleCpAgents (3 agents: 2 online, 1 offline)
 *   2. GET /api/runtime/nodes → [] (no mesh nodes)
 *   3. GET /api/explorer/tree → sampleTree (1 domain, 2 missions)
 */
async function mockFullData() {
  const { apiClient, unwrap } = await import('@/api/client');
  (apiClient.GET as ReturnType<typeof vi.fn>)
    .mockResolvedValueOnce(sampleCpAgents) // /api/agents
    .mockResolvedValueOnce([]) // /api/runtime/nodes
    .mockResolvedValueOnce(sampleTree); // /api/explorer/tree
  (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('Dashboard', () => {
  let queryClient: QueryClient;

  beforeEach(async () => {
    queryClient = makeQueryClient();
    vi.clearAllMocks();

    // Re-apply useEventStream default after clearAllMocks
    const { useEventStream } = await import('@/lib/useEventStream');
    (useEventStream as ReturnType<typeof vi.fn>).mockReturnValue({
      events: fixtureEvents,
      status: 'open',
      isLive: true,
      lastError: null,
      rateLimit: null,
      reconnectCount: 0,
      reconnectDelay: 0,
      messagesReceived: 2,
      clearEvents: vi.fn(),
    });
  });

  afterEach(() => {
    queryClient.clear();
  });

  // ── Structure ─────────────────────────────────────────────────────────────

  it('renders the dashboard container', async () => {
    await mockFullData();
    renderWith(<Dashboard />, queryClient);

    // Dashboard testid is present immediately (before queries resolve)
    expect(screen.getByTestId('dashboard')).toBeInTheDocument();
  });

  // ── Fleet card ────────────────────────────────────────────────────────────

  it('shows fleet online count (2 of 3 agents online)', async () => {
    await mockFullData();
    renderWith(<Dashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('dash-fleet-online')).toBeInTheDocument());
    expect(screen.getByTestId('dash-fleet-online')).toHaveTextContent('2');
  });

  it('renders a link to /agents in the Fleet card', async () => {
    await mockFullData();
    renderWith(<Dashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('dashboard')).toBeInTheDocument());

    const agentsLink = screen.getByRole('link', { name: /agents/i });
    expect(agentsLink).toBeInTheDocument();
    expect(agentsLink).toHaveAttribute('href', '/agents');
  });

  // ── Work card ─────────────────────────────────────────────────────────────

  it('renders a link to /domains in the Work card', async () => {
    await mockFullData();
    renderWith(<Dashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('dashboard')).toBeInTheDocument());

    const domainsLink = screen.getByRole('link', { name: /domains/i });
    expect(domainsLink).toBeInTheDocument();
    expect(domainsLink).toHaveAttribute('href', '/domains');
  });

  it('shows domain count in the Work card', async () => {
    await mockFullData();
    renderWith(<Dashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('dash-work-domains')).toBeInTheDocument());
    // 1 domain in fixture
    expect(screen.getByTestId('dash-work-domains')).toHaveTextContent('1');
  });

  // ── Recent activity ───────────────────────────────────────────────────────

  it('renders fixture events in the recent activity strip', async () => {
    await mockFullData();
    renderWith(<Dashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('dashboard')).toBeInTheDocument());

    expect(screen.getByText(/task\.created/i)).toBeInTheDocument();
    expect(screen.getByText(/agent\.heartbeat/i)).toBeInTheDocument();
  });

  it('shows empty state when there are no events', async () => {
    const { useEventStream } = await import('@/lib/useEventStream');
    (useEventStream as ReturnType<typeof vi.fn>).mockReturnValue({
      events: [],
      status: 'open',
      isLive: true,
      lastError: null,
      rateLimit: null,
      reconnectCount: 0,
      reconnectDelay: 0,
      messagesReceived: 0,
      clearEvents: vi.fn(),
    });

    await mockFullData();
    renderWith(<Dashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('dashboard')).toBeInTheDocument());
    expect(screen.getByTestId('activity-empty')).toBeInTheDocument();
  });
});
