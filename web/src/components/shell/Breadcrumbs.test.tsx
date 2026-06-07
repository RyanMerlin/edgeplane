import { render, screen } from '@testing-library/react';
import type React from 'react';
import { describe, expect, it, vi } from 'vitest';
vi.mock('@tanstack/react-router', () => ({
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => <a href={to}>{children}</a>,
}));
import { CrumbTrail } from './Breadcrumbs';

describe('CrumbTrail', () => {
  it('renders links for non-current crumbs and plain text for the current', () => {
    render(
      <CrumbTrail crumbs={[{ label: 'Agents', to: '/agents' }, { label: 'x', to: undefined }]} />,
    );
    expect(screen.getByRole('link', { name: 'Agents' })).toHaveAttribute('href', '/agents');
    expect(screen.getByText('x')).toBeInTheDocument();
  });
  it('renders nothing for an empty trail', () => {
    const { container } = render(<CrumbTrail crumbs={[]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
