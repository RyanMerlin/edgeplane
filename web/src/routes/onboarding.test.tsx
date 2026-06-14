/**
 * Onboarding screen — unit tests.
 *
 * Mocking strategy: vi.mock('@/api/client') replaces the openapi-fetch
 * singleton with a controlled mock (same pattern as governance.test.tsx).
 * No network calls; each test controls what `apiClient.GET` returns.
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
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

import { OnboardingPage } from './onboarding';

// ── Sample fixture ────────────────────────────────────────────────────────────

const sampleManifest = {
  name: 'EdgePlane Dev',
  version: '0.12.0',
  integration_contract_version: '1',
  generated_for_base_url: 'http://localhost:8008',
  agent_configs: {},
  automation: { config_generator_script: 'echo ok' },
  bootstrap: {
    local_script: 'sh bootstrap.sh',
    remote_script: 'curl | sh',
    step_1: 'Install edgeplaned',
    step_2: 'Run edgeplaned enroll',
  },
  endpoints: {
    explorer_tree: '/api/explorer/tree',
    governance_active: '/api/governance/policy/active',
    health: '/api/health',
    mcp_call: '/api/mcp/call',
    mcp_health: '/api/mcp/health',
    mcp_tools: '/api/mcp/tools',
    openapi: '/api/openapi.json',
    ui: '/ui',
  },
  ep_serve_mcp_server: {
    name: 'edgeplane-serve',
    command: 'edgeplane',
    args: ['mcp', 'serve'],
    env: {},
  },
  mcp_defaults: {
    endpoint_candidates: ['http://edgeplane:8008'],
    healthcheck_path: '/api/health',
    protocol_version: '2024-11-05',
    startup_timeout_sec: 10,
    tool_timeout_sec: 30,
  },
  mcp_server: {
    name: 'edgeplane',
    command: 'edgeplane',
    args: ['mcp', 'serve'],
    env: { EP_AGENT_TOKEN: '<token>' },
  },
  notes: ['Use EP_AGENT_TOKEN for authentication.'],
};

// ── Test helper ───────────────────────────────────────────────────────────────

function makeQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
}

function renderOnboarding(qc: QueryClient) {
  return render(
    <QueryClientProvider client={qc}>
      <OnboardingPage />
    </QueryClientProvider>,
  );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('OnboardingPage', () => {
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

  it('shows loading state while query is pending', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));

    renderOnboarding(queryClient);

    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('renders manifest JSON from a sample OnboardingManifest', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleManifest);

    renderOnboarding(queryClient);

    // Manifest preview pane should contain the JSON
    await waitFor(() => expect(screen.getByTestId('manifest-json')).toBeInTheDocument());

    // Key fields present in the rendered JSON (pre block)
    const manifestPre = screen.getByTestId('manifest-json');
    expect(manifestPre.textContent).toContain('EdgePlane Dev');
    expect(manifestPre.textContent).toContain('0.12.0');

    // Instance meta block in the configuration pane
    const nameLabels = screen.getAllByText('EdgePlane Dev');
    expect(nameLabels.length).toBeGreaterThanOrEqual(1);
  });

  it('shows error state when query fails', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    const err = Object.assign(new Error('Unauthorized'), { status: 401 });
    (apiClient.GET as ReturnType<typeof vi.fn>).mockRejectedValue(err);
    // unwrap must reject too — `Promise.resolve(rejectedPromise)` propagates the
    // rejection, but explicitly mocking ensures clarity in test isolation.
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation(() => Promise.reject(err));

    renderOnboarding(queryClient);

    await waitFor(() => expect(screen.getByTestId('error-state')).toBeInTheDocument(), {
      timeout: 3000,
    });
    expect(screen.getByText(/Failed to load manifest/)).toBeInTheDocument();
  });

  it('shows empty state when data resolves to null', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(null);

    renderOnboarding(queryClient);

    await waitFor(() => expect(screen.getByTestId('empty-state')).toBeInTheDocument());
    expect(screen.getByText(/No manifest yet/)).toBeInTheDocument();
  });

  it('renders endpoint input and manifest URL', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(null);

    renderOnboarding(queryClient);

    // Endpoint input field should exist
    await waitFor(() => expect(screen.getByTestId('endpoint-input')).toBeInTheDocument());

    // Manifest URL should be derived from endpoint
    expect(screen.getByTestId('manifest-url')).toBeInTheDocument();
  });

  it('renders regenerate and copy buttons', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(null);

    renderOnboarding(queryClient);

    expect(screen.getByTestId('regenerate-btn')).toBeInTheDocument();
    expect(screen.getByTestId('copy-btn')).toBeInTheDocument();
  });
});
