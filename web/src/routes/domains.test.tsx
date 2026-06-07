/**
 * Explorer screen — unit tests.
 *
 * Mocking strategy: vi.mock('@/api/client') replaces the openapi-fetch
 * singleton with a controlled mock (same pattern as governance.test.tsx).
 * Tests:
 *   - Tree renders from a sample ExplorerTreeResponse (domains + nested missions)
 *   - Clicking a mission node triggers the detail query and renders the result
 *   - Clicking "Open Task" within mission detail triggers task detail query
 *   - Loading/error states for tree and node detail
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ── Mocks must be hoisted before component imports ────────────────────────────

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

vi.mock('@/api/client', () => ({
  apiClient: {
    GET: vi.fn(),
    POST: vi.fn().mockResolvedValue({ data: { ok: true } }),
    use: vi.fn(),
  },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

// ── Import AFTER mocks ────────────────────────────────────────────────────────

import { ExplorerPage } from './domains';

// ── Sample fixtures ───────────────────────────────────────────────────────────

const sampleMission = {
  id: 'mission-uuid-1',
  name: 'Alpha Mission',
  description: 'First mission workstream',
  domain_id: 'domain-uuid-1',
  status: 'in_progress',
  owners: 'aria-operator',
  tags: null,
  task_count: 3,
  task_status_counts: { open: 1, in_progress: 1, done: 1 },
  recent_tasks: [
    {
      id: 101,
      mission_id: 'mission-uuid-1',
      title: 'Implement auth',
      status: 'done',
      owner: 'aria-operator',
      updated_at: '2026-05-30T12:00:00Z',
    },
  ],
  updated_at: '2026-05-30T12:00:00Z',
};

const sampleDomain = {
  id: 'domain-uuid-1',
  name: 'Production',
  description: 'Main production domain',
  status: 'active',
  owners: 'aria-operator',
  tags: null,
  visibility: 'public',
  mission_count: 1,
  task_count: 3,
  missions: [sampleMission],
  updated_at: '2026-05-30T12:00:00Z',
};

const sampleTree = {
  domain_count: 1,
  mission_count: 1,
  task_count: 3,
  generated_at: '2026-05-31T10:00:00Z',
  domains: [sampleDomain],
  unassigned_missions: [],
};

const sampleMissionDetail = {
  node_type: 'mission',
  node_id: 'mission-uuid-1',
  domain: null,
  mission: {
    id: 'mission-uuid-1',
    name: 'Alpha Mission',
    description: 'First mission workstream',
    domain_id: 'domain-uuid-1',
    status: 'in_progress',
    owners: 'aria-operator',
    tags: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-05-30T12:00:00Z',
  },
  tasks: [
    {
      id: 101,
      public_id: 'task-pub-101',
      mission_id: 'mission-uuid-1',
      title: 'Implement auth',
      description: 'Set up OIDC authentication',
      status: 'done',
      owner: 'aria-operator',
      contributors: '',
      created_at: '2026-01-01T00:00:00Z',
      updated_at: '2026-05-30T12:00:00Z',
    },
  ],
  missions: null,
  task: null,
};

const sampleTaskDetail = {
  node_type: 'task',
  node_id: 'task-pub-101',
  task: {
    id: 101,
    public_id: 'task-pub-101',
    mission_id: 'mission-uuid-1',
    title: 'Implement auth',
    description: 'Set up OIDC authentication',
    status: 'done',
    owner: 'aria-operator',
    contributors: '',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-05-30T12:00:00Z',
  },
  domain: null,
  mission: null,
  missions: null,
  tasks: null,
};

// ── Test helpers ──────────────────────────────────────────────────────────────

function makeQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
}

function renderExplorer(qc: QueryClient) {
  return render(
    <QueryClientProvider client={qc}>
      <ExplorerPage />
    </QueryClientProvider>,
  );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('ExplorerPage', () => {
  let queryClient: QueryClient;

  beforeEach(async () => {
    queryClient = makeQueryClient();
    vi.clearAllMocks();
    // Reset unwrap to passthrough after each test (clearAllMocks doesn't reset implementations)
    const { unwrap } = await import('@/api/client');
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));
  });

  afterEach(() => {
    queryClient.clear();
  });

  it('shows loading state while tree query is pending', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));

    renderExplorer(queryClient);

    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('shows error state when tree query fails', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockRejectedValue(
      Object.assign(new Error('Unauthorized'), { status: 401 }),
    );

    renderExplorer(queryClient);

    await waitFor(() => expect(screen.getByTestId('error-state')).toBeInTheDocument());
    expect(screen.getByText(/Failed to load explorer/)).toBeInTheDocument();
  });

  it('renders domain and nested mission from sample tree', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);

    renderExplorer(queryClient);

    // Domain row
    await waitFor(() => expect(screen.getByTestId('domain-row-domain-uuid-1')).toBeInTheDocument());
    expect(screen.getByText('Production')).toBeInTheDocument();

    // Nested mission row (indented under domain)
    expect(screen.getByTestId('mission-row-mission-uuid-1')).toBeInTheDocument();
    expect(screen.getByText('Alpha Mission')).toBeInTheDocument();
  });

  it('shows domain detail inline when a domain is clicked', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);

    renderExplorer(queryClient);

    await waitFor(() => expect(screen.getByTestId('domain-row-domain-uuid-1')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('domain-row-domain-uuid-1'));

    // Domain detail should appear without an API call for the node detail
    await waitFor(() => {
      // The domain name appears in the detail pane header
      const headers = screen.getAllByText('Production');
      expect(headers.length).toBeGreaterThanOrEqual(1);
    });
    expect(screen.getByText('Main production domain')).toBeInTheDocument();
  });

  it('fetches and renders mission detail when a mission is clicked', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    // First call: tree; subsequent: mission detail
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleTree)
      .mockResolvedValue(sampleMissionDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderExplorer(queryClient);

    await waitFor(() =>
      expect(screen.getByTestId('mission-row-mission-uuid-1')).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTestId('mission-row-mission-uuid-1'));

    // Mission detail should appear
    await waitFor(() => {
      expect(screen.getByText('First mission workstream')).toBeInTheDocument();
    });

    // Task within mission detail
    expect(screen.getByText('Implement auth')).toBeInTheDocument();
  });

  it('fetches and renders task detail when Open Task is clicked in mission detail', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleTree) // tree
      .mockResolvedValueOnce(sampleMissionDetail) // mission detail
      .mockResolvedValue(sampleTaskDetail); // task detail
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderExplorer(queryClient);

    await waitFor(() =>
      expect(screen.getByTestId('mission-row-mission-uuid-1')).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTestId('mission-row-mission-uuid-1'));

    // Wait for mission detail to render
    await waitFor(() => expect(screen.getByTestId('open-task-101')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('open-task-101'));

    // Task detail should appear
    await waitFor(() => {
      expect(screen.getByText('task-pub-101')).toBeInTheDocument();
    });
    expect(screen.getByText('Set up OIDC authentication')).toBeInTheDocument();
  });

  it('shows detail-loading state while mission detail is fetching', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValueOnce(sampleTree);
    // Mission detail never resolves
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderExplorer(queryClient);

    await waitFor(() =>
      expect(screen.getByTestId('mission-row-mission-uuid-1')).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTestId('mission-row-mission-uuid-1'));

    await waitFor(() => expect(screen.getByTestId('detail-loading-state')).toBeInTheDocument());
  });

  it('shows detail-error state when node detail fails', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValueOnce(sampleTree);
    (apiClient.GET as ReturnType<typeof vi.fn>).mockRejectedValue(
      Object.assign(new Error('Not found'), { status: 404 }),
    );
    (unwrap as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleTree)
      .mockRejectedValue(Object.assign(new Error('Not found'), { status: 404 }));

    renderExplorer(queryClient);

    await waitFor(() =>
      expect(screen.getByTestId('mission-row-mission-uuid-1')).toBeInTheDocument(),
    );

    fireEvent.click(screen.getByTestId('mission-row-mission-uuid-1'));

    await waitFor(() => expect(screen.getByTestId('detail-error-state')).toBeInTheDocument());
    expect(screen.getByText(/Failed to load detail/)).toBeInTheDocument();
  });

  it('shows empty state when tree is empty', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
      domain_count: 0,
      mission_count: 0,
      task_count: 0,
      generated_at: '2026-05-31T10:00:00Z',
      domains: [],
      unassigned_missions: [],
    });

    renderExplorer(queryClient);

    await waitFor(() => expect(screen.getByTestId('empty-state')).toBeInTheDocument());
    expect(screen.getByText('No domains or missions')).toBeInTheDocument();
  });

  it('shows prompt to select a node when nothing is selected', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);

    renderExplorer(queryClient);

    await waitFor(() => expect(screen.getByTestId('detail-empty-state')).toBeInTheDocument());
    expect(screen.getByText('Select a domain or mission.')).toBeInTheDocument();
  });
});
