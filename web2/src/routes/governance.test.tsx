/**
 * Governance screen — unit tests.
 *
 * Mocking strategy: vi.mock('@/api/client') replaces the openapi-fetch
 * singleton with a controlled mock so tests don't touch the network.
 *
 * Rendering strategy: we import the GovernancePage component directly
 * (it's exported as a named export for testability) and wrap it with
 * QueryClientProvider. TanStack Router's createFileRoute is called at
 * module load time but the component can be rendered independently.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ── Mocks must be hoisted before imports that depend on them ──────────────────

// Mock TanStack Router's createFileRoute so the module loads cleanly in jsdom
// without a real router context. The route registration is a side effect we
// don't need for component-level tests.
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

// Mock the typed API client
vi.mock('@/api/client', () => ({
  apiClient: { GET: vi.fn(), POST: vi.fn(), use: vi.fn() },
  unwrap: vi.fn((p: Promise<unknown>) => p),
}));

// Mock toast store
const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

// ── Import component AFTER mocks are registered ───────────────────────────────

// We import the named export added to governance.tsx for test access
import { GovernancePage } from './governance';

// ── Sample fixture ────────────────────────────────────────────────────────────

const samplePolicy = {
  id: 1,
  version: 3,
  state: 'active',
  policy: {
    global: {
      require_approval_for_mutations: true,
      allow_create_without_approval: false,
      allow_update: true,
      allow_delete: false,
      allow_publish: true,
    },
    actions: {
      'domain.create': { enabled: true, requires_approval: false },
      'domain.delete': { enabled: false, requires_approval: true },
    },
    terminal: { allow_create_actions: true, allow_publish_actions: false },
    mcp: { allow_mutation_tools: false },
  },
  change_note: 'Initial policy',
  created_by: 'admin@example.com',
  published_by: 'admin@example.com',
  published_at: '2026-01-15T10:00:00Z',
  created_at: '2026-01-15T09:00:00Z',
  updated_at: '2026-01-15T10:00:00Z',
};

// ── Test helper ───────────────────────────────────────────────────────────────

function renderGovernance(queryClient: QueryClient) {
  return render(
    <QueryClientProvider client={queryClient}>
      <GovernancePage />
    </QueryClientProvider>,
  );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('GovernancePage', () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false, refetchOnWindowFocus: false },
        mutations: { retry: false },
      },
    });
    vi.clearAllMocks();
  });

  afterEach(() => {
    queryClient.clear();
  });

  it('shows loading state while query is pending', async () => {
    const { unwrap } = await import('@/api/client');
    (unwrap as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));

    renderGovernance(queryClient);

    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('renders policy data when query succeeds', async () => {
    const { unwrap } = await import('@/api/client');
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(samplePolicy);

    renderGovernance(queryClient);

    // State badge
    await waitFor(() => expect(screen.getByText('active')).toBeInTheDocument());
    // Version
    expect(screen.getByText('v3')).toBeInTheDocument();
    // Meta fields
    expect(screen.getByText('Initial policy')).toBeInTheDocument();
    // Sections
    expect(screen.getByText('Global Flags')).toBeInTheDocument();
    expect(screen.getByText('Action Rules')).toBeInTheDocument();
    // Action name after dot-split
    expect(screen.getByText('create')).toBeInTheDocument();
  });

  it('shows error state when query fails', async () => {
    const { unwrap } = await import('@/api/client');
    (unwrap as ReturnType<typeof vi.fn>).mockRejectedValue(
      Object.assign(new Error('Unauthorized'), { status: 401 }),
    );

    renderGovernance(queryClient);

    await waitFor(() => expect(screen.getByTestId('error-state')).toBeInTheDocument());
    expect(screen.getByText(/Failed to load policy/)).toBeInTheDocument();
  });

  it('shows empty state when data resolves to null', async () => {
    const { unwrap } = await import('@/api/client');
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(null);

    renderGovernance(queryClient);

    await waitFor(() => expect(screen.getByTestId('empty-state')).toBeInTheDocument());
    expect(screen.getByText('No policies configured')).toBeInTheDocument();
  });
});
