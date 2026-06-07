/**
 * AgentsTable — unit tests.
 *
 * Migrated from the presentation half of the former routes/agents.test.tsx.
 * AgentsTable is pure presentation, so the merged agent list + states are
 * passed as props (no apiClient / QueryClient / router context). The merge
 * logic itself is covered by lib/useMergedAgents.test.ts.
 */

import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { MergedAgent } from '@/lib/useMergedAgents';
import { AgentsTable } from './AgentsTable';

// ── Fixtures ──────────────────────────────────────────────────────────────────

const agents: MergedAgent[] = [
  {
    public_id: 'aria-operator-e8820c0d',
    name: 'aria-operator',
    status: 'online',
    capabilities: 'fleet-management,code-editing',
    source: 'controlplane',
    metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
    updated_at: '2026-05-31T10:00:00Z',
  },
  {
    public_id: 'aria-research-f1a2b3c4',
    name: 'aria-research',
    status: 'offline',
    capabilities: 'research,analysis',
    source: 'controlplane',
    metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
    updated_at: '2026-05-31T09:00:00Z',
  },
];

const noop = () => {};

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('AgentsTable', () => {
  it('shows loading state', () => {
    render(
      <AgentsTable agents={[]} isLoading={true} isError={false} error={null} onRowClick={noop} />,
    );
    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('shows error state with the error message', () => {
    render(
      <AgentsTable
        agents={[]}
        isLoading={false}
        isError={true}
        error={new Error('Unauthorized')}
        onRowClick={noop}
      />,
    );
    expect(screen.getByTestId('error-state')).toBeInTheDocument();
    expect(screen.getByText(/Failed to load agents/)).toBeInTheDocument();
    expect(screen.getByText(/Unauthorized/)).toBeInTheDocument();
  });

  it('shows empty state when there are no agents', () => {
    render(
      <AgentsTable agents={[]} isLoading={false} isError={false} error={null} onRowClick={noop} />,
    );
    expect(screen.getByTestId('empty-state')).toBeInTheDocument();
    expect(screen.getByText('No agents registered')).toBeInTheDocument();
  });

  it('renders a row per agent with name + status', () => {
    render(
      <AgentsTable
        agents={agents}
        isLoading={false}
        isError={false}
        error={null}
        onRowClick={noop}
      />,
    );
    expect(screen.getByTestId('agents-table')).toBeInTheDocument();
    expect(screen.getByText('aria-operator')).toBeInTheDocument();
    expect(screen.getByText('aria-research')).toBeInTheDocument();
    expect(screen.getByText('online')).toBeInTheDocument();
    expect(screen.getByText('offline')).toBeInTheDocument();
    expect(screen.getByTestId('agent-row-aria-operator-e8820c0d')).toBeInTheDocument();
    expect(screen.getByTestId('agent-row-aria-research-f1a2b3c4')).toBeInTheDocument();
  });

  it('renders rows alphabetically by name regardless of input order', () => {
    const reversed = [agents[1], agents[0]]; // research, operator
    render(
      <AgentsTable
        agents={reversed}
        isLoading={false}
        isError={false}
        error={null}
        onRowClick={noop}
      />,
    );
    const rows = screen.getAllByTestId(/^agent-row-/);
    expect(rows[0]).toHaveAttribute('data-testid', 'agent-row-aria-operator-e8820c0d');
    expect(rows[1]).toHaveAttribute('data-testid', 'agent-row-aria-research-f1a2b3c4');
  });

  it('shows Node + Runtime from agent metadata when present', () => {
    render(
      <AgentsTable
        agents={[agents[0]]}
        isLoading={false}
        isError={false}
        error={null}
        onRowClick={noop}
      />,
    );
    const row = screen.getByTestId('agent-row-aria-operator-e8820c0d');
    expect(within(row).getByText('claude-code')).toBeInTheDocument(); // runtime
    expect(within(row).getByText('excalibur')).toBeInTheDocument(); // node_id
  });

  it('resolves Node + Runtime from mesh topology when agent metadata is empty', () => {
    const meshOnly: MergedAgent = {
      public_id: 'aria-work-aa11bb22',
      name: 'aria-work',
      status: 'online',
      capabilities: '',
      source: 'both',
      metadata: '{}',
      runtime_kind: 'claude_acp',
      node_name: 'excalibur',
      last_heartbeat_at: '2026-05-31T12:00:00Z',
    };
    render(
      <AgentsTable
        agents={[meshOnly]}
        isLoading={false}
        isError={false}
        error={null}
        onRowClick={noop}
      />,
    );
    const row = screen.getByTestId('agent-row-aria-work-aa11bb22');
    expect(within(row).getByText('excalibur')).toBeInTheDocument(); // node from topology
    expect(within(row).getByText('claude_acp')).toBeInTheDocument(); // runtime_kind
  });

  it('calls onRowClick with the agent when a row is clicked', () => {
    const onRowClick = vi.fn();
    render(
      <AgentsTable
        agents={agents}
        isLoading={false}
        isError={false}
        error={null}
        onRowClick={onRowClick}
      />,
    );
    fireEvent.click(screen.getByTestId('agent-row-aria-operator-e8820c0d'));
    expect(onRowClick).toHaveBeenCalledOnce();
    expect(onRowClick.mock.calls[0][0]).toMatchObject({ public_id: 'aria-operator-e8820c0d' });
  });
});
