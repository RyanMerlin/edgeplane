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
    useParams: vi.fn(() => ({ nodeId: 'node-uuid-1' })),
    Link: ({
      to,
      children,
      style,
    }: { to: string; children: React.ReactNode; style?: React.CSSProperties }) => (
      <a href={to} style={style}>
        {children}
      </a>
    ),
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

import { NodeDetailPage } from './nodes.$nodeId';

const sampleNode = {
  id: 'node-uuid-1',
  node_name: 'excalibur',
  hostname: 'excalibur.local',
  status: 'online',
  trust_tier: 'admin',
  runtime_version: '0.7.0',
  tailscale_fqdn: 'excalibur.hartley-neon.ts.net',
  tailscale_ip: '100.64.0.1',
  last_heartbeat_at: '2026-06-09T10:00:00Z',
  owner_subject: 'merlin',
  registered_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-06-09T10:00:00Z',
  capabilities: ['acp', 'mesh'],
  capacity: { cpu: 16, memory_gb: 64 },
  labels: { env: 'prod' },
};

const sampleAgent = {
  id: 1,
  public_id: 'aria-operator-e8820c0d',
  name: 'aria-operator',
  status: 'online',
  capabilities: 'fleet-management',
  metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
  home_domain_id: 'dom-abc',
  current_domain_id: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-06-09T10:00:00Z',
};

function makeQC() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
  });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('NodeDetailPage', () => {
  let qc: QueryClient;
  beforeEach(() => {
    qc = makeQC();
    vi.clearAllMocks();
    global.fetch = vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(sampleNode) });
  });
  afterEach(() => qc.clear());

  it('shows loading state while fetching', async () => {
    global.fetch = vi.fn().mockReturnValue(new Promise(() => {}));
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    (unwrap as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    wrap(<NodeDetailPage />, qc);
    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('renders node hostname and status', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({ data: [sampleAgent] });
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue([sampleAgent]);
    wrap(<NodeDetailPage />, qc);
    await waitFor(() => expect(screen.getByTestId('node-detail-header')).toBeInTheDocument());
    expect(screen.getAllByText('excalibur').length).toBeGreaterThan(0);
    expect(screen.getAllByText('online').length).toBeGreaterThan(0);
    expect(screen.getByText('v0.7.0')).toBeInTheDocument();
  });

  it('shows agents assigned to this node', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({ data: [sampleAgent] });
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue([sampleAgent]);
    wrap(<NodeDetailPage />, qc);
    await waitFor(() => expect(screen.getByTestId('node-agents-section')).toBeInTheDocument());
    expect(screen.getByText('aria-operator')).toBeInTheDocument();
  });
});
