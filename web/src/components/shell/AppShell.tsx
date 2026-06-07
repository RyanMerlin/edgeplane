import type React from 'react';
import Breadcrumbs from './Breadcrumbs';
import { Sidebar } from './Sidebar';

/** App frame: persistent sidebar + full-height content column with a breadcrumb header. */
export function AppShell({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', height: '100vh', overflow: 'hidden' }}>
      <Sidebar />
      <div
        data-testid="app-content"
        style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', height: '100%' }}
      >
        <header
          style={{
            flexShrink: 0,
            height: 44,
            display: 'flex',
            alignItems: 'center',
            padding: '0 18px',
            borderBottom: '1px solid var(--border-subtle)',
          }}
        >
          <Breadcrumbs />
        </header>
        <main style={{ flex: 1, minHeight: 0, overflow: 'auto', padding: '20px 18px' }}>
          {children}
        </main>
      </div>
    </div>
  );
}

export default AppShell;
