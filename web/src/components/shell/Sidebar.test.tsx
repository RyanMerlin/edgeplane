import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen } from '@testing-library/react';
import type React from 'react';
import { describe, expect, it, vi } from 'vitest';

const mockPathname = '/agents/my-agent-operator-bb05ea7a';
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
      userName: string | null;
      isAdmin: boolean;
      logout: () => Promise<void>;
    }) => unknown,
  ) =>
    selector({
      userSubject: '73c5a571f3b774a535810a3835f3b8fa',
      userEmail: null,
      userName: null,
      isAdmin: false,
      logout: logoutSpy,
    }),
}));

vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { message: string | null }) => unknown) =>
    selector({ message: null }),
}));

vi.mock('@/api/client', () => ({
  apiClient: { GET: vi.fn(), use: vi.fn() },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

import { Sidebar, avatarLabel } from './Sidebar';

function makeQC() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
  });
}
function renderSidebar() {
  const qc = makeQC();
  return render(
    <QueryClientProvider client={qc}>
      <Sidebar />
    </QueryClientProvider>,
  );
}

describe('avatarLabel', () => {
  it('returns null for null input', () => {
    expect(avatarLabel(null)).toBeNull();
  });
  it('returns null for an opaque hash', () => {
    expect(avatarLabel('73c5a571f3b774a535810a3835f3b8fa')).toBeNull();
  });
  it('returns initials for a dotted email address', () => {
    expect(avatarLabel('ada.lovelace@example.com')).toBe('AL');
  });
  it('returns initials for a display name with spaces', () => {
    expect(avatarLabel('Ada Lovelace')).toBe('AL');
  });
});

describe('Sidebar', () => {
  it('renders /agents nav item with aria-current="page" when pathname is under /agents', () => {
    renderSidebar();
    const link = screen.getByTestId('nav-/agents');
    expect(link).toHaveAttribute('aria-current', 'page');
  });

  it('renders /nodes nav item', () => {
    renderSidebar();
    expect(screen.getByTestId('nav-/nodes')).toBeInTheDocument();
  });

  it('renders / nav item WITHOUT aria-current when not on root', () => {
    renderSidebar();
    const link = screen.getByTestId('nav-/');
    expect(link).not.toHaveAttribute('aria-current', 'page');
  });

  it('does NOT render Onboarding as a top-level rail link', () => {
    renderSidebar();
    expect(screen.queryByTestId('nav-onboarding')).not.toBeInTheDocument();
  });

  it('shows a glyph avatar (not a hash slice) for an opaque subject', () => {
    renderSidebar();
    expect(screen.queryByText(/^73/)).not.toBeInTheDocument();
  });

  it('renders logout button in account menu after opening it', () => {
    renderSidebar();
    fireEvent.click(screen.getByTestId('account-btn'));
    expect(screen.getByTestId('logout-item')).toBeInTheDocument();
  });

  it('calls logout when logout button is clicked', async () => {
    logoutSpy.mockResolvedValue(undefined);
    renderSidebar();
    fireEvent.click(screen.getByTestId('account-btn'));
    fireEvent.click(screen.getByTestId('logout-item'));
    expect(logoutSpy).toHaveBeenCalled();
  });

  it('reveals menu-onboarding directly after opening the account menu', () => {
    renderSidebar();
    fireEvent.click(screen.getByTestId('account-btn'));
    const onboardingLink = screen.getByTestId('menu-onboarding');
    expect(onboardingLink).toBeInTheDocument();
    expect(onboardingLink).toHaveAttribute('href', '/onboarding');
  });
});
