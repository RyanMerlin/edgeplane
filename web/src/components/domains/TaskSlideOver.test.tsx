import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { TaskSlideOver } from './TaskSlideOver';

const sampleTask = {
  id: 101,
  public_id: 'task-pub-101',
  mission_id: 'mission-uuid-1',
  title: 'Implement auth',
  description: 'Set up OIDC authentication',
  status: 'done',
  owner: 'my-agent-operator',
  contributors: '',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-05-30T12:00:00Z',
};

describe('TaskSlideOver', () => {
  it('renders nothing when closed', () => {
    const { container } = render(<TaskSlideOver task={null} isOpen={false} onClose={vi.fn()} />);
    expect(container.querySelector('[data-testid="slide-over"]')).not.toBeInTheDocument();
  });

  it('renders task detail when open', () => {
    render(<TaskSlideOver task={sampleTask} isOpen onClose={vi.fn()} />);
    expect(screen.getByTestId('slide-over')).toBeInTheDocument();
    expect(screen.getByText('Implement auth')).toBeInTheDocument();
    expect(screen.getByText('done')).toBeInTheDocument();
    expect(screen.getByText('Set up OIDC authentication')).toBeInTheDocument();
  });

  it('calls onClose when close button is clicked', () => {
    const onClose = vi.fn();
    render(<TaskSlideOver task={sampleTask} isOpen onClose={onClose} />);
    fireEvent.click(screen.getByTestId('slide-over-close'));
    expect(onClose).toHaveBeenCalled();
  });

  it('calls onClose when backdrop is clicked', () => {
    const onClose = vi.fn();
    render(<TaskSlideOver task={sampleTask} isOpen onClose={onClose} />);
    fireEvent.click(screen.getByTestId('slide-over-backdrop'));
    expect(onClose).toHaveBeenCalled();
  });
});
