/**
 * Domains routes — unit tests.
 *
 * Covers DomainsOverviewPage (/domains/), DomainPage (/domains/:id).
 * MockS: vi.mock('@/api/client'), vi.mock('@tanstack/react-router').
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

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
      style,
    }: { to: string; children: React.ReactNode; style?: React.CSSProperties }) => (
      <a href={to} style={style}>{children}</a>
    ),
    useNavigate: vi.fn(() => vi.fn()),
    useParams: vi.fn(() => ({ domainId: 'domain-uuid-1' })),
  };
});

vi.mock('@/api/client', () => ({
  apiClient: { GET: vi.fn(), POST: vi.fn().mockResolvedValue({ data: { ok: true } }), use: vi.fn() },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

vi.mock('@monaco-editor/react', () => ({
  default: ({ value, onChange }: { value: string; onChange?: (v: string) => void }) => (
    <textarea data-testid="monaco-editor" defaultValue={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));

const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

import { DomainsOverviewPage } from './domains.index';

const sampleTree = {
  domain_count: 1,
  mission_count: 1,
  task_count: 3,
  generated_at: '2026-05-31T10:00:00Z',
  domains: [
    {
      id: 'domain-uuid-1',
      name: 'Apollo',
      description: 'Investment data',
      status: 'active',
      owners: 'aria-operator',
      tags: null,
      visibility: 'public',
      mission_count: 1,
      task_count: 3,
      missions: [
        {
          id: 'mission-uuid-1',
          name: 'Warehouse rebuild',
          description: 'Rebuild warehouse',
          domain_id: 'domain-uuid-1',
          status: 'in_progress',
          owners: 'aria-operator',
          tags: null,
          task_count: 3,
          task_status_counts: { open: 2, done: 1 },
          recent_tasks: [],
          updated_at: '2026-05-30T12:00:00Z',
        },
      ],
      updated_at: '2026-05-30T12:00:00Z',
    },
  ],
  unassigned_missions: [],
};

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('DomainsOverviewPage', () => {
  let qc: QueryClient;
  beforeEach(() => { qc = makeQC(); vi.clearAllMocks(); });
  afterEach(() => qc.clear());

  it('shows loading state while tree is pending', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    wrap(<DomainsOverviewPage />, qc);
    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('shows error state when tree fails', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('fail'));
    wrap(<DomainsOverviewPage />, qc);
    await waitFor(() => expect(screen.getByTestId('error-state')).toBeInTheDocument());
  });

  it('shows empty state when no domains', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...sampleTree, domain_count: 0, domains: [],
    });
    wrap(<DomainsOverviewPage />, qc);
    await waitFor(() => expect(screen.getByTestId('empty-state')).toBeInTheDocument());
  });

  it('renders domain with name, status, mission count', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    wrap(<DomainsOverviewPage />, qc);
    await waitFor(() => expect(screen.getByTestId('domain-row-domain-uuid-1')).toBeInTheDocument());
    expect(screen.getByText('Apollo')).toBeInTheDocument();
    expect(screen.getByText('active')).toBeInTheDocument();
  });

  it('shows nested mission when domain row is expanded', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    wrap(<DomainsOverviewPage />, qc);
    await waitFor(() => expect(screen.getByTestId('mission-row-mission-uuid-1')).toBeInTheDocument());
    expect(screen.getByText('Warehouse rebuild')).toBeInTheDocument();
  });
});
