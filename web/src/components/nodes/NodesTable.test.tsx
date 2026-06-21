import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { NodesTable } from './NodesTable';

const sampleNode = {
  id: 'node-uuid-1',
  node_name: 'node-0',
  hostname: 'node-0.local',
  status: 'online',
  trust_tier: 'admin',
  runtime_version: '0.7.0',
  tailscale_fqdn: 'node-0.example.ts.net',
  tailscale_ip: '100.64.0.1',
  last_heartbeat_at: '2026-06-09T10:00:00Z',
  owner_subject: 'merlin',
  registered_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-06-09T10:00:00Z',
  capabilities: [],
  capacity: {},
  labels: {},
};

describe('NodesTable', () => {
  it('shows loading state', () => {
    render(<NodesTable nodes={[]} isLoading onRowClick={vi.fn()} />);
    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('shows empty state when no nodes', () => {
    render(<NodesTable nodes={[]} isLoading={false} onRowClick={vi.fn()} />);
    expect(screen.getByTestId('empty-state')).toBeInTheDocument();
  });

  it('renders a row per node', () => {
    render(<NodesTable nodes={[sampleNode]} isLoading={false} onRowClick={vi.fn()} />);
    expect(screen.getByTestId('node-row-node-uuid-1')).toBeInTheDocument();
    expect(screen.getByText('node-0')).toBeInTheDocument();
    expect(screen.getByText('online')).toBeInTheDocument();
    expect(screen.getByText('0.7.0')).toBeInTheDocument();
  });

  it('calls onRowClick with node when row is clicked', () => {
    const onRowClick = vi.fn();
    render(<NodesTable nodes={[sampleNode]} isLoading={false} onRowClick={onRowClick} />);
    fireEvent.click(screen.getByTestId('node-row-node-uuid-1'));
    expect(onRowClick).toHaveBeenCalledWith(sampleNode);
  });
});
