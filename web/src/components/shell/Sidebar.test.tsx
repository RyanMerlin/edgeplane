import { fireEvent, render, screen } from '@testing-library/react';
import type React from 'react';
import { describe, expect, it, vi } from 'vitest';

// Mock router
const mockPathname = '/agents/aria-operator-bb05ea7a';
vi.mock('@tanstack/react-router', () => ({
  Link: ({
    to,
    children,
    'data-testid': testid,
    'aria-current': ariaCurrent,
    ...rest
  }: {
    to: string;
    children: React.ReactNode;
    'data-testid'?: string;
    'aria-current'?: string;
  }) => (
    <a href={to} data-testid={testid} aria-current={ariaCurrent} {...rest}>
      {children}
    </a>
  ),
  useRouterState: ({ select }: { select: (s: { location: { pathname: string } }) => string }) =>
    select({ location: { pathname: mockPathname } }),
}));

const logoutSpy = vi.fn();
vi.mock('@/stores/auth', () => ({
  useAuthStore: (
    selector: (s: {
      userSubject: string | null;
      userEmail: string | null;
      logout: () => Promise<void>;
    }) => unknown,
  ) =>
    selector({
      userSubject: '73c5a571f3b774a535810a3835f3b8fa',
      userEmail: null,
      logout: logoutSpy,
    }),
}));

vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { message: string | null }) => unknown) =>
    selector({ message: null }),
}));

import { Sidebar, avatarLabel } from './Sidebar';

describe('avatarLabel', () => {
  it('returns null for null input', () => {
    expect(avatarLabel(null)).toBeNull();
  });
  it('returns null for an opaque hash', () => {
    expect(avatarLabel('73c5a571f3b774a535810a3835f3b8fa')).toBeNull();
  });
  it('returns RM for an email address', () => {
    expect(avatarLabel('ryan.merlin@example.com')).toBe('RM');
  });
  it('returns RM for a display name with spaces', () => {
    expect(avatarLabel('Ryan Merlin')).toBe('RM');
  });
});

describe('Sidebar', () => {
  it('renders /agents nav item with aria-current="page" when pathname is under /agents', () => {
    render(<Sidebar />);
    const link = screen.getByTestId('nav-/agents');
    expect(link).toHaveAttribute('aria-current', 'page');
  });

  it('renders / nav item WITHOUT aria-current when not on root', () => {
    render(<Sidebar />);
    const link = screen.getByTestId('nav-/');
    expect(link).not.toHaveAttribute('aria-current', 'page');
  });

  it('does NOT render Onboarding as a top-level rail link', () => {
    render(<Sidebar />);
    // The old nav-onboarding rail item must be absent from the sidebar rail
    expect(screen.queryByTestId('nav-onboarding')).not.toBeInTheDocument();
  });

  it('shows a glyph avatar (not a hash slice) for an opaque subject', () => {
    render(<Sidebar />);
    // Should NOT render a slice of the opaque hash
    expect(screen.queryByText(/^73/)).not.toBeInTheDocument();
  });

  it('renders logout button in account menu after opening it', () => {
    render(<Sidebar />);
    const accountBtn = screen.getByTestId('account-btn');
    fireEvent.click(accountBtn);
    expect(screen.getByTestId('logout-item')).toBeInTheDocument();
  });

  it('calls logout when logout button is clicked', async () => {
    logoutSpy.mockResolvedValue(undefined);
    render(<Sidebar />);
    const accountBtn = screen.getByTestId('account-btn');
    fireEvent.click(accountBtn);
    const logoutBtn = screen.getByTestId('logout-item');
    fireEvent.click(logoutBtn);
    expect(logoutSpy).toHaveBeenCalled();
  });

  it('reveals menu-onboarding directly after opening the account menu', () => {
    render(<Sidebar />);
    // Open account menu
    fireEvent.click(screen.getByTestId('account-btn'));
    // Onboarding is a flat top-level item — visible immediately
    const onboardingLink = screen.getByTestId('menu-onboarding');
    expect(onboardingLink).toBeInTheDocument();
    expect(onboardingLink).toHaveAttribute('href', '/onboarding');
  });
});
