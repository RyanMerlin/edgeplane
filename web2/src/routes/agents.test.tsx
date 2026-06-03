/**
 * Agents screens — unit tests.
 *
 * Mocking strategy: vi.mock('@/api/client') replaces the openapi-fetch
 * singleton with a controlled mock (same pattern as governance.test.tsx).
 * No network calls; each test controls what `apiClient.GET` returns.
 *
 * AgentsPage: tests list render, loading, error, empty states.
 * AgentDetailPage: tests detail render, loading, 404 not-found.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ── Mocks must be hoisted before component imports ────────────────────────────

// Stub TanStack Router so modules load cleanly in jsdom without a router context.
vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    createFileRoute: (_path: string) => (opts: { component: React.ComponentType }) => ({
      ...opts,
      id: _path,
      useParams: () => ({ agentId: 'aria-test-abc12345' }),
    }),
    // Link renders as a plain anchor in tests
    Link: ({
      to,
      children,
      className,
      style,
    }: {
      to: string;
      children: React.ReactNode;
      className?: string;
      style?: React.CSSProperties;
    }) => (
      <a href={to} className={className} style={style}>
        {children}
      </a>
    ),
    useNavigate: () => vi.fn(),
  };
});

// Mock the typed API client (same shape as governance.test.tsx)
vi.mock('@/api/client', () => ({
  apiClient: {
    GET: vi.fn(),
    POST: vi.fn().mockResolvedValue({ data: { ok: true } }),
    use: vi.fn(),
  },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

// Mock toast store (not used by agents pages but imported transitively)
const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

// ── Import components AFTER mocks ─────────────────────────────────────────────

import { AgentsPage } from './agents';
import { AgentDetailPage } from './agents.$agentId';

// ── Sample fixtures ───────────────────────────────────────────────────────────

const sampleAgents = [
  {
    id: 1,
    public_id: 'aria-operator-e8820c0d',
    name: 'aria-operator',
    status: 'online',
    capabilities: 'fleet-management,code-editing',
    metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
    home_domain_id: 'dom-abc',
    current_domain_id: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-05-31T10:00:00Z',
  },
  {
    id: 2,
    public_id: 'aria-research-f1a2b3c4',
    name: 'aria-research',
    status: 'offline',
    capabilities: 'research,analysis',
    metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
    home_domain_id: 'dom-def',
    current_domain_id: null,
    created_at: '2026-01-02T00:00:00Z',
    updated_at: '2026-05-31T09:00:00Z',
  },
];

const sampleAgent = sampleAgents[0];

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

// ── AgentsPage tests ──────────────────────────────────────────────────────────

describe('AgentsPage', () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    queryClient = makeQueryClient();
    vi.clearAllMocks();
  });

  afterEach(() => {
    queryClient.clear();
  });

  it('shows loading state while both queries are pending', async () => {
    const { apiClient } = await import('@/api/client');
    // Never-resolving promises keep queries in loading state
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));

    renderWith(<AgentsPage />, queryClient);

    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('renders agent rows from a sample Agent[]', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    // unwrap passes through whatever GET returns
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleAgents) // GET /api/agents → cp list
      .mockResolvedValueOnce([]) // GET /api/runtime/nodes → empty node list
      .mockResolvedValue([]); // fallback for any further GET calls

    // unwrap is identity in the mock; stub it to return the value directly
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<AgentsPage />, queryClient);

    await waitFor(() => expect(screen.getByTestId('agents-table')).toBeInTheDocument());

    // Both agent names should be in the table
    expect(screen.getByText('aria-operator')).toBeInTheDocument();
    expect(screen.getByText('aria-research')).toBeInTheDocument();

    // Status badges
    expect(screen.getByText('online')).toBeInTheDocument();
    expect(screen.getByText('offline')).toBeInTheDocument();

    // Both rows exist
    expect(screen.getByTestId('agent-row-aria-operator-e8820c0d')).toBeInTheDocument();
    expect(screen.getByTestId('agent-row-aria-research-f1a2b3c4')).toBeInTheDocument();
  });

  it('shows error state when both queries fail', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (unwrap as ReturnType<typeof vi.fn>).mockRejectedValue(
      Object.assign(new Error('Unauthorized'), { status: 401 }),
    );
    (apiClient.GET as ReturnType<typeof vi.fn>).mockRejectedValue(
      Object.assign(new Error('Unauthorized'), { status: 401 }),
    );

    renderWith(<AgentsPage />, queryClient);

    await waitFor(() => expect(screen.getByTestId('error-state')).toBeInTheDocument());
    expect(screen.getByText(/Failed to load agents/)).toBeInTheDocument();
  });

  it('shows empty state when both queries resolve with empty data', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue([]);

    renderWith(<AgentsPage />, queryClient);

    await waitFor(() => expect(screen.getByTestId('empty-state')).toBeInTheDocument());
    expect(screen.getByText('No agents registered')).toBeInTheDocument();
  });
});

// ── AgentDetailPage tests ─────────────────────────────────────────────────────

describe('AgentDetailPage', () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    queryClient = makeQueryClient();
    vi.clearAllMocks();
  });

  afterEach(() => {
    queryClient.clear();
  });

  it('shows loading state while query is pending', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));

    renderWith(<AgentDetailPage />, queryClient);

    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('renders a single agent record', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
      data: sampleAgent,
      error: undefined,
      response: { ok: true, status: 200 },
    });

    renderWith(<AgentDetailPage />, queryClient);

    await waitFor(() => expect(screen.getAllByText('aria-operator').length).toBeGreaterThan(0));

    // Status badge — appears in pane header and in the dl
    expect(screen.getAllByText('online').length).toBeGreaterThan(0);

    // Public ID shown in the detail grid
    expect(screen.getByText('aria-operator-e8820c0d')).toBeInTheDocument();

    // Capabilities
    expect(screen.getByText('fleet-management,code-editing')).toBeInTheDocument();

    // Metadata fields parsed out
    expect(screen.getByText('claude-code')).toBeInTheDocument();
    // 'excalibur' appears in both the metadata table and the acp-node-id span
    expect(screen.getAllByText('excalibur').length).toBeGreaterThanOrEqual(1);

    // Back link to agents list
    expect(screen.getByRole('link', { name: /← Agents/ })).toBeInTheDocument();
  });

  it('shows not-found affordance on 404 response', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
      data: undefined,
      error: undefined,
      response: { ok: false, status: 404 },
    });

    renderWith(<AgentDetailPage />, queryClient);

    await waitFor(() => expect(screen.getByTestId('not-found-state')).toBeInTheDocument());
    expect(screen.getByText('Agent not found')).toBeInTheDocument();
  });

  it('shows generic error state on non-404 failure', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
      data: undefined,
      error: { error: 'Internal Server Error' },
      response: { ok: false, status: 500 },
    });

    renderWith(<AgentDetailPage />, queryClient);

    await waitFor(() => expect(screen.getByTestId('error-state')).toBeInTheDocument());
    expect(screen.getByText(/Failed to load agent/)).toBeInTheDocument();
  });
});
