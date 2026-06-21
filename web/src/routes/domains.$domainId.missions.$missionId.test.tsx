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
    }: { to: string; children: React.ReactNode; style?: React.CSSProperties }) => (
      <a href={to} style={style}>
        {children}
      </a>
    ),
    useParams: vi.fn(() => ({ domainId: 'domain-uuid-1', missionId: 'mission-uuid-1' })),
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

import { MissionPage } from './domains.$domainId.missions.$missionId';

const sampleNodeDetail = {
  node_type: 'mission',
  node_id: 'mission-uuid-1',
  domain: null,
  mission: {
    id: 'mission-uuid-1',
    name: 'Warehouse rebuild',
    description: 'Rebuild warehouse data pipelines',
    domain_id: 'domain-uuid-1',
    status: 'in_progress',
    owners: 'my-agent-operator',
    tags: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-05-30T12:00:00Z',
  },
  tasks: [
    {
      id: 101,
      public_id: 'task-pub-101',
      mission_id: 'mission-uuid-1',
      title: 'Set up ingestion',
      description: 'Ingest OHLCV data',
      status: 'done',
      owner: 'my-agent-operator',
      contributors: '',
      created_at: '2026-01-01T00:00:00Z',
      updated_at: '2026-05-30T12:00:00Z',
    },
  ],
  missions: null,
  task: null,
};

const sampleBrief = {
  brief_md: '# Warehouse\n\nRebuild the data warehouse.',
  brief_version: 2,
  brief_modified_by: 'my-agent-operator',
  brief_modified_at: '2026-06-01T10:00:00Z',
};

function makeQC() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
  });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('MissionPage', () => {
  let qc: QueryClient;
  beforeEach(() => {
    qc = makeQC();
    vi.clearAllMocks();
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve(sampleBrief),
    });
  });
  afterEach(() => qc.clear());

  it('shows mission name after loading', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    wrap(<MissionPage />, qc);
    await waitFor(() => expect(screen.getByTestId('mission-header')).toBeInTheDocument());
    expect(screen.getByTestId('mission-header')).toHaveTextContent('Warehouse rebuild');
  });

  it('shows Brief tab as default with Monaco editor', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    wrap(<MissionPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-brief')).toBeInTheDocument());
    expect(screen.getByTestId('tab-brief')).toHaveAttribute('aria-selected', 'true');
    await waitFor(() => expect(screen.getByTestId('monaco-editor')).toBeInTheDocument());
  });

  it('shows the loaded brief markdown in the editor (not empty)', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    // global.fetch already resolves sampleBrief in beforeEach
    wrap(<MissionPage />, qc);
    await waitFor(() =>
      expect(screen.getByDisplayValue(/Rebuild the data warehouse/)).toBeInTheDocument(),
    );
  });

  it('switches to Tasks tab and shows tasks', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    wrap(<MissionPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-tasks')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('tab-tasks'));
    await waitFor(() => expect(screen.getByTestId('task-row-101')).toBeInTheDocument());
    expect(screen.getByText('Set up ingestion')).toBeInTheDocument();
  });

  it('opens TaskSlideOver when task row is clicked', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    wrap(<MissionPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-tasks')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('tab-tasks'));
    await waitFor(() => expect(screen.getByTestId('task-row-101')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('task-row-101'));
    expect(screen.getByTestId('slide-over')).toBeInTheDocument();
  });
});
