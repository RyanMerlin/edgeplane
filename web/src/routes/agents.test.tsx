/**
 * Agents routes unit tests.
 *
 * Covers:
 *   - AgentDetailPage (/agents/$agentId): loading, data, 404, generic error states.
 *   - AgentsIndexPage (/agents/): renders the table and navigates on row click.
 *
 * Mocking strategy: vi.mock('@/api/client') replaces the openapi-fetch
 * singleton with a controlled mock. No network calls.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
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
    // useNavigate: vi.fn(() => vi.fn()) so tests can override the spy per-test.
    useNavigate: vi.fn(() => vi.fn()),
  };
});

// Mock the typed API client (same shape as onboarding.test.tsx)
vi.mock('@/api/client', () => ({
  apiClient: {
    GET: vi.fn(),
    POST: vi.fn().mockResolvedValue({ data: { ok: true } }),
    use: vi.fn(),
  },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

// Mock toast store (imported transitively by the conversation pane)
const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

// ── Import components AFTER mocks ─────────────────────────────────────────────

import { AgentDetailPage } from './agents.$agentId';
import { AgentsIndexPage } from './agents.index';

// ── Sample fixtures ───────────────────────────────────────────────────────────

const sampleAgent = {
  id: 1,
  public_id: 'my-agent-operator-e8820c0d',
  name: 'my-agent-operator',
  status: 'online',
  capabilities: 'fleet-management,code-editing',
  metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'node-0' }),
  home_domain_id: 'dom-abc',
  current_domain_id: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-05-31T10:00:00Z',
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

    await waitFor(() => expect(screen.getAllByText('my-agent-operator').length).toBeGreaterThan(0));

    // Status badge — appears in pane header and in the dl
    expect(screen.getAllByText('online').length).toBeGreaterThan(0);

    // Public ID shown in the detail grid
    expect(screen.getByText('my-agent-operator-e8820c0d')).toBeInTheDocument();

    // Capabilities
    expect(screen.getByText('fleet-management,code-editing')).toBeInTheDocument();

    // Metadata fields parsed out
    expect(screen.getByText('claude-code')).toBeInTheDocument();
    // 'node-0' appears in both the metadata table and the acp-node-id span
    expect(screen.getAllByText('node-0').length).toBeGreaterThanOrEqual(1);

    // Back link is GONE — breadcrumbs in the shell header now handle up-nav
    expect(screen.queryByRole('link', { name: /← Fleet/ })).not.toBeInTheDocument();

    // Detail container is present with the correct data-testid
    expect(screen.getByTestId('agent-detail')).toBeInTheDocument();
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

// ── AgentsIndexPage tests ─────────────────────────────────────────────────────

describe('AgentsIndexPage', () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    queryClient = makeQueryClient();
    vi.clearAllMocks();
  });

  afterEach(() => {
    queryClient.clear();
  });

  it('renders the agents table', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    const { useNavigate } = await import('@tanstack/react-router');

    // CP agents endpoint returns our sample agent
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({ data: [sampleAgent] });
    // unwrap resolves arrays: first call = cp agents, second = nodes (empty), no per-node calls
    (unwrap as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce([sampleAgent]) // /api/agents
      .mockResolvedValueOnce([]); // /api/runtime/nodes

    const navigate = vi.fn();
    (useNavigate as ReturnType<typeof vi.fn>).mockReturnValue(navigate);

    renderWith(<AgentsIndexPage />, queryClient);

    await waitFor(() => expect(screen.getByTestId('agents-table')).toBeInTheDocument());

    // Row for our sample agent must be present
    expect(screen.getByTestId('agent-row-my-agent-operator-e8820c0d')).toBeInTheDocument();
  });

  it('navigates to detail on row click', async () => {
    const { unwrap } = await import('@/api/client');
    const { useNavigate } = await import('@tanstack/react-router');

    (unwrap as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce([sampleAgent])
      .mockResolvedValueOnce([]);

    const navigate = vi.fn();
    (useNavigate as ReturnType<typeof vi.fn>).mockReturnValue(navigate);

    renderWith(<AgentsIndexPage />, queryClient);

    await waitFor(() =>
      expect(screen.getByTestId('agent-row-my-agent-operator-e8820c0d')).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTestId('agent-row-my-agent-operator-e8820c0d'));

    expect(navigate).toHaveBeenCalledWith({
      to: '/agents/$agentId',
      params: { agentId: 'my-agent-operator-e8820c0d' },
    });
  });
});
