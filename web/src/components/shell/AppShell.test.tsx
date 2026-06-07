import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
vi.mock('./Sidebar', () => ({ Sidebar: () => <div data-testid="app-sidebar-stub" /> }));
vi.mock('./Breadcrumbs', () => ({ default: () => <div data-testid="bc-stub" /> }));
import { AppShell } from './AppShell';

describe('AppShell', () => {
  it('renders the sidebar + a content region containing children + breadcrumbs', () => {
    render(
      <AppShell>
        <div data-testid="page" />
      </AppShell>,
    );
    expect(screen.getByTestId('app-sidebar-stub')).toBeInTheDocument();
    expect(screen.getByTestId('app-content')).toBeInTheDocument();
    expect(screen.getByTestId('bc-stub')).toBeInTheDocument();
    expect(screen.getByTestId('page')).toBeInTheDocument();
  });
});
