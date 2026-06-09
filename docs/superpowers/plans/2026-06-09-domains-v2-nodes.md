# Domains v2 + Nodes Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the flat 3-pane Explorer with hierarchical Domains navigation, add Monaco-powered NORTHSTAR/BRIEF editors, and add a Nodes page.

**Architecture:** TanStack Router file-based routing — `domains.tsx` becomes a layout (`<Outlet />`), new `domains.index.tsx` / `domains.$domainId.tsx` / `domains.$domainId.missions.$missionId.tsx` / `nodes.tsx` / `nodes.index.tsx` / `nodes.$nodeId.tsx` handle the routes. Sidebar gains an inline domain tree via `GET /api/explorer/tree`. Monaco is lazy-loaded via `React.lazy`.

**Tech Stack:** React 19, TanStack Router v1, TanStack Query v5, `@monaco-editor/react` (new), Vitest + RTL, Biome. All commands from `web/` directory.

---

## Codebase snapshot (read before touching anything)

- Routing: `web/src/routes/` — file-based, TanStack Router v1. `agents.tsx` = layout (`<Outlet />`), `agents.index.tsx` = list, `agents.$agentId.tsx` = detail. Same pattern for domains/nodes.
- API client: `import { apiClient, unwrap } from '@/api/client'` — typed via `@/api/schema.gen`. Schema has `RuntimeNode`, `/api/runtime/nodes`, `/api/explorer/tree`. Does NOT have northstar/brief or `/api/runtime/nodes/:id` — use raw `fetch` for those.
- Query keys: `web/src/lib/queryKeys.ts`
- Nav model: `web/src/components/shell/navModel.ts` — `NAV_GROUPS` array + `isNavItemActive`
- Sidebar: `web/src/components/shell/Sidebar.tsx` — renders flat links from `NAV_GROUPS`
- Current `domains.tsx`: single route exporting `ExplorerPage` — will become layout-only
- `explorer.tsx`: already redirects `/explorer` → `/domains` (no change needed)
- Existing schema types: `components['schemas']['ExplorerDomainNode']`, `ExplorerMissionNode`, `ExplorerTreeResponse`, `RuntimeNode`
- Test pattern: `vi.mock('@/api/client')`, `vi.mock('@tanstack/react-router')`, wrap in `QueryClientProvider`
- Build gate: `npm run build && npm test && npm run lint` must pass before each commit

---

## File map

**Create:**
- `web/src/routes/domains.index.tsx` — DomainsOverviewPage
- `web/src/routes/domains.$domainId.tsx` — DomainPage (tabs)
- `web/src/routes/domains.$domainId.missions.$missionId.tsx` — MissionPage (tabs)
- `web/src/routes/nodes.tsx` — Nodes layout
- `web/src/routes/nodes.index.tsx` — NodesPage
- `web/src/routes/nodes.$nodeId.tsx` — NodeDetailPage
- `web/src/components/domains/NarrativeEditor.tsx` — Monaco wrapper
- `web/src/components/domains/TaskSlideOver.tsx` — right slide-over for task detail
- `web/src/components/nodes/NodesTable.tsx` — node list table

**Modify:**
- `web/src/routes/domains.tsx` — retire ExplorerPage, render `<Outlet />`
- `web/src/routes/domains.test.tsx` — update imports for new structure
- `web/src/components/shell/navModel.ts` — add Nodes item
- `web/src/components/shell/navModel.test.ts` — update expected list
- `web/src/components/shell/Sidebar.tsx` — inline domain tree + Nodes icon
- `web/src/components/shell/Sidebar.test.tsx` — add QueryClientProvider, tree tests
- `web/src/lib/queryKeys.ts` — add northstar, brief, nodes keys
- `web/package.json` — add `@monaco-editor/react`

---

## Task 1: Install dep + extend queryKeys

**Files:**
- Modify: `web/package.json`
- Modify: `web/src/lib/queryKeys.ts`

- [ ] **Step 1: Install @monaco-editor/react**

```bash
cd /home/merlin/code/edgeplane/web && npm install @monaco-editor/react
```

Expected: package added to `dependencies` in `package.json`.

- [ ] **Step 2: Update queryKeys.ts**

Replace the entire file:

```typescript
export const queryKeys = {
  onboarding: {
    all: ['onboarding'] as const,
    manifest: () => [...queryKeys.onboarding.all, 'manifest'] as const,
  },
  ai: {
    all: ['ai'] as const,
    sessions: () => [...queryKeys.ai.all, 'sessions'] as const,
    session: (id: string) => [...queryKeys.ai.all, 'session', id] as const,
    turn: (sessionId: string, turnId: number) =>
      [...queryKeys.ai.all, 'turn', sessionId, turnId] as const,
  },
  explorer: {
    all: ['explorer'] as const,
    tree: () => [...queryKeys.explorer.all, 'tree'] as const,
    node: (type: string, id: string) => [...queryKeys.explorer.all, 'node', type, id] as const,
  },
  domains: {
    all: ['domains'] as const,
    northstar: (domainId: string) => [...queryKeys.domains.all, domainId, 'northstar'] as const,
    brief: (domainId: string, missionId: string) =>
      [...queryKeys.domains.all, domainId, 'missions', missionId, 'brief'] as const,
  },
  nodes: {
    all: ['nodes'] as const,
    list: () => [...queryKeys.nodes.all, 'list'] as const,
    detail: (nodeId: string) => [...queryKeys.nodes.all, 'detail', nodeId] as const,
  },
  governance: {
    all: ['governance'] as const,
    policy: () => [...queryKeys.governance.all, 'policy'] as const,
    versions: () => [...queryKeys.governance.all, 'versions'] as const,
    events: () => [...queryKeys.governance.all, 'events'] as const,
  },
  evolve: {
    all: ['evolve'] as const,
    mission: (id: string) => [...queryKeys.evolve.all, 'mission', id] as const,
  },
  agents: {
    all: ['agents'] as const,
    list: () => [...queryKeys.agents.all, 'list'] as const,
    detail: (agentId: string) => [...queryKeys.agents.all, 'detail', agentId] as const,
  },
  jobs: {
    all: ['jobs'] as const,
    list: () => [...queryKeys.jobs.all, 'list'] as const,
    detail: (id: number) => [...queryKeys.jobs.all, 'detail', id] as const,
  },
};
```

- [ ] **Step 3: Verify build**

```bash
cd /home/merlin/code/edgeplane/web && npm run build 2>&1 | tail -5
```

Expected: exit 0, no TypeScript errors.

- [ ] **Step 4: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/package.json web/package-lock.json web/src/lib/queryKeys.ts
git commit -m "$(cat <<'EOF'
feat(web): add @monaco-editor/react dep + extend queryKeys for domains/nodes

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Update navModel — add Nodes

**Files:**
- Modify: `web/src/components/shell/navModel.ts`
- Modify: `web/src/components/shell/navModel.test.ts`

- [ ] **Step 1: Write failing test**

Replace `navModel.test.ts`:

```typescript
import { describe, expect, it } from 'vitest';
import { NAV_GROUPS, isNavItemActive } from './navModel';

describe('navModel', () => {
  it('exposes Dashboard, Agents, Nodes, Domains, Feed, Governance', () => {
    const tos = NAV_GROUPS.flatMap((g) => g.items).map((i) => i.to);
    expect(tos).toEqual(['/', '/agents', '/nodes', '/domains', '/feed', '/governance']);
  });
  it('matches "/" only exactly', () => {
    expect(isNavItemActive('/', '/')).toBe(true);
    expect(isNavItemActive('/', '/agents')).toBe(false);
  });
  it('matches a section by prefix, including detail routes', () => {
    expect(isNavItemActive('/agents', '/agents')).toBe(true);
    expect(isNavItemActive('/agents', '/agents/aria-operator-bb05ea7a')).toBe(true);
    expect(isNavItemActive('/domains', '/domains/apollo')).toBe(true);
    expect(isNavItemActive('/nodes', '/nodes/excalibur-abc')).toBe(true);
    expect(isNavItemActive('/agents', '/agents-foo')).toBe(false);
  });
});
```

- [ ] **Step 2: Run test, confirm it fails**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose navModel 2>&1 | tail -15
```

Expected: FAIL — `['/nodes']` missing from the array.

- [ ] **Step 3: Update navModel.ts**

```typescript
export interface NavItem {
  to: string;
  label: string;
}
export interface NavGroup {
  heading: string | null;
  items: NavItem[];
}

export const NAV_GROUPS: NavGroup[] = [
  { heading: null, items: [{ to: '/', label: 'Dashboard' }] },
  { heading: null, items: [{ to: '/agents', label: 'Agents' }] },
  { heading: null, items: [{ to: '/nodes', label: 'Nodes' }] },
  { heading: null, items: [{ to: '/domains', label: 'Domains' }] },
  {
    heading: null,
    items: [
      { to: '/feed', label: 'Feed' },
      { to: '/governance', label: 'Governance' },
    ],
  },
];

/** Active when pathname equals the item ("/" exact) or is a path-boundary descendant. */
export function isNavItemActive(itemTo: string, pathname: string): boolean {
  if (itemTo === '/') return pathname === '/';
  return pathname === itemTo || pathname.startsWith(`${itemTo}/`);
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose navModel 2>&1 | tail -10
```

Expected: PASS all 3 tests.

- [ ] **Step 5: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/components/shell/navModel.ts web/src/components/shell/navModel.test.ts
git commit -m "$(cat <<'EOF'
feat(web): add Nodes nav item to navModel

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Convert domains.tsx to layout + fix domains.test.tsx

**Files:**
- Modify: `web/src/routes/domains.tsx`
- Modify: `web/src/routes/domains.test.tsx`

The old `ExplorerPage` is retired. `domains.tsx` becomes a passthrough layout. The test file is rewritten to test `DomainsOverviewPage` (created next task) — but since DomainsOverviewPage doesn't exist yet, we stub the import and write tests that will pass once Task 4 lands.

- [ ] **Step 1: Replace domains.tsx with layout**

```typescript
import { Outlet, createFileRoute } from '@tanstack/react-router';

// Domains section layout — list at domains.index.tsx, entity pages at domains.$domainId.tsx
export const Route = createFileRoute('/domains')({
  component: () => <Outlet />,
});
```

- [ ] **Step 2: Replace domains.test.tsx with new test skeleton**

The tests below use the same mock pattern as agents.test.tsx. `DomainsOverviewPage` is imported from `./domains.index` — that file will be created in Task 4.

```typescript
/**
 * Domains routes — unit tests.
 *
 * Covers DomainsOverviewPage (/domains/), DomainPage (/domains/:id).
 * MockS: vi.mock('@/api/client'), vi.mock('@tanstack/react-router').
 */

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    createFileRoute: (_path: string) => (opts: { component: React.ComponentType }) => ({
      ...opts,
      id: _path,
    }),
    Link: ({
      to,
      children,
      style,
    }: { to: string; children: React.ReactNode; style?: React.CSSProperties }) => (
      <a href={to} style={style}>{children}</a>
    ),
    useNavigate: vi.fn(() => vi.fn()),
    useParams: vi.fn(() => ({ domainId: 'domain-uuid-1' })),
  };
});

vi.mock('@/api/client', () => ({
  apiClient: { GET: vi.fn(), POST: vi.fn().mockResolvedValue({ data: { ok: true } }), use: vi.fn() },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

vi.mock('@monaco-editor/react', () => ({
  default: ({ value, onChange }: { value: string; onChange?: (v: string) => void }) => (
    <textarea data-testid="monaco-editor" defaultValue={value} onChange={(e) => onChange?.(e.target.value)} />
  ),
}));

const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

import { DomainsOverviewPage } from './domains.index';

const sampleTree = {
  domain_count: 1,
  mission_count: 1,
  task_count: 3,
  generated_at: '2026-05-31T10:00:00Z',
  domains: [
    {
      id: 'domain-uuid-1',
      name: 'Apollo',
      description: 'Investment data',
      status: 'active',
      owners: 'aria-operator',
      tags: null,
      visibility: 'public',
      mission_count: 1,
      task_count: 3,
      missions: [
        {
          id: 'mission-uuid-1',
          name: 'Warehouse rebuild',
          description: 'Rebuild warehouse',
          domain_id: 'domain-uuid-1',
          status: 'in_progress',
          owners: 'aria-operator',
          tags: null,
          task_count: 3,
          task_status_counts: { open: 2, done: 1 },
          recent_tasks: [],
          updated_at: '2026-05-30T12:00:00Z',
        },
      ],
      updated_at: '2026-05-30T12:00:00Z',
    },
  ],
  unassigned_missions: [],
};

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('DomainsOverviewPage', () => {
  let qc: QueryClient;
  beforeEach(() => { qc = makeQC(); vi.clearAllMocks(); });
  afterEach(() => qc.clear());

  it('shows loading state while tree is pending', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    wrap(<DomainsOverviewPage />, qc);
    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('shows error state when tree fails', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('fail'));
    wrap(<DomainsOverviewPage />, qc);
    await waitFor(() => expect(screen.getByTestId('error-state')).toBeInTheDocument());
  });

  it('shows empty state when no domains', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
      ...sampleTree, domain_count: 0, domains: [],
    });
    wrap(<DomainsOverviewPage />, qc);
    await waitFor(() => expect(screen.getByTestId('empty-state')).toBeInTheDocument());
  });

  it('renders domain with name, status, mission count', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    wrap(<DomainsOverviewPage />, qc);
    await waitFor(() => expect(screen.getByTestId('domain-row-domain-uuid-1')).toBeInTheDocument());
    expect(screen.getByText('Apollo')).toBeInTheDocument();
    expect(screen.getByText('active')).toBeInTheDocument();
  });

  it('shows nested mission when domain row is expanded', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    wrap(<DomainsOverviewPage />, qc);
    await waitFor(() => expect(screen.getByTestId('mission-row-mission-uuid-1')).toBeInTheDocument());
    expect(screen.getByText('Warehouse rebuild')).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: Run tests — domains.test.tsx should fail on import (DomainsOverviewPage missing)**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose domains.test 2>&1 | tail -15
```

Expected: FAIL with "Cannot find module './domains.index'" or similar.

- [ ] **Step 4: Verify build still passes**

```bash
cd /home/merlin/code/edgeplane/web && npm run build 2>&1 | tail -5
```

Expected: exit 0 (domains.tsx is now just a layout, no broken exports).

- [ ] **Step 5: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/routes/domains.tsx web/src/routes/domains.test.tsx
git commit -m "$(cat <<'EOF'
refactor(web): convert domains.tsx to layout, update test skeleton for new routes

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: NarrativeEditor component

Shared Monaco wrapper used by DomainPage (Northstar) and MissionPage (Brief).

**Files:**
- Create: `web/src/components/domains/NarrativeEditor.tsx`
- Create: `web/src/components/domains/NarrativeEditor.test.tsx`

- [ ] **Step 1: Write failing test**

Create `web/src/components/domains/NarrativeEditor.test.tsx`:

```typescript
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@monaco-editor/react', () => ({
  default: ({ value, onChange }: { value: string; onChange?: (v: string | undefined) => void }) => (
    <textarea
      data-testid="monaco-editor"
      defaultValue={value}
      onChange={(e) => onChange?.(e.target.value)}
    />
  ),
}));

import { NarrativeEditor } from './NarrativeEditor';

describe('NarrativeEditor', () => {
  it('renders Save button', () => {
    render(<NarrativeEditor initialValue="# Hello" onSave={vi.fn()} />);
    expect(screen.getByTestId('narrative-save-btn')).toBeInTheDocument();
  });

  it('calls onSave with current value when Save clicked', () => {
    const onSave = vi.fn();
    render(<NarrativeEditor initialValue="# Hello" onSave={onSave} />);
    fireEvent.click(screen.getByTestId('narrative-save-btn'));
    expect(onSave).toHaveBeenCalledWith('# Hello');
  });

  it('shows saving state while isSaving is true', () => {
    render(<NarrativeEditor initialValue="" onSave={vi.fn()} isSaving />);
    expect(screen.getByTestId('narrative-save-btn')).toBeDisabled();
    expect(screen.getByText('Saving…')).toBeInTheDocument();
  });

  it('shows save error when saveError is set', () => {
    render(<NarrativeEditor initialValue="" onSave={vi.fn()} saveError="Network error" />);
    expect(screen.getByTestId('save-error')).toHaveTextContent('Network error');
  });

  it('shows version footer when version and modifiedAt provided', () => {
    render(
      <NarrativeEditor
        initialValue=""
        onSave={vi.fn()}
        version={3}
        modifiedAt="2026-06-01T10:00:00Z"
      />,
    );
    const footer = screen.getByTestId('editor-footer');
    expect(footer).toHaveTextContent('v3');
  });
});
```

- [ ] **Step 2: Run test, confirm fail**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose NarrativeEditor 2>&1 | tail -10
```

Expected: FAIL — `NarrativeEditor` not found.

- [ ] **Step 3: Create NarrativeEditor.tsx**

```typescript
import { Suspense, lazy, useState } from 'react';

const MonacoEditor = lazy(() => import('@monaco-editor/react'));

interface NarrativeEditorProps {
  initialValue: string;
  onSave: (value: string) => void;
  isSaving?: boolean;
  saveError?: string | null;
  version?: number;
  modifiedAt?: string | null;
}

export function NarrativeEditor({
  initialValue,
  onSave,
  isSaving,
  saveError,
  version,
  modifiedAt,
}: NarrativeEditorProps) {
  const [value, setValue] = useState(initialValue);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', gap: 4 }}>
      <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
        <button
          type="button"
          data-testid="narrative-save-btn"
          disabled={isSaving}
          onClick={() => onSave(value)}
          style={{
            padding: '3px 10px',
            background: 'var(--accent-dim)',
            color: 'var(--accent)',
            border: '1px solid var(--accent)',
            borderRadius: 4,
            fontSize: 12,
            cursor: isSaving ? 'not-allowed' : 'pointer',
            fontFamily: 'var(--font)',
          }}
        >
          {isSaving ? 'Saving…' : 'Save'}
        </button>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <Suspense fallback={<div data-testid="editor-loading" style={{ color: 'var(--dim)', fontSize: 13 }}>Loading editor…</div>}>
          <MonacoEditor
            height="100%"
            language="markdown"
            theme="vs-dark"
            value={value}
            onChange={(v) => setValue(v ?? '')}
            options={{ minimap: { enabled: false }, wordWrap: 'on', fontSize: 13 }}
          />
        </Suspense>
      </div>
      {saveError && (
        <div data-testid="save-error" style={{ color: 'var(--err)', fontSize: 12, padding: '2px 0' }}>
          {saveError}
        </div>
      )}
      {(version !== undefined || modifiedAt) && (
        <div
          data-testid="editor-footer"
          style={{ fontSize: 11, color: 'var(--dim)', padding: '2px 0' }}
        >
          {version !== undefined && `v${version}`}
          {modifiedAt && ` · saved ${new Date(modifiedAt).toLocaleString()}`}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose NarrativeEditor 2>&1 | tail -10
```

Expected: PASS all 5 tests.

- [ ] **Step 5: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/components/domains/
git commit -m "$(cat <<'EOF'
feat(web): add NarrativeEditor Monaco wrapper component

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: DomainsOverviewPage (domains.index.tsx)

**Files:**
- Create: `web/src/routes/domains.index.tsx`

- [ ] **Step 1: Run existing domains.test.tsx — still failing on import**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose domains.test 2>&1 | tail -10
```

Expected: FAIL — `Cannot find module './domains.index'`.

- [ ] **Step 2: Create domains.index.tsx**

```typescript
import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';

type ExplorerDomainNode = components['schemas']['ExplorerDomainNode'];
type ExplorerMissionNode = components['schemas']['ExplorerMissionNode'];

export const Route = createFileRoute('/domains/')({
  component: DomainsOverviewPage,
});

export function DomainsOverviewPage() {
  const { data: tree, isLoading, isError } = useQuery({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => unwrap(apiClient.GET('/api/explorer/tree', {})),
    refetchInterval: 30_000,
  });

  if (isLoading) return <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>Loading…</div>;
  if (isError) return <div data-testid="error-state" style={{ padding: 24, color: 'var(--err)', fontSize: 13 }}>Failed to load domains.</div>;
  if (!tree || tree.domains.length === 0) {
    return <div data-testid="empty-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>No domains.</div>;
  }

  return (
    <div data-testid="domains-overview" style={{ padding: '16px 24px' }}>
      <div style={{ marginBottom: 16 }}>
        <span style={{ fontSize: 11, fontWeight: 590, color: 'var(--dim)', letterSpacing: '0.06em', textTransform: 'uppercase' }}>
          {tree.domain_count} domains · {tree.mission_count} missions · {tree.task_count} tasks
        </span>
      </div>
      {tree.domains.map((domain) => (
        <DomainOverviewRow key={domain.id} domain={domain} />
      ))}
    </div>
  );
}

function statusDot(status: string): string {
  const v = status.toLowerCase();
  if (v === 'done' || v === 'completed') return '✓';
  if (v === 'in_progress' || v === 'running') return '⟳';
  if (v === 'blocked' || v === 'failed') return '✗';
  if (v === 'proposed') return '○';
  return '●';
}

function statusColor(status: string): string {
  const v = status.toLowerCase();
  if (v === 'done' || v === 'completed') return 'var(--ok)';
  if (v === 'blocked' || v === 'failed') return 'var(--err)';
  if (v === 'in_progress' || v === 'active' || v === 'running') return 'var(--accent)';
  return 'var(--dim)';
}

function DomainOverviewRow({ domain }: { domain: ExplorerDomainNode }) {
  const [expanded, setExpanded] = useState(true);

  return (
    <div style={{ marginBottom: 8 }}>
      <div
        data-testid={`domain-row-${domain.id}`}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '6px 8px',
          borderRadius: 6,
          cursor: 'pointer',
          background: 'var(--raised)',
          marginBottom: expanded ? 2 : 0,
        }}
      >
        <button
          type="button"
          aria-label={expanded ? 'Collapse' : 'Expand'}
          onClick={() => setExpanded((v) => !v)}
          style={{ background: 'none', border: 'none', color: 'var(--dim)', fontSize: 11, cursor: 'pointer', padding: 0, width: 16 }}
        >
          {expanded ? '▾' : '▸'}
        </button>
        <Link
          to="/domains/$domainId"
          params={{ domainId: domain.id }}
          style={{ flex: 1, display: 'flex', alignItems: 'center', gap: 8, textDecoration: 'none' }}
        >
          <span style={{ fontSize: 13, fontWeight: 510, color: 'var(--text)' }}>{domain.name}</span>
          <span style={{ fontSize: 11, color: statusColor(domain.status), background: 'var(--raised-2)', padding: '1px 6px', borderRadius: 3 }}>
            {domain.status}
          </span>
          <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)' }}>
            {domain.mission_count}m · {domain.task_count}t
          </span>
        </Link>
      </div>
      {expanded && domain.missions.map((mission) => (
        <MissionOverviewRow key={mission.id} mission={mission} domainId={domain.id} />
      ))}
    </div>
  );
}

function MissionOverviewRow({ mission, domainId }: { mission: ExplorerMissionNode; domainId: string }) {
  return (
    <Link
      to="/domains/$domainId/missions/$missionId"
      params={{ domainId, missionId: mission.id }}
      data-testid={`mission-row-${mission.id}`}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '4px 8px 4px 32px',
        borderRadius: 5,
        textDecoration: 'none',
        color: 'var(--text-2)',
        fontSize: 12,
        marginBottom: 1,
      }}
    >
      <span style={{ color: statusColor(mission.status) }}>{statusDot(mission.status)}</span>
      <span>{mission.name}</span>
      <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)' }}>{mission.task_count}t</span>
    </Link>
  );
}
```

- [ ] **Step 3: Run domains.test.tsx — should now pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose domains.test 2>&1 | tail -15
```

Expected: PASS all DomainsOverviewPage tests.

- [ ] **Step 4: Run full test suite**

```bash
cd /home/merlin/code/edgeplane/web && npm test 2>&1 | tail -10
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/routes/domains.index.tsx
git commit -m "$(cat <<'EOF'
feat(web): add DomainsOverviewPage with interactive domain/mission tree

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: DomainPage (domains.$domainId.tsx)

The domain page has three tabs: Northstar (default, Monaco editor), Missions (list), Overview (stats). Header data comes from the explorer tree (find domain by ID). Northstar content from `GET /api/domains/:id/northstar` (not in schema — raw fetch).

**Files:**
- Create: `web/src/routes/domains.$domainId.tsx`
- Create: `web/src/routes/domains.$domainId.test.tsx`

- [ ] **Step 1: Write failing test**

Create `web/src/routes/domains.$domainId.test.tsx`:

```typescript
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    createFileRoute: (_path: string) => (opts: { component: React.ComponentType }) => ({ ...opts, id: _path }),
    Link: ({ to, children, style }: { to: string; children: React.ReactNode; style?: React.CSSProperties }) => (
      <a href={to} style={style}>{children}</a>
    ),
    useParams: vi.fn(() => ({ domainId: 'domain-uuid-1' })),
    useNavigate: vi.fn(() => vi.fn()),
  };
});

vi.mock('@/api/client', () => ({
  apiClient: { GET: vi.fn(), POST: vi.fn().mockResolvedValue({ data: { ok: true } }), use: vi.fn() },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

vi.mock('@monaco-editor/react', () => ({
  default: ({ value }: { value: string }) => (
    <textarea data-testid="monaco-editor" defaultValue={value} readOnly />
  ),
}));

const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

import { DomainPage } from './domains.$domainId';

const sampleTree = {
  domain_count: 1, mission_count: 1, task_count: 3,
  generated_at: '2026-05-31T10:00:00Z',
  domains: [{
    id: 'domain-uuid-1', name: 'Apollo', description: 'Investment data',
    status: 'active', owners: 'aria-operator', tags: null, visibility: 'public',
    mission_count: 1, task_count: 3, updated_at: '2026-05-30T12:00:00Z',
    missions: [{
      id: 'mission-uuid-1', name: 'Warehouse rebuild', description: 'rebuild',
      domain_id: 'domain-uuid-1', status: 'in_progress', owners: 'aria-operator',
      tags: null, task_count: 3, task_status_counts: { open: 2, done: 1 },
      recent_tasks: [], updated_at: '2026-05-30T12:00:00Z',
    }],
  }],
  unassigned_missions: [],
};

const sampleNorthstar = {
  northstar_md: '# Apollo\n\nDrives investment data.',
  northstar_version: 3,
  northstar_modified_by: 'aria-operator',
  northstar_modified_at: '2026-06-01T10:00:00Z',
};

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('DomainPage', () => {
  let qc: QueryClient;
  beforeEach(() => { qc = makeQC(); vi.clearAllMocks(); });
  afterEach(() => qc.clear());

  it('shows domain name in header after loading', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    (unwrap as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(sampleTree)
      .mockResolvedValueOnce(sampleNorthstar);
    global.fetch = vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(sampleNorthstar) });
    wrap(<DomainPage />, qc);
    await waitFor(() => expect(screen.getByTestId('domain-header')).toBeInTheDocument());
    expect(screen.getByText('Apollo')).toBeInTheDocument();
    expect(screen.getByText('active')).toBeInTheDocument();
  });

  it('renders Northstar tab by default with Monaco editor', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    global.fetch = vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(sampleNorthstar) });
    wrap(<DomainPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-northstar')).toBeInTheDocument());
    expect(screen.getByTestId('tab-northstar')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('monaco-editor')).toBeInTheDocument();
  });

  it('switches to Missions tab and shows missions', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleTree);
    global.fetch = vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(sampleNorthstar) });
    wrap(<DomainPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-missions')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('tab-missions'));
    await waitFor(() => expect(screen.getByTestId('mission-row-mission-uuid-1')).toBeInTheDocument());
    expect(screen.getByText('Warehouse rebuild')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test, confirm fail**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose "domains.\$domainId.test" 2>&1 | tail -10
```

Expected: FAIL — module not found.

- [ ] **Step 3: Create domains.$domainId.tsx**

```typescript
import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { NarrativeEditor } from '@/components/domains/NarrativeEditor';
import { queryKeys } from '@/lib/queryKeys';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, createFileRoute, useParams } from '@tanstack/react-router';
import { useState } from 'react';

type ExplorerDomainNode = components['schemas']['ExplorerDomainNode'];
type ExplorerMissionNode = components['schemas']['ExplorerMissionNode'];

interface NorthstarResponse {
  northstar_md: string;
  northstar_version: number;
  northstar_modified_by: string | null;
  northstar_modified_at: string | null;
}

export const Route = createFileRoute('/domains/$domainId')({
  component: DomainPage,
});

export function DomainPage() {
  const { domainId } = useParams({ from: '/domains/$domainId' });
  const [activeTab, setActiveTab] = useState<'northstar' | 'missions' | 'overview'>('northstar');
  const qc = useQueryClient();

  const { data: tree, isLoading: treeLoading } = useQuery({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => unwrap(apiClient.GET('/api/explorer/tree', {})),
    refetchInterval: 30_000,
  });

  const domain: ExplorerDomainNode | undefined = tree?.domains.find((d) => d.id === domainId);

  const { data: northstar, isLoading: nsLoading } = useQuery({
    queryKey: queryKeys.domains.northstar(domainId),
    queryFn: async (): Promise<NorthstarResponse> => {
      const res = await fetch(`/api/domains/${domainId}/northstar`, {
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
      });
      if (!res.ok) throw new Error(`northstar fetch failed: ${res.status}`);
      return res.json();
    },
    enabled: activeTab === 'northstar',
  });

  const saveMutation = useMutation({
    mutationFn: async (md: string) => {
      const res = await fetch(`/api/domains/${domainId}/northstar`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({ northstar_md: md }),
      });
      if (!res.ok) throw new Error(`save failed: ${res.status}`);
      return res.json();
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.domains.northstar(domainId) });
    },
  });

  if (treeLoading) return <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>Loading…</div>;
  if (!domain) return <div data-testid="not-found-state" style={{ padding: 24, color: 'var(--err)', fontSize: 13 }}>Domain not found.</div>;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      {/* Header */}
      <div
        data-testid="domain-header"
        style={{ padding: '12px 24px 0', borderBottom: '1px solid var(--border)', flexShrink: 0 }}
      >
        <div style={{ fontSize: 11, color: 'var(--dim)', marginBottom: 4 }}>
          <Link to="/domains" style={{ color: 'var(--dim)', textDecoration: 'none' }}>Domains</Link>
          {' › '}
          <span style={{ color: 'var(--text)' }}>{domain.name}</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
          <span style={{ fontSize: 16, fontWeight: 590, color: 'var(--text)' }}>{domain.name}</span>
          <span style={{ fontSize: 11, color: 'var(--accent)', background: 'var(--accent-dim)', padding: '1px 6px', borderRadius: 3 }}>
            {domain.status}
          </span>
          <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)' }}>
            {domain.mission_count} missions · {domain.task_count} tasks
          </span>
        </div>
        {/* Tabs */}
        <div style={{ display: 'flex', gap: 0 }}>
          {(['northstar', 'missions', 'overview'] as const).map((tab) => (
            <button
              key={tab}
              type="button"
              data-testid={`tab-${tab}`}
              aria-selected={activeTab === tab}
              onClick={() => setActiveTab(tab)}
              style={{
                padding: '6px 14px',
                fontSize: 12,
                background: 'none',
                border: 'none',
                borderBottom: activeTab === tab ? '2px solid var(--accent)' : '2px solid transparent',
                color: activeTab === tab ? 'var(--text)' : 'var(--dim)',
                cursor: 'pointer',
                fontFamily: 'var(--font)',
                textTransform: 'capitalize',
                marginBottom: -1,
              }}
            >
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>
      </div>

      {/* Tab content */}
      <div style={{ flex: 1, minHeight: 0, padding: '16px 24px', display: 'flex', flexDirection: 'column' }}>
        {activeTab === 'northstar' && (
          <NarrativeEditor
            initialValue={northstar?.northstar_md ?? (nsLoading ? '' : '')}
            onSave={(md) => saveMutation.mutate(md)}
            isSaving={saveMutation.isPending}
            saveError={saveMutation.isError ? String(saveMutation.error) : null}
            version={northstar?.northstar_version}
            modifiedAt={northstar?.northstar_modified_at}
          />
        )}
        {activeTab === 'missions' && <MissionsTab missions={domain.missions} domainId={domainId} />}
        {activeTab === 'overview' && <OverviewTab domain={domain} />}
      </div>
    </div>
  );
}

function MissionsTab({ missions, domainId }: { missions: ExplorerMissionNode[]; domainId: string }) {
  if (missions.length === 0) {
    return <div style={{ color: 'var(--dim)', fontSize: 13 }}>No missions in this domain.</div>;
  }
  return (
    <div>
      {missions.map((m) => (
        <Link
          key={m.id}
          to="/domains/$domainId/missions/$missionId"
          params={{ domainId, missionId: m.id }}
          data-testid={`mission-row-${m.id}`}
          style={{
            display: 'flex', alignItems: 'center', gap: 10, padding: '8px 10px',
            borderRadius: 5, textDecoration: 'none', marginBottom: 2,
            background: 'var(--raised)',
          }}
        >
          <span style={{ fontSize: 13, color: 'var(--text)' }}>{m.name}</span>
          <span style={{ fontSize: 11, color: 'var(--accent)', background: 'var(--accent-dim)', padding: '1px 5px', borderRadius: 3 }}>
            {m.status}
          </span>
          <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)' }}>{m.task_count}t</span>
        </Link>
      ))}
    </div>
  );
}

function OverviewTab({ domain }: { domain: ExplorerDomainNode }) {
  return (
    <dl style={{ display: 'grid', gridTemplateColumns: 'max-content 1fr', gap: '6px 16px', fontSize: 13 }}>
      <dt style={{ color: 'var(--dim)' }}>Description</dt>
      <dd style={{ color: 'var(--text)', margin: 0 }}>{domain.description || '—'}</dd>
      <dt style={{ color: 'var(--dim)' }}>Status</dt>
      <dd style={{ color: 'var(--accent)', margin: 0 }}>{domain.status}</dd>
      <dt style={{ color: 'var(--dim)' }}>Owners</dt>
      <dd style={{ color: 'var(--text)', margin: 0 }}>{domain.owners || '—'}</dd>
      <dt style={{ color: 'var(--dim)' }}>Updated</dt>
      <dd style={{ color: 'var(--text)', margin: 0 }}>{new Date(domain.updated_at).toLocaleString()}</dd>
    </dl>
  );
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose "domains.\$domainId.test" 2>&1 | tail -15
```

Expected: PASS all 3 tests.

- [ ] **Step 5: Full test suite**

```bash
cd /home/merlin/code/edgeplane/web && npm test 2>&1 | tail -10
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/routes/domains.\$domainId.tsx web/src/routes/domains.\$domainId.test.tsx
git commit -m "$(cat <<'EOF'
feat(web): add DomainPage with Northstar/Missions/Overview tabs

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: TaskSlideOver component

Right slide-over panel for task detail (opened from MissionPage Tasks tab).

**Files:**
- Create: `web/src/components/domains/TaskSlideOver.tsx`
- Create: `web/src/components/domains/TaskSlideOver.test.tsx`

- [ ] **Step 1: Write failing test**

Create `web/src/components/domains/TaskSlideOver.test.tsx`:

```typescript
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
  owner: 'aria-operator',
  contributors: '',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-05-30T12:00:00Z',
};

describe('TaskSlideOver', () => {
  it('renders nothing when closed', () => {
    const { container } = render(
      <TaskSlideOver task={null} isOpen={false} onClose={vi.fn()} />,
    );
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
```

- [ ] **Step 2: Run test, confirm fail**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose TaskSlideOver 2>&1 | tail -10
```

- [ ] **Step 3: Create TaskSlideOver.tsx**

```typescript
interface TaskRecord {
  id: number;
  public_id: string;
  mission_id: string;
  title: string;
  description?: string | null;
  status: string;
  owner?: string | null;
  contributors?: string | null;
  created_at: string;
  updated_at: string;
}

interface TaskSlideOverProps {
  task: TaskRecord | null;
  isOpen: boolean;
  onClose: () => void;
}

export function TaskSlideOver({ task, isOpen, onClose }: TaskSlideOverProps) {
  if (!isOpen || !task) return null;

  return (
    <>
      <div
        data-testid="slide-over-backdrop"
        onClick={onClose}
        style={{
          position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)', zIndex: 40,
        }}
      />
      <div
        data-testid="slide-over"
        style={{
          position: 'fixed', top: 0, right: 0, bottom: 0,
          width: 420, background: 'var(--frame)', borderLeft: '1px solid var(--border)',
          zIndex: 50, display: 'flex', flexDirection: 'column', padding: '16px 20px',
          overflowY: 'auto',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 16 }}>
          <span style={{ fontSize: 14, fontWeight: 590, color: 'var(--text)', flex: 1 }}>
            {task.title}
          </span>
          <button
            type="button"
            data-testid="slide-over-close"
            onClick={onClose}
            style={{ background: 'none', border: 'none', color: 'var(--dim)', cursor: 'pointer', fontSize: 16 }}
          >
            ✕
          </button>
        </div>
        <dl style={{ display: 'grid', gridTemplateColumns: 'max-content 1fr', gap: '6px 16px', fontSize: 13 }}>
          <dt style={{ color: 'var(--dim)' }}>Status</dt>
          <dd style={{ margin: 0, color: 'var(--accent)' }}>{task.status}</dd>
          <dt style={{ color: 'var(--dim)' }}>ID</dt>
          <dd style={{ margin: 0, color: 'var(--text-2)', fontFamily: 'var(--mono)', fontSize: 12 }}>{task.public_id}</dd>
          {task.owner && (
            <>
              <dt style={{ color: 'var(--dim)' }}>Owner</dt>
              <dd style={{ margin: 0, color: 'var(--text)' }}>{task.owner}</dd>
            </>
          )}
        </dl>
        {task.description && (
          <p style={{ marginTop: 16, fontSize: 13, color: 'var(--text-2)', lineHeight: 1.6 }}>
            {task.description}
          </p>
        )}
      </div>
    </>
  );
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose TaskSlideOver 2>&1 | tail -10
```

Expected: PASS all 4 tests.

- [ ] **Step 5: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/components/domains/TaskSlideOver.tsx web/src/components/domains/TaskSlideOver.test.tsx
git commit -m "$(cat <<'EOF'
feat(web): add TaskSlideOver panel component

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: MissionPage (domains.$domainId.missions.$missionId.tsx)

Tabs: Brief (Monaco, default), Tasks (list + TaskSlideOver), Overview.

**Files:**
- Create: `web/src/routes/domains.$domainId.missions.$missionId.tsx`
- Create: `web/src/routes/domains.$domainId.missions.$missionId.test.tsx`

- [ ] **Step 1: Write failing test**

Create `web/src/routes/domains.$domainId.missions.$missionId.test.tsx`:

```typescript
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    createFileRoute: (_path: string) => (opts: { component: React.ComponentType }) => ({ ...opts, id: _path }),
    Link: ({ to, children, style }: { to: string; children: React.ReactNode; style?: React.CSSProperties }) => (
      <a href={to} style={style}>{children}</a>
    ),
    useParams: vi.fn(() => ({ domainId: 'domain-uuid-1', missionId: 'mission-uuid-1' })),
    useNavigate: vi.fn(() => vi.fn()),
  };
});

vi.mock('@/api/client', () => ({
  apiClient: { GET: vi.fn(), POST: vi.fn().mockResolvedValue({ data: { ok: true } }), use: vi.fn() },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

vi.mock('@monaco-editor/react', () => ({
  default: ({ value }: { value: string }) => (
    <textarea data-testid="monaco-editor" defaultValue={value} readOnly />
  ),
}));

const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

import { MissionPage } from './domains.$domainId.missions.$missionId';

const sampleNodeDetail = {
  node_type: 'mission',
  node_id: 'mission-uuid-1',
  domain: null,
  mission: {
    id: 'mission-uuid-1', name: 'Warehouse rebuild',
    description: 'Rebuild warehouse data pipelines',
    domain_id: 'domain-uuid-1', status: 'in_progress',
    owners: 'aria-operator', tags: null,
    created_at: '2026-01-01T00:00:00Z', updated_at: '2026-05-30T12:00:00Z',
  },
  tasks: [{
    id: 101, public_id: 'task-pub-101', mission_id: 'mission-uuid-1',
    title: 'Set up ingestion', description: 'Ingest OHLCV data',
    status: 'done', owner: 'aria-operator', contributors: '',
    created_at: '2026-01-01T00:00:00Z', updated_at: '2026-05-30T12:00:00Z',
  }],
  missions: null, task: null,
};

const sampleBrief = {
  brief_md: '# Warehouse\n\nRebuild the data warehouse.',
  brief_version: 2,
  brief_modified_by: 'aria-operator',
  brief_modified_at: '2026-06-01T10:00:00Z',
};

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('MissionPage', () => {
  let qc: QueryClient;
  beforeEach(() => {
    qc = makeQC(); vi.clearAllMocks();
    global.fetch = vi.fn().mockResolvedValue({
      ok: true, json: () => Promise.resolve(sampleBrief),
    });
  });
  afterEach(() => qc.clear());

  it('shows mission name after loading', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    wrap(<MissionPage />, qc);
    await waitFor(() => expect(screen.getByTestId('mission-header')).toBeInTheDocument());
    expect(screen.getByText('Warehouse rebuild')).toBeInTheDocument();
  });

  it('shows Brief tab as default with Monaco editor', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    wrap(<MissionPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-brief')).toBeInTheDocument());
    expect(screen.getByTestId('tab-brief')).toHaveAttribute('aria-selected', 'true');
    await waitFor(() => expect(screen.getByTestId('monaco-editor')).toBeInTheDocument());
  });

  it('switches to Tasks tab and shows tasks', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    wrap(<MissionPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-tasks')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('tab-tasks'));
    await waitFor(() => expect(screen.getByTestId('task-row-101')).toBeInTheDocument());
    expect(screen.getByText('Set up ingestion')).toBeInTheDocument();
  });

  it('opens TaskSlideOver when task row is clicked', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue(sampleNodeDetail);
    wrap(<MissionPage />, qc);
    await waitFor(() => expect(screen.getByTestId('tab-tasks')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('tab-tasks'));
    await waitFor(() => expect(screen.getByTestId('task-row-101')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('task-row-101'));
    expect(screen.getByTestId('slide-over')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test, confirm fail**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose "missions.\$missionId.test" 2>&1 | tail -10
```

- [ ] **Step 3: Create domains.$domainId.missions.$missionId.tsx**

```typescript
import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { NarrativeEditor } from '@/components/domains/NarrativeEditor';
import { TaskSlideOver } from '@/components/domains/TaskSlideOver';
import { queryKeys } from '@/lib/queryKeys';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, createFileRoute, useParams } from '@tanstack/react-router';
import { useState } from 'react';

type ExplorerMission = components['schemas']['ExplorerMission'];
type ExplorerTask = components['schemas']['ExplorerTask'];

interface BriefResponse {
  brief_md: string;
  brief_version: number;
  brief_modified_by: string | null;
  brief_modified_at: string | null;
}

interface TaskRecord {
  id: number;
  public_id: string;
  mission_id: string;
  title: string;
  description?: string | null;
  status: string;
  owner?: string | null;
  contributors?: string | null;
  created_at: string;
  updated_at: string;
}

export const Route = createFileRoute('/domains/$domainId/missions/$missionId')({
  component: MissionPage,
});

export function MissionPage() {
  const { domainId, missionId } = useParams({ from: '/domains/$domainId/missions/$missionId' });
  const [activeTab, setActiveTab] = useState<'brief' | 'tasks' | 'overview'>('brief');
  const [selectedTask, setSelectedTask] = useState<TaskRecord | null>(null);
  const qc = useQueryClient();

  const { data: detail, isLoading } = useQuery({
    queryKey: queryKeys.explorer.node('mission', missionId),
    queryFn: () => unwrap(apiClient.GET('/api/explorer/node/{node_type}/{node_id}', {
      params: { path: { node_type: 'mission', node_id: missionId } },
    })),
  });

  const mission = detail?.mission as ExplorerMission | undefined;
  const tasks = (detail?.tasks ?? []) as TaskRecord[];

  const { data: brief } = useQuery({
    queryKey: queryKeys.domains.brief(domainId, missionId),
    queryFn: async (): Promise<BriefResponse> => {
      const res = await fetch(`/api/domains/${domainId}/m/${missionId}/brief`, {
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
      });
      if (!res.ok) throw new Error(`brief fetch failed: ${res.status}`);
      return res.json();
    },
    enabled: activeTab === 'brief',
  });

  const saveMutation = useMutation({
    mutationFn: async (md: string) => {
      const res = await fetch(`/api/domains/${domainId}/m/${missionId}/brief`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify({ brief_md: md }),
      });
      if (!res.ok) throw new Error(`save failed: ${res.status}`);
      return res.json();
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: queryKeys.domains.brief(domainId, missionId) });
    },
  });

  if (isLoading) return <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>Loading…</div>;
  if (!mission) return <div data-testid="not-found-state" style={{ padding: 24, color: 'var(--err)', fontSize: 13 }}>Mission not found.</div>;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div
        data-testid="mission-header"
        style={{ padding: '12px 24px 0', borderBottom: '1px solid var(--border)', flexShrink: 0 }}
      >
        <div style={{ fontSize: 11, color: 'var(--dim)', marginBottom: 4 }}>
          <Link to="/domains" style={{ color: 'var(--dim)', textDecoration: 'none' }}>Domains</Link>
          {' › '}
          <Link to="/domains/$domainId" params={{ domainId }} style={{ color: 'var(--dim)', textDecoration: 'none' }}>
            {domainId}
          </Link>
          {' › '}
          <span style={{ color: 'var(--text)' }}>{mission.name}</span>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 8 }}>
          <span style={{ fontSize: 16, fontWeight: 590, color: 'var(--text)' }}>{mission.name}</span>
          <span style={{ fontSize: 11, color: 'var(--accent)', background: 'var(--accent-dim)', padding: '1px 6px', borderRadius: 3 }}>
            {mission.status}
          </span>
        </div>
        <div style={{ display: 'flex', gap: 0 }}>
          {(['brief', 'tasks', 'overview'] as const).map((tab) => (
            <button
              key={tab}
              type="button"
              data-testid={`tab-${tab}`}
              aria-selected={activeTab === tab}
              onClick={() => setActiveTab(tab)}
              style={{
                padding: '6px 14px', fontSize: 12, background: 'none', border: 'none',
                borderBottom: activeTab === tab ? '2px solid var(--accent)' : '2px solid transparent',
                color: activeTab === tab ? 'var(--text)' : 'var(--dim)',
                cursor: 'pointer', fontFamily: 'var(--font)', textTransform: 'capitalize', marginBottom: -1,
              }}
            >
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, padding: '16px 24px', display: 'flex', flexDirection: 'column' }}>
        {activeTab === 'brief' && (
          <NarrativeEditor
            initialValue={brief?.brief_md ?? ''}
            onSave={(md) => saveMutation.mutate(md)}
            isSaving={saveMutation.isPending}
            saveError={saveMutation.isError ? String(saveMutation.error) : null}
            version={brief?.brief_version}
            modifiedAt={brief?.brief_modified_at}
          />
        )}
        {activeTab === 'tasks' && (
          <TasksTab tasks={tasks} onTaskClick={(t) => setSelectedTask(t)} />
        )}
        {activeTab === 'overview' && <OverviewTab mission={mission} />}
      </div>

      <TaskSlideOver
        task={selectedTask}
        isOpen={selectedTask !== null}
        onClose={() => setSelectedTask(null)}
      />
    </div>
  );
}

function TasksTab({
  tasks,
  onTaskClick,
}: { tasks: TaskRecord[]; onTaskClick: (t: TaskRecord) => void }) {
  if (tasks.length === 0) {
    return <div style={{ color: 'var(--dim)', fontSize: 13 }}>No tasks in this mission.</div>;
  }
  return (
    <div>
      {tasks.map((t) => (
        <button
          key={t.id}
          type="button"
          data-testid={`task-row-${t.id}`}
          onClick={() => onTaskClick(t)}
          style={{
            display: 'flex', alignItems: 'center', gap: 10, padding: '8px 10px',
            borderRadius: 5, marginBottom: 2, width: '100%', textAlign: 'left',
            background: 'var(--raised)', border: 'none', cursor: 'pointer', fontFamily: 'var(--font)',
          }}
        >
          <span style={{ fontSize: 13, color: 'var(--text)', flex: 1 }}>{t.title}</span>
          <span style={{ fontSize: 11, color: 'var(--accent)', background: 'var(--accent-dim)', padding: '1px 5px', borderRadius: 3 }}>
            {t.status}
          </span>
          {t.owner && <span style={{ fontSize: 11, color: 'var(--dim)' }}>{t.owner}</span>}
        </button>
      ))}
    </div>
  );
}

function OverviewTab({ mission }: { mission: ExplorerMission }) {
  return (
    <dl style={{ display: 'grid', gridTemplateColumns: 'max-content 1fr', gap: '6px 16px', fontSize: 13 }}>
      <dt style={{ color: 'var(--dim)' }}>Description</dt>
      <dd style={{ margin: 0, color: 'var(--text)' }}>{mission.description || '—'}</dd>
      <dt style={{ color: 'var(--dim)' }}>Status</dt>
      <dd style={{ margin: 0, color: 'var(--accent)' }}>{mission.status}</dd>
      <dt style={{ color: 'var(--dim)' }}>Owners</dt>
      <dd style={{ margin: 0, color: 'var(--text)' }}>{mission.owners || '—'}</dd>
      <dt style={{ color: 'var(--dim)' }}>Updated</dt>
      <dd style={{ margin: 0, color: 'var(--text)' }}>{new Date(mission.updated_at).toLocaleString()}</dd>
    </dl>
  );
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose "missions.\$missionId.test" 2>&1 | tail -15
```

Expected: PASS all 4 tests.

- [ ] **Step 5: Full test suite**

```bash
cd /home/merlin/code/edgeplane/web && npm test 2>&1 | tail -10
```

- [ ] **Step 6: Commit**

```bash
cd /home/merlin/code/edgeplane && git add "web/src/routes/domains.\$domainId.missions.\$missionId.tsx" "web/src/routes/domains.\$domainId.missions.\$missionId.test.tsx"
git commit -m "$(cat <<'EOF'
feat(web): add MissionPage with Brief/Tasks/Overview tabs and TaskSlideOver

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: NodesTable component

Linear-density table for listing runtime nodes.

**Files:**
- Create: `web/src/components/nodes/NodesTable.tsx`
- Create: `web/src/components/nodes/NodesTable.test.tsx`

- [ ] **Step 1: Write failing test**

Create `web/src/components/nodes/NodesTable.test.tsx`:

```typescript
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { NodesTable } from './NodesTable';

const sampleNode = {
  id: 'node-uuid-1', node_name: 'excalibur', hostname: 'excalibur.local',
  status: 'online', trust_tier: 'admin', runtime_version: '0.7.0',
  tailscale_fqdn: 'excalibur.hartley-neon.ts.net', tailscale_ip: '100.64.0.1',
  last_heartbeat_at: '2026-06-09T10:00:00Z', owner_subject: 'merlin',
  registered_at: '2026-01-01T00:00:00Z', updated_at: '2026-06-09T10:00:00Z',
  capabilities: [], capacity: {}, labels: {},
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
    expect(screen.getByText('excalibur')).toBeInTheDocument();
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
```

- [ ] **Step 2: Run test, confirm fail**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose NodesTable 2>&1 | tail -10
```

- [ ] **Step 3: Create NodesTable.tsx**

```typescript
import type { components } from '@/api/schema.gen';

type RuntimeNode = components['schemas']['RuntimeNode'];

function statusColor(status: string): string {
  const v = status.toLowerCase();
  if (v === 'online') return 'var(--ok)';
  if (v === 'offline' || v === 'cordoned') return 'var(--err)';
  if (v === 'draining') return 'var(--warn)';
  return 'var(--dim)';
}

function relativeTime(iso: string | null | undefined): string {
  if (!iso) return '—';
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

interface NodesTableProps {
  nodes: RuntimeNode[];
  isLoading: boolean;
  onRowClick: (node: RuntimeNode) => void;
}

export function NodesTable({ nodes, isLoading, onRowClick }: NodesTableProps) {
  if (isLoading) {
    return <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>Loading…</div>;
  }
  if (nodes.length === 0) {
    return <div data-testid="empty-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>No nodes registered.</div>;
  }

  return (
    <div data-testid="nodes-table" style={{ overflowX: 'auto' }}>
      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 13 }}>
        <thead>
          <tr style={{ borderBottom: '1px solid var(--border)' }}>
            {['Status', 'Name', 'Tailscale FQDN', 'Version', 'Heartbeat'].map((col) => (
              <th key={col} style={{ padding: '8px 12px', textAlign: 'left', color: 'var(--dim)', fontWeight: 510, fontSize: 11 }}>
                {col}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {nodes.map((node) => (
            <tr
              key={node.id}
              data-testid={`node-row-${node.id}`}
              onClick={() => onRowClick(node)}
              style={{ borderBottom: '1px solid var(--border-subtle)', cursor: 'pointer' }}
            >
              <td style={{ padding: '8px 12px' }}>
                <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
                  <span style={{ width: 7, height: 7, borderRadius: '50%', background: statusColor(node.status), flexShrink: 0 }} />
                  <span style={{ color: statusColor(node.status), fontSize: 11 }}>{node.status}</span>
                </span>
              </td>
              <td style={{ padding: '8px 12px', color: 'var(--text)', fontWeight: 510 }}>{node.node_name}</td>
              <td style={{ padding: '8px 12px', color: 'var(--text-2)', fontFamily: 'var(--mono)', fontSize: 12 }}>
                {node.tailscale_fqdn ?? '—'}
              </td>
              <td style={{ padding: '8px 12px', color: 'var(--text-2)', fontFamily: 'var(--mono)', fontSize: 12 }}>
                {node.runtime_version}
              </td>
              <td style={{ padding: '8px 12px', color: 'var(--dim)', fontSize: 12 }}>
                {relativeTime(node.last_heartbeat_at)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose NodesTable 2>&1 | tail -10
```

Expected: PASS all 4 tests.

- [ ] **Step 5: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/components/nodes/
git commit -m "$(cat <<'EOF'
feat(web): add NodesTable component

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Nodes layout + NodesPage (nodes.tsx + nodes.index.tsx)

**Files:**
- Create: `web/src/routes/nodes.tsx`
- Create: `web/src/routes/nodes.index.tsx`
- Create: `web/src/routes/nodes.test.tsx`

- [ ] **Step 1: Write failing test**

Create `web/src/routes/nodes.test.tsx`:

```typescript
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    createFileRoute: (_path: string) => (opts: { component: React.ComponentType }) => ({ ...opts, id: _path }),
    useNavigate: vi.fn(() => vi.fn()),
  };
});

vi.mock('@/api/client', () => ({
  apiClient: { GET: vi.fn(), POST: vi.fn().mockResolvedValue({ data: { ok: true } }), use: vi.fn() },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

import { NodesIndexPage } from './nodes.index';

const sampleNode = {
  id: 'node-uuid-1', node_name: 'excalibur', hostname: 'excalibur.local',
  status: 'online', trust_tier: 'admin', runtime_version: '0.7.0',
  tailscale_fqdn: 'excalibur.hartley-neon.ts.net', tailscale_ip: '100.64.0.1',
  last_heartbeat_at: '2026-06-09T10:00:00Z', owner_subject: 'merlin',
  registered_at: '2026-01-01T00:00:00Z', updated_at: '2026-06-09T10:00:00Z',
  capabilities: [], capacity: {}, labels: {},
};

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('NodesIndexPage', () => {
  let qc: QueryClient;
  beforeEach(() => { qc = makeQC(); vi.clearAllMocks(); });
  afterEach(() => qc.clear());

  it('shows loading state while nodes are fetching', async () => {
    const { apiClient } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    wrap(<NodesIndexPage />, qc);
    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('renders node rows when data arrives', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({ data: [sampleNode] });
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue([sampleNode]);
    wrap(<NodesIndexPage />, qc);
    await waitFor(() => expect(screen.getByTestId('node-row-node-uuid-1')).toBeInTheDocument());
    expect(screen.getByText('excalibur')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test, confirm fail**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose nodes.test 2>&1 | tail -10
```

- [ ] **Step 3: Create nodes.tsx**

```typescript
import { Outlet, createFileRoute } from '@tanstack/react-router';

export const Route = createFileRoute('/nodes')({
  component: () => <Outlet />,
});
```

- [ ] **Step 4: Create nodes.index.tsx**

```typescript
import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { NodesTable } from '@/components/nodes/NodesTable';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, useNavigate } from '@tanstack/react-router';

type RuntimeNode = components['schemas']['RuntimeNode'];

export const Route = createFileRoute('/nodes/')({
  component: NodesIndexPage,
});

export function NodesIndexPage() {
  const navigate = useNavigate();

  const { data, isLoading, isError } = useQuery({
    queryKey: queryKeys.nodes.list(),
    queryFn: () => unwrap(apiClient.GET('/api/runtime/nodes', {})),
    refetchInterval: 30_000,
  });

  const nodes: RuntimeNode[] = Array.isArray(data) ? data : [];

  if (isError) {
    return <div data-testid="error-state" style={{ padding: 24, color: 'var(--err)', fontSize: 13 }}>Failed to load nodes.</div>;
  }

  return (
    <div style={{ padding: '16px 24px' }}>
      <div style={{ marginBottom: 12 }}>
        <span style={{ fontSize: 11, fontWeight: 590, color: 'var(--dim)', letterSpacing: '0.06em', textTransform: 'uppercase' }}>
          Fleet Nodes
        </span>
      </div>
      <NodesTable
        nodes={nodes}
        isLoading={isLoading}
        onRowClick={(node) => navigate({ to: '/nodes/$nodeId', params: { nodeId: node.id } })}
      />
    </div>
  );
}
```

- [ ] **Step 5: Run test, confirm pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose nodes.test 2>&1 | tail -10
```

Expected: PASS all 2 tests.

- [ ] **Step 6: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/routes/nodes.tsx web/src/routes/nodes.index.tsx web/src/routes/nodes.test.tsx
git commit -m "$(cat <<'EOF'
feat(web): add NodesPage list route

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: NodeDetailPage (nodes.$nodeId.tsx)

Header: hostname, status, tailscale info, trust tier, version. Agents section filtered by node. Info section: capacity JSON as key/value.

**Files:**
- Create: `web/src/routes/nodes.$nodeId.tsx`
- Create: `web/src/routes/nodes.$nodeId.test.tsx`

- [ ] **Step 1: Write failing test**

Create `web/src/routes/nodes.$nodeId.test.tsx`:

```typescript
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen, waitFor } from '@testing-library/react';
import type React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    createFileRoute: (_path: string) => (opts: { component: React.ComponentType }) => ({ ...opts, id: _path }),
    useParams: vi.fn(() => ({ nodeId: 'node-uuid-1' })),
    Link: ({ to, children, style }: { to: string; children: React.ReactNode; style?: React.CSSProperties }) => (
      <a href={to} style={style}>{children}</a>
    ),
  };
});

vi.mock('@/api/client', () => ({
  apiClient: { GET: vi.fn(), POST: vi.fn().mockResolvedValue({ data: { ok: true } }), use: vi.fn() },
  unwrap: vi.fn((p: unknown) => Promise.resolve(p)),
}));

const mockShowToast = vi.fn();
vi.mock('@/stores/toast', () => ({
  useToastStore: (selector: (s: { show: typeof mockShowToast }) => unknown) =>
    selector({ show: mockShowToast }),
}));

import { NodeDetailPage } from './nodes.$nodeId';

const sampleNode = {
  id: 'node-uuid-1', node_name: 'excalibur', hostname: 'excalibur.local',
  status: 'online', trust_tier: 'admin', runtime_version: '0.7.0',
  tailscale_fqdn: 'excalibur.hartley-neon.ts.net', tailscale_ip: '100.64.0.1',
  last_heartbeat_at: '2026-06-09T10:00:00Z', owner_subject: 'merlin',
  registered_at: '2026-01-01T00:00:00Z', updated_at: '2026-06-09T10:00:00Z',
  capabilities: ['acp', 'mesh'], capacity: { cpu: 16, memory_gb: 64 }, labels: { env: 'prod' },
};

const sampleAgent = {
  id: 1, public_id: 'aria-operator-e8820c0d', name: 'aria-operator',
  status: 'online', capabilities: 'fleet-management',
  metadata: JSON.stringify({ runtime: 'claude-code', node_id: 'excalibur' }),
  home_domain_id: 'dom-abc', current_domain_id: null,
  created_at: '2026-01-01T00:00:00Z', updated_at: '2026-06-09T10:00:00Z',
};

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });
}
function wrap(node: React.ReactElement, qc: QueryClient) {
  return render(<QueryClientProvider client={qc}>{node}</QueryClientProvider>);
}

describe('NodeDetailPage', () => {
  let qc: QueryClient;
  beforeEach(() => {
    qc = makeQC(); vi.clearAllMocks();
    global.fetch = vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve(sampleNode) });
  });
  afterEach(() => qc.clear());

  it('shows loading state while fetching', async () => {
    global.fetch = vi.fn().mockReturnValue(new Promise(() => {}));
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    (unwrap as ReturnType<typeof vi.fn>).mockReturnValue(new Promise(() => {}));
    wrap(<NodeDetailPage />, qc);
    expect(screen.getByTestId('loading-state')).toBeInTheDocument();
  });

  it('renders node hostname and status', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({ data: [sampleAgent] });
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue([sampleAgent]);
    wrap(<NodeDetailPage />, qc);
    await waitFor(() => expect(screen.getByTestId('node-detail-header')).toBeInTheDocument());
    expect(screen.getByText('excalibur')).toBeInTheDocument();
    expect(screen.getByText('online')).toBeInTheDocument();
    expect(screen.getByText('0.7.0')).toBeInTheDocument();
  });

  it('shows agents assigned to this node', async () => {
    const { apiClient, unwrap } = await import('@/api/client');
    (apiClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({ data: [sampleAgent] });
    (unwrap as ReturnType<typeof vi.fn>).mockResolvedValue([sampleAgent]);
    wrap(<NodeDetailPage />, qc);
    await waitFor(() => expect(screen.getByTestId('node-agents-section')).toBeInTheDocument());
    expect(screen.getByText('aria-operator')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test, confirm fail**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose "nodes.\$nodeId.test" 2>&1 | tail -10
```

- [ ] **Step 3: Create nodes.$nodeId.tsx**

The node detail endpoint `/api/runtime/nodes/:id` is NOT in the openapi schema; use raw `fetch`.

```typescript
import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useQuery } from '@tanstack/react-query';
import { Link, createFileRoute, useParams } from '@tanstack/react-router';

type Agent = components['schemas']['Agent'];
type RuntimeNode = components['schemas']['RuntimeNode'];

export const Route = createFileRoute('/nodes/$nodeId')({
  component: NodeDetailPage,
});

export function NodeDetailPage() {
  const { nodeId } = useParams({ from: '/nodes/$nodeId' });

  const { data: node, isLoading } = useQuery({
    queryKey: queryKeys.nodes.detail(nodeId),
    queryFn: async (): Promise<RuntimeNode> => {
      const res = await fetch(`/api/runtime/nodes/${nodeId}`, { credentials: 'include' });
      if (!res.ok) throw new Error(`node fetch failed: ${res.status}`);
      return res.json();
    },
  });

  const { data: allAgents } = useQuery({
    queryKey: queryKeys.agents.list(),
    queryFn: () => unwrap(apiClient.GET('/api/agents', {})),
  });

  const nodeAgents = ((allAgents ?? []) as Agent[]).filter((a) => {
    try {
      const meta = JSON.parse(a.metadata ?? '{}');
      return meta.node_id === node?.node_name;
    } catch {
      return false;
    }
  });

  if (isLoading || !node) {
    return <div data-testid="loading-state" style={{ padding: 24, color: 'var(--dim)', fontSize: 13 }}>Loading…</div>;
  }

  const capacityEntries = (() => {
    try { return Object.entries(node.capacity as Record<string, unknown>); }
    catch { return []; }
  })();

  return (
    <div style={{ padding: '16px 24px' }}>
      <div style={{ fontSize: 11, color: 'var(--dim)', marginBottom: 12 }}>
        <Link to="/nodes" style={{ color: 'var(--dim)', textDecoration: 'none' }}>Nodes</Link>
        {' › '}
        <span style={{ color: 'var(--text)' }}>{node.node_name}</span>
      </div>

      <div data-testid="node-detail-header" style={{ marginBottom: 24 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8 }}>
          <span style={{ fontSize: 18, fontWeight: 590, color: 'var(--text)' }}>{node.node_name}</span>
          <span style={{ fontSize: 11, color: 'var(--ok)', background: 'rgba(87,208,138,0.12)', padding: '1px 7px', borderRadius: 3 }}>
            {node.status}
          </span>
          <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)', fontFamily: 'var(--mono)' }}>
            v{node.runtime_version}
          </span>
        </div>
        <dl style={{ display: 'grid', gridTemplateColumns: 'max-content 1fr', gap: '5px 16px', fontSize: 13 }}>
          <dt style={{ color: 'var(--dim)' }}>Hostname</dt>
          <dd style={{ margin: 0, color: 'var(--text)' }}>{node.hostname}</dd>
          {node.tailscale_fqdn && (
            <>
              <dt style={{ color: 'var(--dim)' }}>Tailscale FQDN</dt>
              <dd style={{ margin: 0, color: 'var(--text)', fontFamily: 'var(--mono)', fontSize: 12 }}>{node.tailscale_fqdn}</dd>
            </>
          )}
          {node.tailscale_ip && (
            <>
              <dt style={{ color: 'var(--dim)' }}>Tailscale IP</dt>
              <dd style={{ margin: 0, color: 'var(--text)', fontFamily: 'var(--mono)', fontSize: 12 }}>{node.tailscale_ip}</dd>
            </>
          )}
          <dt style={{ color: 'var(--dim)' }}>Trust Tier</dt>
          <dd style={{ margin: 0, color: 'var(--accent)' }}>{node.trust_tier}</dd>
        </dl>
      </div>

      <div data-testid="node-agents-section" style={{ marginBottom: 24 }}>
        <div style={{ fontSize: 11, fontWeight: 590, color: 'var(--dim)', letterSpacing: '0.06em', textTransform: 'uppercase', marginBottom: 8 }}>
          Agents ({nodeAgents.length})
        </div>
        {nodeAgents.length === 0 ? (
          <div style={{ color: 'var(--dim)', fontSize: 13 }}>No agents on this node.</div>
        ) : (
          nodeAgents.map((a) => (
            <div
              key={a.public_id}
              style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '7px 10px', borderRadius: 5, background: 'var(--raised)', marginBottom: 2, fontSize: 13 }}
            >
              <span style={{ color: 'var(--text)', fontWeight: 510 }}>{a.name}</span>
              <span style={{ fontSize: 11, color: 'var(--ok)', background: 'rgba(87,208,138,0.12)', padding: '1px 5px', borderRadius: 3 }}>
                {a.status}
              </span>
              <Link
                to="/agents/$agentId"
                params={{ agentId: a.public_id }}
                style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--accent)', textDecoration: 'none' }}
              >
                View →
              </Link>
            </div>
          ))
        )}
      </div>

      {capacityEntries.length > 0 && (
        <div>
          <div style={{ fontSize: 11, fontWeight: 590, color: 'var(--dim)', letterSpacing: '0.06em', textTransform: 'uppercase', marginBottom: 8 }}>
            Capacity
          </div>
          <dl style={{ display: 'grid', gridTemplateColumns: 'max-content 1fr', gap: '5px 16px', fontSize: 13 }}>
            {capacityEntries.map(([k, v]) => (
              <>
                <dt key={`k-${k}`} style={{ color: 'var(--dim)' }}>{k}</dt>
                <dd key={`v-${k}`} style={{ margin: 0, color: 'var(--text)', fontFamily: 'var(--mono)', fontSize: 12 }}>
                  {String(v)}
                </dd>
              </>
            ))}
          </dl>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose "nodes.\$nodeId.test" 2>&1 | tail -15
```

Expected: PASS all 3 tests.

- [ ] **Step 5: Full suite + build**

```bash
cd /home/merlin/code/edgeplane/web && npm test && npm run build 2>&1 | tail -10
```

- [ ] **Step 6: Commit**

```bash
cd /home/merlin/code/edgeplane && git add "web/src/routes/nodes.\$nodeId.tsx" "web/src/routes/nodes.\$nodeId.test.tsx"
git commit -m "$(cat <<'EOF'
feat(web): add NodeDetailPage with agents and capacity info

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Update Sidebar — inline domain tree + Nodes icon

The Sidebar needs: (1) Nodes icon added to `NAV_ICON`, (2) the `/domains` nav item replaced with a `SidebarDomainsSection` that renders the inline expandable tree.

**Files:**
- Modify: `web/src/components/shell/Sidebar.tsx`
- Modify: `web/src/components/shell/Sidebar.test.tsx`

- [ ] **Step 1: Update Sidebar.test.tsx to add QueryClientProvider and new assertions**

Replace the full file:

```typescript
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen } from '@testing-library/react';
import type React from 'react';
import { describe, expect, it, vi } from 'vitest';

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
      userName: string | null;
      logout: () => Promise<void>;
    }) => unknown,
  ) =>
    selector({
      userSubject: '73c5a571f3b774a535810a3835f3b8fa',
      userEmail: null,
      userName: null,
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
  return new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } });
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
  it('returns null for null input', () => { expect(avatarLabel(null)).toBeNull(); });
  it('returns null for an opaque hash', () => { expect(avatarLabel('73c5a571f3b774a535810a3835f3b8fa')).toBeNull(); });
  it('returns RM for an email address', () => { expect(avatarLabel('ryan.merlin@example.com')).toBe('RM'); });
  it('returns RM for a display name with spaces', () => { expect(avatarLabel('Ryan Merlin')).toBe('RM'); });
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
```

- [ ] **Step 2: Run Sidebar.test.tsx — confirms existing tests still pass (nav-/nodes may fail)**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose Sidebar.test 2>&1 | tail -15
```

Expected: Most pass; `renders /nodes nav item` fails because Sidebar doesn't have it yet.

- [ ] **Step 3: Update Sidebar.tsx**

Replace the `NAV_ICON` map and the nav items rendering section. The full updated file:

```typescript
import { apiClient, unwrap } from '@/api/client';
import type { components } from '@/api/schema.gen';
import { queryKeys } from '@/lib/queryKeys';
import { useAuthStore } from '@/stores/auth';
import { useQuery } from '@tanstack/react-query';
import { Link, useRouterState } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { NAV_GROUPS, isNavItemActive } from './navModel';

type ExplorerDomainNode = components['schemas']['ExplorerDomainNode'];
type ExplorerMissionNode = components['schemas']['ExplorerMissionNode'];

// ── Helpers ─────────────────────────────────────────────────────────────────

export function avatarLabel(email: string | null): string | null {
  if (!email) return null;
  if (/^[0-9a-f]{24,}$/i.test(email)) return null;
  const atIdx = email.indexOf('@');
  const local = atIdx > 0 ? email.slice(0, atIdx) : email;
  const parts = local.split(/[._\-\s]+/).filter(Boolean);
  if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
  if (parts.length === 1 && /^[a-zA-Z]+$/.test(parts[0])) return parts[0][0].toUpperCase();
  return null;
}

function applyTheme(next: string) {
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = next;
    localStorage.setItem('edgeplane:theme', next);
  }
}

// ── Nav icon map ─────────────────────────────────────────────────────────────
const NAV_ICON: Record<string, string> = {
  '/': '◇',
  '/agents': '◉',
  '/nodes': '▦',
  '/domains': '▤',
  '/feed': '≋',
  '/governance': '⚖',
};

// ── Inline Domains tree ──────────────────────────────────────────────────────

function SidebarDomainsSection({ pathname }: { pathname: string }) {
  const isActive = isNavItemActive('/domains', pathname);
  const { data: tree } = useQuery({
    queryKey: queryKeys.explorer.tree(),
    queryFn: () => unwrap(apiClient.GET('/api/explorer/tree', {})),
    refetchInterval: 30_000,
  });

  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());

  const toggle = (id: string) =>
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <div>
      {/* DOMAINS section label */}
      <Link
        to="/domains"
        data-testid="nav-/domains"
        aria-current={pathname === '/domains' ? 'page' : undefined}
        style={{
          display: 'flex', alignItems: 'center', gap: 9, height: 28,
          padding: '0 8px', borderRadius: 6,
          color: isActive ? 'var(--text)' : 'var(--text-2)',
          fontSize: 13, fontWeight: 510, textDecoration: 'none',
          background: pathname === '/domains' ? 'var(--raised-2)' : 'transparent',
          userSelect: 'none',
        }}
      >
        <span style={{ width: 15, height: 15, flexShrink: 0, color: isActive ? 'var(--accent)' : 'var(--dim)', display: 'grid', placeItems: 'center', fontSize: 13 }}>
          {NAV_ICON['/domains']}
        </span>
        Domains
      </Link>

      {/* Domain tree items */}
      {tree?.domains.map((domain: ExplorerDomainNode) => {
        const domainActive = pathname.startsWith(`/domains/${domain.id}`);
        const expanded = expandedIds.has(domain.id);
        return (
          <div key={domain.id}>
            <div style={{ display: 'flex', alignItems: 'center' }}>
              <button
                type="button"
                onClick={() => toggle(domain.id)}
                style={{ background: 'none', border: 'none', color: 'var(--dim)', fontSize: 10, cursor: 'pointer', padding: '0 2px 0 20px', flexShrink: 0 }}
              >
                {expanded ? '▾' : '▸'}
              </button>
              <Link
                to="/domains/$domainId"
                params={{ domainId: domain.id }}
                style={{
                  flex: 1, display: 'flex', alignItems: 'center', height: 26,
                  padding: '0 8px 0 2px', borderRadius: 5, fontSize: 12,
                  color: domainActive ? 'var(--text)' : 'var(--text-2)',
                  textDecoration: 'none', fontWeight: domainActive ? 510 : 400,
                  background: domainActive ? 'var(--raised-2)' : 'transparent',
                }}
              >
                {domain.name}
              </Link>
            </div>
            {expanded && domain.missions.map((m: ExplorerMissionNode) => {
              const mActive = pathname.includes(`/missions/${m.id}`);
              return (
                <Link
                  key={m.id}
                  to="/domains/$domainId/missions/$missionId"
                  params={{ domainId: domain.id, missionId: m.id }}
                  style={{
                    display: 'flex', alignItems: 'center', height: 24,
                    padding: '0 8px 0 38px', borderRadius: 5, fontSize: 11,
                    color: mActive ? 'var(--text)' : 'var(--text-2)',
                    textDecoration: 'none',
                    background: mActive ? 'var(--raised-2)' : 'transparent',
                  }}
                >
                  <span style={{ width: 6, height: 6, borderRadius: '50%', background: 'var(--dim)', marginRight: 6, flexShrink: 0 }} />
                  {m.name}
                </Link>
              );
            })}
          </div>
        );
      })}
    </div>
  );
}

// ── Sidebar ──────────────────────────────────────────────────────────────────

export function Sidebar() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const userSubject = useAuthStore((s) => s.userSubject);
  const userEmail = useAuthStore((s) => s.userEmail);
  const userName = useAuthStore((s) => s.userName);
  const logout = useAuthStore((s) => s.logout);

  const [showMenu, setShowMenu] = useState(false);
  const [_theme, setTheme] = useState('dark');
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const saved = localStorage.getItem('edgeplane:theme');
    const initial = saved === 'light' ? 'light' : 'dark';
    setTheme(initial);
    applyTheme(initial);
  }, []);

  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (showMenu && menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setShowMenu(false);
      }
    }
    document.addEventListener('click', handleClick);
    return () => document.removeEventListener('click', handleClick);
  }, [showMenu]);

  const toggleTheme = () => {
    setTheme((prev) => {
      const next = prev === 'dark' ? 'light' : 'dark';
      applyTheme(next);
      return next;
    });
  };

  const label = avatarLabel(userName ?? userEmail);

  return (
    <nav
      data-testid="sidebar"
      style={{
        width: 'var(--sidebar, 232px)', height: '100%',
        display: 'flex', flexDirection: 'column',
        background: 'var(--frame)', borderRight: '1px solid var(--border)',
        flexShrink: 0, padding: '10px 8px 8px', gap: 2, boxSizing: 'border-box',
      }}
    >
      {/* Brand row */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 8px 10px' }}>
        <span style={{ color: 'var(--accent)', fontSize: 16, lineHeight: 1 }}>⬡</span>
        <span style={{ fontSize: '13.5px', fontWeight: 590, color: 'var(--text)', letterSpacing: '-0.01em' }}>
          EdgePlane
        </span>
      </div>

      {/* Search */}
      <button
        type="button"
        aria-label="Search"
        onClick={() => {}}
        style={{
          display: 'flex', alignItems: 'center', gap: 8, height: 30, padding: '0 8px', marginBottom: 8,
          background: 'var(--input)', border: '1px solid var(--border-subtle)', borderRadius: 6,
          color: 'var(--dim)', fontSize: 13, cursor: 'pointer', width: '100%',
          textAlign: 'left', fontFamily: 'var(--font)',
        }}
      >
        Search…
        <kbd style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--dim)', fontFamily: 'var(--mono)', background: 'none', border: 'none', padding: 0 }}>
          ⌘K
        </kbd>
      </button>

      {/* Nav items */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 1, flex: 1, overflowY: 'auto' }}>
        {NAV_GROUPS.flatMap((g) => g.items).map((item) => {
          if (item.to === '/domains') {
            return <SidebarDomainsSection key="/domains" pathname={pathname} />;
          }
          const active = isNavItemActive(item.to, pathname);
          const icon = NAV_ICON[item.to] ?? '·';
          return (
            <Link
              key={item.to}
              to={item.to}
              data-testid={`nav-${item.to}`}
              aria-current={active ? 'page' : undefined}
              style={{
                display: 'flex', alignItems: 'center', gap: 9, height: 28, padding: '0 8px',
                borderRadius: 6, color: active ? 'var(--text)' : 'var(--text-2)',
                fontSize: 13, fontWeight: 510, textDecoration: 'none',
                background: active ? 'var(--raised-2)' : 'transparent',
                cursor: 'pointer', userSelect: 'none', transition: 'background .12s ease, color .12s ease',
              }}
              onMouseEnter={(e) => { if (!active) (e.currentTarget as HTMLElement).style.background = 'var(--raised)'; }}
              onMouseLeave={(e) => { if (!active) (e.currentTarget as HTMLElement).style.background = 'transparent'; }}
            >
              <span style={{ width: 15, height: 15, flexShrink: 0, color: active ? 'var(--accent)' : 'var(--dim)', display: 'grid', placeItems: 'center', fontSize: 13, transition: 'color .12s ease' }}>
                {icon}
              </span>
              {item.label}
            </Link>
          );
        })}
      </div>

      {/* Bottom account control */}
      <div ref={menuRef} style={{ marginTop: 'auto', position: 'relative' }}>
        {showMenu && (
          <div
            role="menu"
            data-testid="account-menu"
            style={{
              position: 'absolute', bottom: 42, left: 4, right: 4,
              background: 'var(--frame)', border: '1px solid var(--border)',
              borderRadius: 8, padding: 4, zIndex: 20,
              boxShadow: '0 8px 28px rgba(0,0,0,0.5)',
              display: 'flex', flexDirection: 'column', alignItems: 'stretch',
            }}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => { e.stopPropagation(); if (e.key === 'Escape') setShowMenu(false); }}
          >
            <button type="button" role="menuitem" data-testid="menu-preferences"
              style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-start', gap: 8, padding: '6px 8px', borderRadius: 5, fontSize: 13, color: 'var(--text-2)', cursor: 'pointer', width: '100%', background: 'transparent', border: 'none', fontFamily: 'var(--font)' }}>
              Preferences
            </button>
            <Link to="/onboarding" data-testid="menu-onboarding" role="menuitem"
              style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 8px', borderRadius: 5, fontSize: 13, color: 'var(--text-2)', cursor: 'pointer', textDecoration: 'none', width: '100%', boxSizing: 'border-box' }}>
              Onboarding
            </Link>
            <button type="button" role="menuitem" data-testid="theme-item" onClick={toggleTheme}
              style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-start', gap: 8, padding: '6px 8px', borderRadius: 5, fontSize: 13, color: 'var(--text-2)', cursor: 'pointer', width: '100%', background: 'transparent', border: 'none', fontFamily: 'var(--font)' }}>
              ☾ Theme
            </button>
            <button type="button" role="menuitem" data-testid="logout-item"
              onClick={async () => { setShowMenu(false); await logout(); }}
              style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-start', gap: 8, padding: '6px 8px', borderRadius: 5, fontSize: 13, color: 'var(--err)', cursor: 'pointer', width: '100%', background: 'transparent', border: 'none', fontFamily: 'var(--font)' }}>
              Logout
            </button>
          </div>
        )}
        <button type="button" data-testid="account-btn" aria-haspopup="menu" aria-expanded={showMenu}
          onClick={() => setShowMenu((v) => !v)}
          title={userName ?? userEmail ?? userSubject ?? 'User menu'}
          style={{ display: 'flex', alignItems: 'center', gap: 9, height: 34, padding: '0 8px', width: '100%', background: 'transparent', border: 'none', borderRadius: 6, cursor: 'pointer', color: 'var(--text-2)', fontFamily: 'var(--font)', fontSize: 13, textAlign: 'left' }}>
          <span style={{ width: 22, height: 22, borderRadius: 5, background: 'var(--accent-dim)', color: 'var(--accent)', display: 'grid', placeItems: 'center', fontSize: 11, fontWeight: 590, flexShrink: 0 }}>
            {label ?? '⬡'}
          </span>
          <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {userName ?? userEmail ?? 'Account'}
          </span>
          <span style={{ marginLeft: 'auto', color: 'var(--dim)', fontSize: 11 }}>⌄</span>
        </button>
      </div>
    </nav>
  );
}

export default Sidebar;
```

- [ ] **Step 4: Run Sidebar.test.tsx, confirm all pass**

```bash
cd /home/merlin/code/edgeplane/web && npm test -- --reporter=verbose Sidebar.test 2>&1 | tail -15
```

Expected: PASS all tests including `renders /nodes nav item`.

- [ ] **Step 5: Full test suite + build**

```bash
cd /home/merlin/code/edgeplane/web && npm test && npm run build 2>&1 | tail -10
```

Expected: all pass, exit 0.

- [ ] **Step 6: Commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/components/shell/Sidebar.tsx web/src/components/shell/Sidebar.test.tsx
git commit -m "$(cat <<'EOF'
feat(web): update Sidebar with inline domain tree and Nodes nav item

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Final verification + lint

- [ ] **Step 1: Run lint**

```bash
cd /home/merlin/code/edgeplane/web && npm run lint 2>&1 | tail -20
```

Fix any Biome lint errors before proceeding.

- [ ] **Step 2: Run full test suite**

```bash
cd /home/merlin/code/edgeplane/web && npm test 2>&1
```

All tests must pass.

- [ ] **Step 3: Run build**

```bash
cd /home/merlin/code/edgeplane/web && npm run build 2>&1 | tail -10
```

Exit 0, no TypeScript errors.

- [ ] **Step 4: Verify routeTree.gen.ts is regenerated**

TanStack Router's vite plugin regenerates `routeTree.gen.ts` on build. Confirm new routes appear:

```bash
grep -E "domains\.index|domains\.\$domainId|nodes\.index|nodes\.\$nodeId" /home/merlin/code/edgeplane/web/src/routeTree.gen.ts | head -10
```

Expected: All new route names present.

- [ ] **Step 5: Commit lint fixes if any, then final commit**

```bash
cd /home/merlin/code/edgeplane && git add web/src/routeTree.gen.ts
git commit -m "$(cat <<'EOF'
chore(web): regenerate routeTree.gen.ts for domains-v2 + nodes routes

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

## Spec coverage checklist (self-review)

| Spec requirement | Task |
|---|---|
| Nodes nav item between Agents and Domains | Task 2 |
| Domains becomes inline tree in sidebar | Task 12 |
| `/domains` overview page (interactive tree) | Task 5 |
| `/domains/:id` DomainPage with tabs | Task 6 |
| Northstar Monaco editor on DomainPage | Task 6 |
| `/domains/:id/missions/:mid` MissionPage | Task 8 |
| Brief Monaco editor on MissionPage | Task 8 |
| TaskSlideOver for task detail | Task 7 |
| `/nodes` NodesPage list table | Task 10 |
| `/nodes/:nodeId` NodeDetailPage | Task 11 |
| `@monaco-editor/react` dep added | Task 1 |
| `queryKeys` extended | Task 1 |
| `/explorer` redirect preserved | (no change — already working) |
| `domains.tsx` converted to layout | Task 3 |
