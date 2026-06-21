import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
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
      'data-testid': testid,
    }: {
      to: string;
      children: React.ReactNode;
      style?: React.CSSProperties;
      'data-testid'?: string;
    }) => (
      <a href={to} style={style} data-testid={testid}>
        {children}
      </a>
    ),
    useParams: vi.fn(() => ({ domainId: 'domain-uuid-1' })),
    useNavigate: vi.fn(() => vi.fn()),
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

vi.mock('@monaco-editor/react', () => ({
  default: ({ value }: { value: string }) => (
    <textarea data-testid="monaco-editor" defaultValue={value} readOnly />
  ),
}));

const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

import { DomainPage } from './domains.$domainId';

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
      owners: 'my-agent-operator',
      tags: null,
      visibility: 'public',
      mission_count: 1,
      task_count: 3,
      updated_at: '2026-05-30T12:00:00Z',
      missions: [
        {
          id: 'mission-uuid-1',
          name: 'Warehouse rebuild',
          description: 'rebuild',
          domain_id: 'domain-uuid-1',
          status: 'in_progress',
          owners: 'my-agent-operator',
          tags: null,
          task_count: 3,
          task_status_counts: { open: 2, done: 1 },
          recent_tasks: [],
          updated_at: '2026-05-30T12:00:00Z',
        },
      ],
    },
  ],
  unassigned_missions: [],
};

const sampleNorthstar = {
  northstar_md: '# Apollo\n\nDrives investment data.',
  northstar_version: 3,
  northstar_modified_by: 'my-agent-operator',
  northstar_modified_at: '2026-06-01T10:00:00Z',
};

function makeQC() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
  });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('DomainPage', () => {
  let qc: QueryClient;
  beforeEach(() => {
    qc = makeQC();
    vi.clearAllMocks();
  });
  afterEach(() => qc.clear());

  it('shows domain name in header after loading', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    global.fetch = vi
      .fn()
      .mockResolvedValue({ ok: true, json: () => Promise.resolve(sampleNorthstar) });
    wrap(<DomainPage />, qc);
    await waitFor(() => expect(screen.getByTestId('domain-header')).toBeInTheDocument());
    expect(screen.getByText('Apollo')).toBeInTheDocument();
    expect(screen.getByText('active')).toBeInTheDocument();
  });

  it('renders Northstar tab by default with Monaco editor', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    global.fetch = vi
      .fn()
      .mockResolvedValue({ ok: true, json: () => Promise.resolve(sampleNorthstar) });
    wrap(<DomainPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-northstar')).toBeInTheDocument());
    expect(screen.getByTestId('tab-northstar')).toHaveAttribute('aria-selected', 'true');
    await waitFor(() => expect(screen.getByTestId('monaco-editor')).toBeInTheDocument());
  });

  it('shows the loaded northstar markdown in the editor (not empty)', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    global.fetch = vi
      .fn()
      .mockResolvedValue({ ok: true, json: () => Promise.resolve(sampleNorthstar) });
    wrap(<DomainPage />, qc);
    await waitFor(() =>
      expect(screen.getByDisplayValue(/Drives investment data/)).toBeInTheDocument(),
    );
  });

  it('switches to Missions tab and shows missions', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    global.fetch = vi
      .fn()
      .mockResolvedValue({ ok: true, json: () => Promise.resolve(sampleNorthstar) });
    wrap(<DomainPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-missions')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('tab-missions'));
    await waitFor(() =>
      expect(screen.getByTestId('mission-row-mission-uuid-1')).toBeInTheDocument(),
    );
    expect(screen.getByText('Warehouse rebuild')).toBeInTheDocument();
  });
});
