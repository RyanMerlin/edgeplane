/**
 * Fleet dashboard (/) — unit tests.
 *
 * The Fleet route hosts two sub-views toggled in-page: the Agents table
 * (default) and the per-agent Conversations console. Tests that exercise the
 * conversation console switch into it via showConsole() first.
 *
 * Mocking strategy:
 *   - vi.mock('@/api/client') — controls apiClient.GET / unwrap
 *   - vi.mock('@/lib/conversation/useAcpConversation') — no real WebSocket
 *   - vi.mock('@tanstack/react-router') — no router context required
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
    // no node_id in metadata — "not attachable" case
    capabilities: 'reporting',
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

/** Mock both queries with the standard sample-cp-agents / no-mesh response. */
async function mockSampleAgents() {
  const { apiClient, unwrap } = await import('@/api/client');
  (apiClient.GET as ReturnType<typeof vi.fn>)
    .mockResolvedValueOnce(sampleCpAgents)
    .mockResolvedValueOnce([])
    .mockResolvedValue([]);
  (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));
}

/** Switch from the default Agents-table view into the Conversations console. */
async function showConsole() {
  await waitFor(() => expect(screen.getByTestId('fleet-view-toggle')).toBeInTheDocument());
  fireEvent.click(screen.getByTestId('fleet-view-console'));
  await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());
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

  // ── Loading / error states ──────────────────────────────────────────────────

  it('shows loading state while both queries are pending', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));

    renderWith(<FleetDashboard />, queryClient);

    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('shows error state when both queries fail', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (unwrap as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('Network error'));
    (apiClient.GET as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('Network error'));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('error-state')).toBeInTheDocument());
    expect(screen.getByText(/Failed to load fleet/)).toBeInTheDocument();
  });

  // ── Default view + toggle ─────────────────────────────────────────────────

  it('defaults to the Agents table view and exposes a Conversations|Agents toggle', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('fleet-view-toggle')).toBeInTheDocument());
    expect(screen.getByTestId('agents-table')).toBeInTheDocument();
    expect(screen.getByTestId('fleet-view-table')).toHaveAttribute('aria-selected', 'true');
  });

  it('clicking an agent row navigates to its detail route', async () => {
    const navigate = vi.fn();
    const router = await import('@tanstack/react-router');
    (router.useNavigate as ReturnType<typeof vi.fn>).mockReturnValue(navigate);
    await mockSampleAgents();

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('agents-table')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('agent-row-aria-engineer-f1a2b3c4'));

    expect(navigate).toHaveBeenCalledWith({
      to: '/agents/$agentId',
      params: { agentId: 'aria-engineer-f1a2b3c4' },
    });
  });

  it('switching to the Conversations view shows per-agent tabs instead of the table', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('agents-table')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('fleet-view-console'));

    await waitFor(() => expect(screen.getByTestId('profile-tabs')).toBeInTheDocument());
    expect(screen.queryByTestId('agents-table')).not.toBeInTheDocument();
  });

  // ── Conversations console — tab render ────────────────────────────────────

  it('renders exactly one tab per registered agent', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    for (const agent of sampleCpAgents) {
      expect(screen.getByTestId(`tab-${agent.public_id}`)).toBeInTheDocument();
    }

    // Exactly 3 agent tabs — scope to the agent tablist; the view toggle is separate.
    const tabs = within(screen.getByTestId('profile-tabs')).getAllByRole('tab');
    expect(tabs).toHaveLength(3);
  });

  it('tab labels show agent name as-is (no stripping)', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    expect(screen.getByTestId('tab-aria-engineer-f1a2b3c4')).toHaveTextContent(
      'aria-engineer-f1a2b3c4',
    );
    expect(screen.getByTestId('tab-worker-bot-99')).toHaveTextContent('worker-bot-99');
  });

  it('no agent tabs are disabled — no "not registered" placeholders', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    const tabs = within(screen.getByTestId('profile-tabs')).getAllByRole('tab');
    for (const tab of tabs) {
      expect(tab).not.toBeDisabled();
    }
  });

  // ── Empty state ───────────────────────────────────────────────────────────

  it('shows empty state when no agents are registered', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([])
      .mockResolvedValue([]);
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation((p: unknown) => Promise.resolve(p));

    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('empty-state')).toBeInTheDocument());
    expect(screen.getByText(/No agents registered/)).toBeInTheDocument();
  });

  // ── Conversations console — tab switching ─────────────────────────────────

  it('clicking a different tab changes the active agent', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    fireEvent.click(screen.getByTestId('tab-worker-bot-99'));

    await waitFor(() =>
      expect(screen.getByTestId('tab-worker-bot-99')).toHaveAttribute('aria-selected', 'true'),
    );
    expect(screen.getByTestId('tab-aria-engineer-f1a2b3c4')).toHaveAttribute(
      'aria-selected',
      'false',
    );
  });

  it('Ctrl+2 hotkey switches to the second agent (index 1)', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    fireEvent.keyDown(document, { key: '2', ctrlKey: true });

    await waitFor(() =>
      expect(screen.getByTestId('tab-worker-bot-99')).toHaveAttribute('aria-selected', 'true'),
    );
  });

  it('Ctrl+1 hotkey switches to the first agent (index 0)', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

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
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    // analytics-agent-77 has no node_id in metadata
    fireEvent.click(screen.getByTestId('tab-analytics-agent-77'));

    await waitFor(() =>
      expect(screen.getByTestId('agent-not-attachable-analytics-agent-77')).toBeInTheDocument(),
    );
  });

  // ── ConversationView rendered for active, attachable agent ────────────────

  it('renders ConversationView for the active agent with a valid node_id', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    // First agent (aria-engineer-f1a2b3c4) is active by default and has node_id=excalibur
    await waitFor(() => expect(screen.getByTestId('conversation-view')).toBeInTheDocument());
  });

  it('calls useAcpConversation with the correct nodeId and agentId', async () => {
    const { useAcpConversation } = await import('@/lib/conversation/useAcpConversation');
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    await waitFor(() => expect(useAcpConversation as ReturnType<typeof vi.fn>).toHaveBeenCalled());

    expect(useAcpConversation as ReturnType<typeof vi.fn>).toHaveBeenCalledWith(
      'excalibur',
      'aria-engineer-f1a2b3c4',
    );
  });

  // ── Fleet summary ─────────────────────────────────────────────────────────

  it('shows fleet online/total summary when agents are loaded', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);

    await waitFor(() => expect(screen.getByTestId('fleet-summary')).toBeInTheDocument());

    // 2 online (aria-engineer + analytics-agent), 3 total
    const countEl = screen.getByTestId('fleet-online-count');
    expect(countEl).toHaveTextContent('2');
    expect(countEl).toHaveTextContent('3');
  });

  // ── Status badge / node id (console pane header) ──────────────────────────

  it('shows per-agent status badge in the pane header', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    await waitFor(() =>
      expect(screen.getByTestId('agent-status-badge-aria-engineer-f1a2b3c4')).toBeInTheDocument(),
    );
    expect(screen.getByTestId('agent-status-badge-aria-engineer-f1a2b3c4')).toHaveTextContent(
      'online',
    );
  });

  it('shows node id in the agent pane header when present', async () => {
    await mockSampleAgents();
    renderWith(<FleetDashboard />, queryClient);
    await showConsole();

    await waitFor(() =>
      expect(screen.getByTestId('agent-node-aria-engineer-f1a2b3c4')).toBeInTheDocument(),
    );
    expect(screen.getByTestId('agent-node-aria-engineer-f1a2b3c4')).toHaveTextContent('excalibur');
  });
});
