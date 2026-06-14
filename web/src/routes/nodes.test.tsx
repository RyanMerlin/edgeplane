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

const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

import { NodesIndexPage } from './nodes.index';

const sampleNode = {
  id: 'node-uuid-1',
  node_name: 'excalibur',
  hostname: 'excalibur.local',
  status: 'online',
  trust_tier: 'admin',
  runtime_version: '0.7.0',
  tailscale_fqdn: 'excalibur.example.ts.net',
  tailscale_ip: '100.64.0.1',
  last_heartbeat_at: '2026-06-09T10:00:00Z',
  owner_subject: 'merlin',
  registered_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-06-09T10:00:00Z',
  capabilities: [],
  capacity: {},
  labels: {},
};

function makeQC() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
  });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('NodesIndexPage', () => {
  let qc: QueryClient;
  beforeEach(() => {
    qc = makeQC();
    vi.clearAllMocks();
  });
  afterEach(() => qc.clear());

  it('shows loading state while nodes are fetching', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    wrap(<NodesIndexPage />, qc);
    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('renders node rows when data arrives', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({ data: [sampleNode] });
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue([sampleNode]);
    wrap(<NodesIndexPage />, qc);
    await waitFor(() => expect(screen.getByTestId('node-row-node-uuid-1')).toBeInTheDocument());
    expect(screen.getByText('excalibur')).toBeInTheDocument();
  });
});
