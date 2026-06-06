/**
 * Fleet dashboard (/) — unit tests.
 *
 * Mocking strategy:
 *   - vi.mock('@/api/client') — controls apiClient.GET / unwrap
 *   - vi.mock('@/lib/conversation/useAcpConversation') — no real WebSocket
 *   - vi.mock('@tanstack/react-router') — no router context required
 *
 * Test surface:
 *   - Tabs render one-per-agent from the merged agent list (generic names AND aria-* names)
 *   - Tab label = agent.name as-is (no stripping)
 *   - No placeholder / disabled "not registered" tabs
 *   - Empty state when zero agents
 *   - Clicking a tab changes the active agent
 *   - Ctrl+N hotkey switches active tab by index
 *   - Not-attachable fallback (agent has no node_id in metadata)
 *   - ConversationView rendered for active, attachable agent
 *   - useAcpConversation called with correct nodeId + agentId
 *   - Loading / error states
 *   - Fleet summary counts
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

// Intentionally mix a generic name with an aria-* name to prove both are shown as-is.
const sampleCpAgents = [
  {
    id: 1,
    public_id: 'aria-engineer-f1a2b3c4',
    name: 'aria-engineer-f1a2b3c4',
    status: 'online',
    capabilities: 'mc-development',
    metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
    home_domain_id: 'dom-abc',
    current_domain_id: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-05-31T10:00:00Z',
  },
  {
    id: 2,
    public_id: 'worker-bot-99',
    name: 'worker-bot-99',
    status: 'offline',
    capabilities: 'batch-processing',
    metadata: JSON.stringify({ runtime: 'custom', node_id: 'kai' }),
    home_domain_id: 'dom-def',
    current_domain_id: null,
    created_at: '2026-01-02T00:00:00Z',
    updated_at: '2026-05-31T09:00:00Z',
  },
  {
    id: 3,
    public_id: 'analytics-agent-77',
    name: 'analytics-agent-77',
    status: 'online',
    capabilities: 'reporting',
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

  // ── Tab render — one per agent, label = name as-is ────────────────────────

  it('renders exactly one tab per registered agent', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    // Tabs keyed by public_id
    for (const agent of sampleCpAgents) {
      expect(screen.getByTestId(`tab-${agent.public_id}`)).toBeInTheDocument();
    }

    // Exactly 3 tabs — no extras, no placeholders
    const tabs = screen.getAllByRole('tab');
    expect(tabs).toHaveLength(3);
  });

  it('tab labels show agent name as-is (no stripping)', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    // aria-* name should show the full name, not a stripped version
    expect(screen.getByTestId('tab-aria-engineer-f1a2b3c4')).toHaveTextContent(
      'aria-engineer-f1a2b3c4',
    );
    // Generic name shown verbatim
    expect(screen.getByTestId('tab-worker-bot-99')).toHaveTextContent('worker-bot-99');
  });

  it('no tabs are disabled — no "not registered" placeholder tabs', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    const tabs = screen.getAllByRole('tab');
    for (const tab of tabs) {
      expect(tab).not.toBeDisabled();
    }
  });

  // ── Empty state ───────────────────────────────────────────────────────────

  it('shows empty state when no agents are registered', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce([]) // no cp agents
      .mockResolvedValueOnce([]) // no runtime nodes
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('empty-state')).toBeInTheDocument());
    expect(screen.getByText(/No agents registered/)).toBeInTheDocument();
  });

  // ── Tab switching via click ───────────────────────────────────────────────

  it('clicking a different tab changes the active agent', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    // First agent is default active; click the second
    fireEvent.click(screen.getByTestId('tab-worker-bot-99'));

    await waitFor(() =>
      expect(screen.getByTestId('tab-worker-bot-99')).toHaveAttribute('aria-selected', 'true'),
    );
    expect(screen.getByTestId('tab-aria-engineer-f1a2b3c4')).toHaveAttribute(
      'aria-selected',
      'false',
    );
  });

  // ── Tab switching via keyboard hotkey ─────────────────────────────────────

  it('Ctrl+2 hotkey switches to the second agent (index 1)', async () => {
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
      expect(screen.getByTestId('tab-worker-bot-99')).toHaveAttribute('aria-selected', 'true'),
    );
  });

  it('Ctrl+1 hotkey switches to the first agent (index 0)', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    // Switch to second agent first, then back via hotkey
    fireEvent.click(screen.getByTestId('tab-worker-bot-99'));
    await waitFor(() =>
      expect(screen.getByTestId('tab-worker-bot-99')).toHaveAttribute('aria-selected', 'true'),
    );

    fireEvent.keyDown(document, { key: '1', ctrlKey: true });

    await waitFor(() =>
      expect(screen.getByTestId('tab-aria-engineer-f1a2b3c4')).toHaveAttribute(
        'aria-selected',
        'true',
      ),
    );
  });

  // ── Not-attachable fallback ───────────────────────────────────────────────

  it('shows not-attachable fallback when the active agent has no node_id', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());

    // analytics-agent-77 has no node_id in metadata
    fireEvent.click(screen.getByTestId('tab-analytics-agent-77'));

    await waitFor(() =>
      expect(screen.getByTestId('agent-not-attachable-analytics-agent-77')).toBeInTheDocument(),
    );
  });

  // ── ConversationView rendered for active, attachable agent ────────────────

  it('renders ConversationView for the active agent with a valid node_id', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    // First agent (aria-engineer-f1a2b3c4) is active by default and has node_id=excalibur
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

    // Should be called with the first agent's nodeId and public_id
    expect(useAcpConversation as ReturnType<typeof vi.fn>).toHaveBeenCalledWith(
      'excalibur',
      'aria-engineer-f1a2b3c4',
    );
  });

  // ── Fleet summary ─────────────────────────────────────────────────────────

  it('shows fleet online/total summary when agents are loaded', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents) // aria-engineer=online, worker-bot=offline, analytics=online
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('fleet-summary')).toBeInTheDocument());

    // 2 online (aria-engineer + analytics-agent), 3 total
    const countEl = screen.getByTestId('fleet-online-count');
    expect(countEl).toHaveTextContent('2');
    expect(countEl).toHaveTextContent('3');
  });

  // ── Status badge ──────────────────────────────────────────────────────────

  it('shows per-agent status badge in the pane header', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() =>
      expect(screen.getByTestId('agent-status-badge-aria-engineer-f1a2b3c4')).toBeInTheDocument(),
    );
    expect(screen.getByTestId('agent-status-badge-aria-engineer-f1a2b3c4')).toHaveTextContent(
      'online',
    );
  });

  it('shows node id in the agent pane header when present', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleCpAgents)
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() =>
      expect(screen.getByTestId('agent-node-aria-engineer-f1a2b3c4')).toBeInTheDocument(),
    );
    expect(screen.getByTestId('agent-node-aria-engineer-f1a2b3c4')).toHaveTextContent('excalibur');
  });
});
