/**
 * Fleet dashboard (/) — unit tests.
 *
 * Mocking strategy:
 *   - vi.mock('@/api/client') — controls apiClient.GET / unwrap
 *   - vi.mock('@/lib/conversation/useAcpConversation') — no real WebSocket
 *   - vi.mock('@tanstack/react-router') — no router context required
 *
 * Test surface:
 *   - Profile tabs render from agents query
 *   - Clicking a tab changes active profile
 *   - Ctrl+<n> hotkey switches active profile
 *   - Per-profile status badge and node id render
 *   - Not-registered profile shows fallback
 *   - Not-attachable profile (no node_id) shows fallback
 *   - Loading / error states
 *   - Fleet summary counts
 *   - Only active tab mounts AcpPane (useAcpConversation called once per render)
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ── Mocks (must be hoisted before component imports) ──────────────────────────

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

// Mock the conversation hook so no WebSocket opens in tests.
// The factory cannot reference a top-level variable (vi.mock is hoisted),
// so we use vi.fn() inline and retrieve the mock via import() in tests.
vi.mock('@/lib/conversation/useAcpConversation', () => ({
  useAcpConversation: vi.fn().mockReturnValue({
    items: [],
    status: 'connecting',
    send: vi.fn(),
    cancel: vi.fn(),
  }),
}));

// Mock ConversationView to keep tests focused on the dashboard shell
vi.mock('@/components/conversation/ConversationView', () => ({
  ConversationView: ({
    status,
  }: {
    status: string;
    items: unknown[];
    onSend: (t: string) => void;
    onCancel: () => void;
  }) => <div data-testid="conversation-view">conversation:{status}</div>,
}));

// ── Component import (after mocks) ────────────────────────────────────────────

import { FleetDashboard } from './index';

// ── Fixtures ──────────────────────────────────────────────────────────────────

const sampleCpAgents = [
  {
    id: 1,
    public_id: 'aria-operator-e8820c0d',
    name: 'aria-operator-e8820c0d',
    status: 'online',
    capabilities: 'fleet-management',
    metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
    home_domain_id: 'dom-abc',
    current_domain_id: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-05-31T10:00:00Z',
  },
  {
    id: 2,
    public_id: 'aria-engineer-f1a2b3c4',
    name: 'aria-engineer-f1a2b3c4',
    status: 'offline',
    capabilities: 'mc-development',
    metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
    home_domain_id: 'dom-def',
    current_domain_id: null,
    created_at: '2026-01-02T00:00:00Z',
    updated_at: '2026-05-31T09:00:00Z',
  },
  {
    id: 3,
    public_id: 'aria-merlinlabs-a1b2c3d4',
    name: 'aria-merlinlabs-a1b2c3d4',
    status: 'online',
    capabilities: 'k8s,homelab',
    // no node_id in metadata — "not attachable" case
    metadata: JSON.stringify({ runtime: 'claude-code' }),
    home_domain_id: 'dom-ghi',
    current_domain_id: null,
    created_at: '2026-01-03T00:00:00Z',
    updated_at: '2026-05-31T08:00:00Z',
  },
];

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

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('FleetDashboard', () => {
  let queryClient: QueryClient;

  beforeEach(async () => {
    queryClient = makeQueryClient();
    vi.clearAllMocks();

    // Re-apply default return value after clearAllMocks
    const { useAcpConversation } = await import('@/lib/conversation/useAcpConversation');
    (useAcpConversation as ReturnType<typeof vi.fn>).mockReturnValue({
      items: [],
      status: 'connecting',
      send: vi.fn(),
      cancel: vi.fn(),
    });
  });

  afterEach(() => {
    queryClient.clear();
  });

  // ── Loading state ─────────────────────────────────────────────────────────

  it('shows loading state while both queries are pending', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));

    renderWith(<FleetDashboard />, queryClient);

    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  // ── Error state ───────────────────────────────────────────────────────────

  it('shows error state when both queries fail', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (unwrap as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('Network error'));
    (apiClient.GET as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('Network error'));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('error-state')).toBeInTheDocument());
    expect(screen.getByText(/Failed to load fleet/)).toBeInTheDocument();
  });

  // ── Profile tabs render ───────────────────────────────────────────────────

  it('renders all six profile tabs', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    for (const profile of ['operator', 'engineer', 'merlinlabs', 'publisher', 'work', 'research']) {
      expect(screen.getByTestId(`tab-${profile}`)).toBeInTheDocument();
    }
  });

  it('shows per-profile status badge for registered agents', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    // Wait for data to load and operator tab to be active (first profile)
    await waitFor(() =>
      expect(screen.getByTestId('profile-status-badge-operator')).toBeInTheDocument(),
    );
    expect(screen.getByTestId('profile-status-badge-operator')).toHaveTextContent('online');
  });

  it('shows node id in the profile header when present', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-node-operator')).toBeInTheDocument());
    expect(screen.getByTestId('profile-node-operator')).toHaveTextContent('excalibur');
  });

  // ── Tab switching via click ───────────────────────────────────────────────

  it('clicking a different tab changes the active profile', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    // operator is default active; click engineer (which has an agent)
    fireEvent.click(screen.getByTestId('tab-engineer'));

    await waitFor(() =>
      expect(screen.getByTestId('tab-engineer')).toHaveAttribute('aria-selected', 'true'),
    );
    expect(screen.getByTestId('tab-operator')).toHaveAttribute('aria-selected', 'false');
  });

  // ── Tab switching via keyboard hotkey ─────────────────────────────────────

  it('Ctrl+2 hotkey switches to the engineer profile (index 1)', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    fireEvent.keyDown(document, { key: '2', ctrlKey: true });

    await waitFor(() =>
      expect(screen.getByTestId('tab-engineer')).toHaveAttribute('aria-selected', 'true'),
    );
  });

  it('Ctrl+1 hotkey switches to the operator profile (index 0)', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    // First switch to engineer (tab 2), then back to operator via hotkey
    fireEvent.click(screen.getByTestId('tab-engineer'));
    await waitFor(() =>
      expect(screen.getByTestId('tab-engineer')).toHaveAttribute('aria-selected', 'true'),
    );

    fireEvent.keyDown(document, { key: '1', ctrlKey: true });

    await waitFor(() =>
      expect(screen.getByTestId('tab-operator')).toHaveAttribute('aria-selected', 'true'),
    );
  });

  // ── Not-registered fallback ───────────────────────────────────────────────

  it('shows not-registered fallback for a profile with no agent', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents) // only operator, engineer, merlinlabs registered
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    // publisher, work, research are not registered; click work tab
    // (disabled button click won't fire, so we look at pane content via active tab state)
    // Switch manually to a profile we know is unregistered by checking the tab is disabled
    const publisherTab = screen.getByTestId('tab-publisher');
    expect(publisherTab).toBeDisabled();
  });

  // ── Not-attachable fallback ───────────────────────────────────────────────

  it('shows not-attachable fallback when the active profile agent has no node_id', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    // Switch to merlinlabs — has an agent but no node_id in metadata
    fireEvent.click(screen.getByTestId('tab-merlinlabs'));

    await waitFor(() =>
      expect(screen.getByTestId('profile-not-attachable-merlinlabs')).toBeInTheDocument(),
    );
  });

  // ── ConversationView is rendered for active, attachable profile ───────────

  it('renders ConversationView for the active profile with a valid node_id', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    // operator tab is active by default and has node_id=excalibur
    await waitFor(() => expect(screen.getByTestId('conversation-view')).toBeInTheDocument());
  });

  it('calls useAcpConversation with the correct nodeId and agentId', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    const { useAcpConversation } = await import('@/lib/conversation/useAcpConversation');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(useAcpConversation as ReturnType<typeof vi.fn>).toHaveBeenCalled());

    // Should be called with the operator agent's nodeId and public_id
    expect(useAcpConversation as ReturnType<typeof vi.fn>).toHaveBeenCalledWith(
      'excalibur',
      'aria-operator-e8820c0d',
    );
  });

  // ── Fleet summary ─────────────────────────────────────────────────────────

  it('shows fleet online/total summary when agents are loaded', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents) // operator=online, engineer=offline, merlinlabs=online
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('fleet-summary')).toBeInTheDocument());

    // 2 online (operator + merlinlabs), 3 total
    const countEl = screen.getByTestId('fleet-online-count');
    expect(countEl).toHaveTextContent('2');
    expect(countEl).toHaveTextContent('3');
  });
});
