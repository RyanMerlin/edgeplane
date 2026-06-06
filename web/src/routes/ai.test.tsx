/**
 * AI console route — screen tests.
 *
 * Mocking strategy: same pattern as agents.test.tsx.
 *   - vi.mock('@/api/client') controls all network calls.
 *   - vi.mock('@/lib/conversation/useRestConversation') controls session transport.
 *   - TanStack Router mocked to remove router context requirement.
 *   - QueryClient with retry:false for deterministic test behaviour.
 *
 * Tests:
 *   1. Sessions list renders in the sidebar
 *   2. Selecting a session renders its conversation items
 *   3. Approve button calls the approve handler
 *   4. Composer sends a turn (calls send)
 *   5. Backend unavailable (sessions query error) shows the unavailable state
 *   6. New Session button opens the modal
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
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
      useParams: () => ({}),
    }),
    Link: ({
      to,
      children,
      className,
    }: {
      to: string;
      children: React.ReactNode;
      className?: string;
    }) => (
      <a href={to} className={className}>
        {children}
      </a>
    ),
    useNavigate: () => vi.fn(),
  };
});

vi.mock('@/api/client', () => ({
  apiClient: {
    GET: vi.fn(),
    POST: vi.fn(),
    use: vi.fn(),
  },
  unwrap: vi.fn(),
}));

// Factory is self-contained — no references to outer-scope variables.
// The spies are accessible via the mocked module after import.
vi.mock('@/lib/conversation/useRestConversation', () => ({
  useRestConversation: vi.fn().mockReturnValue({
    items: [],
    status: 'open',
    send: vi.fn().mockResolvedValue(undefined),
    approve: vi.fn().mockResolvedValue(undefined),
    reject: vi.fn().mockResolvedValue(undefined),
  }),
}));

// ── Import components and mocked modules AFTER vi.mock calls ──────────────────

import { apiClient, unwrap } from '@/api/client';
import { useRestConversation } from '@/lib/conversation/useRestConversation';
import { AiConsolePage } from './ai';

// ── Fixtures ──────────────────────────────────────────────────────────────────

const sampleSessions = [
  {
    id: 'ais_abc123',
    title: 'Test session A',
    status: 'active',
    runtime_kind: 'opencode',
    owner_subject: 'test@example.com',
    turns: [],
    events: [],
    pending_actions: [],
    created_at: '2024-01-01T00:00:00.000000',
    updated_at: '2024-01-01T00:00:00.000000',
  },
  {
    id: 'ais_def456',
    title: 'Test session B',
    status: 'completed',
    runtime_kind: 'claude_code',
    owner_subject: 'test@example.com',
    turns: [],
    events: [],
    pending_actions: [],
    created_at: '2024-01-01T00:01:00.000000',
    updated_at: '2024-01-01T00:01:00.000000',
  },
];

const sampleCapabilities = [
  {
    runtime_kind: 'opencode',
    display_name: 'OpenCode',
    icon_slug: 'opencode',
    supports_streaming: true,
    supports_file_workspace: true,
    supports_tool_interception: true,
    supports_skill_packs: false,
    supports_session_resume: false,
    max_context_tokens: 100000,
  },
];

// ── Test helpers ──────────────────────────────────────────────────────────────

function makeQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
}

function renderPage(qc: QueryClient) {
  return render(
    <QueryClientProvider client={qc}>
      <AiConsolePage />
    </QueryClientProvider>,
  );
}

/** Set up the mocked apiClient + unwrap to return sessions for GET calls. */
function mockSessionsList(sessions: typeof sampleSessions = sampleSessions) {
  (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
    data: sessions,
    error: undefined,
    response: { ok: true, status: 200 },
  });
  (unwrap as ReturnType<typeof vi.fn>).mockImplementation(
    async (promise: Promise<{ data?: unknown }>) => {
      const { data } = await promise;
      return data;
    },
  );
}

/** Set up the mocked transport to return specific items. */
function mockTransport(
  items: import('@/lib/conversation/types').ConversationItem[],
  send = vi.fn().mockResolvedValue(undefined),
  approve = vi.fn().mockResolvedValue(undefined),
  reject = vi.fn().mockResolvedValue(undefined),
) {
  (useRestConversation as ReturnType<typeof vi.fn>).mockReturnValue({
    items,
    status: 'open',
    send,
    approve,
    reject,
  });
  return { send, approve, reject };
}

// ── Tests ──────────────────────────────────────────────────────────────────────

describe('AiConsolePage', () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    queryClient = makeQueryClient();
    vi.clearAllMocks();
    // Default transport: empty items
    mockTransport([]);
  });

  afterEach(() => {
    queryClient.clear();
  });

  // 1. Sessions list renders in the sidebar
  it('sessions list renders in the sidebar', async () => {
    mockSessionsList();
    renderPage(queryClient);

    await waitFor(() => {
      expect(screen.getByTestId('session-row-ais_abc123')).toBeInTheDocument();
    });
    expect(screen.getByTestId('session-row-ais_def456')).toBeInTheDocument();
  });

  // 2. Selecting a session renders its conversation view
  it('clicking a session row renders its conversation view', async () => {
    mockSessionsList();
    renderPage(queryClient);

    await waitFor(() => {
      expect(screen.getByTestId('session-row-ais_def456')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId('session-row-ais_def456'));

    await waitFor(() => {
      expect(screen.getByTestId('conversation-view')).toBeInTheDocument();
    });
  });

  // 3. Approve button calls the approve handler
  it('Approve button calls the approve handler with the action id', async () => {
    const approvalItem: import('@/lib/conversation/types').ConversationItem = {
      kind: 'approval',
      id: 'action-act1',
      tool: 'bash',
      args: { cmd: 'ls' },
      reason: 'Write operation',
      status: 'pending',
    };
    const { approve } = mockTransport([approvalItem]);
    mockSessionsList();
    renderPage(queryClient);

    await waitFor(() => {
      expect(screen.getByTestId('approval-prompt-action-act1')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId('approval-approve-action-act1'));

    await waitFor(() => {
      expect(approve).toHaveBeenCalledWith('act1');
    });
  });

  // 4. Composer sends a turn
  it('composer sends a turn when the user types and clicks Send', async () => {
    const { send } = mockTransport([]);
    mockSessionsList();
    renderPage(queryClient);

    await waitFor(() => {
      expect(screen.getByTestId('conversation-view')).toBeInTheDocument();
    });

    const textarea = screen.getByTestId('composer-textarea');
    fireEvent.change(textarea, { target: { value: 'List missions' } });
    fireEvent.click(screen.getByTestId('composer-send'));

    await waitFor(() => {
      expect(send).toHaveBeenCalledWith('List missions');
    });
  });

  // 5. Backend unavailable state
  it('shows the unavailable state when sessions query errors', async () => {
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
      data: undefined,
      error: { error: 'AI backend not configured' },
      response: { ok: false, status: 503 },
    });
    (unwrap as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('AI backend not configured'));

    renderPage(queryClient);

    await waitFor(() => {
      expect(screen.getByTestId('ai-unavailable')).toBeInTheDocument();
    });
  });

  // 6. New Session button opens the modal
  it('New Session button opens the create-session modal', async () => {
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
      data: sampleCapabilities,
      error: undefined,
      response: { ok: true, status: 200 },
    });
    (unwrap as ReturnType<typeof vi.fn>).mockImplementation(
      async (promise: Promise<{ data?: unknown }>) => {
        const { data } = await promise;
        return data;
      },
    );

    renderPage(queryClient);

    fireEvent.click(screen.getByTestId('new-session-btn'));

    expect(screen.getByTestId('new-session-modal')).toBeInTheDocument();
  });
});
