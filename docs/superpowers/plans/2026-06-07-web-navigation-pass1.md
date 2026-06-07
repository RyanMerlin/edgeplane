# Web Navigation Redesign — Pass 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Replace the flat top-nav shell with a persistent left-sidebar shell (two co-equal spines: WHO=Agents, WHAT=Domains), with active-state, breadcrumbs, and full-height content — fixing the detail-view navigation bugs structurally.

**Architecture:** A new `components/shell/` owns the app frame (`AppShell` = `Sidebar` + content with a breadcrumb header). Pure, testable modules (`navModel`, `breadcrumbs`) drive active-state and the crumb trail. `__root.tsx` renders `AppShell` instead of `TopBar`. `/` becomes a `Dashboard`; `/agents` returns as a first-class section (index route) keeping the `/agents/$agentId` detail; `/explorer` → `/domains`. Console drops from nav. Built on `feat/web-tab-consolidation`.

**Tech Stack:** React 19, TanStack Router (file routes) + Query, Zustand (auth/theme), Vitest + React Testing Library, Biome. Spec: `docs/superpowers/specs/2026-06-07-web-navigation-design.md`.

---

## File Structure

- `web/src/components/shell/navModel.ts` — nav item list + `isNavItemActive(itemTo, pathname)` prefix matcher. **(pure, unit-tested)**
- `web/src/components/shell/breadcrumbs.ts` — `buildCrumbs({pathname, params, labels})` → crumb array. **(pure, unit-tested)**
- `web/src/components/shell/Breadcrumbs.tsx` — renders a crumb trail from `useRouterState` + a label resolver.
- `web/src/components/shell/Sidebar.tsx` — the rail (groups, active-state, footer = Onboarding + account menu w/ theme + logout + avatar initials/glyph).
- `web/src/components/shell/AppShell.tsx` — `Sidebar` + full-height content column (breadcrumb header + `<Outlet/>`).
- `web/src/routes/__root.tsx` — render `AppShell` (keep auth gate + OIDC); drop `TopBar`/`StatusBar` nav chrome.
- `web/src/routes/index.tsx` — rewrite to `Dashboard` (fleet card + work card + recent activity).
- `web/src/routes/agents.tsx` — layout that renders `<Outlet/>` (no redirect).
- `web/src/routes/agents.index.tsx` — **new** index route → the Agents table.
- `web/src/routes/agents.$agentId.tsx` — full-height detail; drop the standalone back-link.
- `web/src/routes/domains.tsx` — **renamed** from `explorer.tsx` (route `/domains`).
- `web/src/routes/explorer.tsx` — **new** thin redirect `/explorer` → `/domains`.

Existing kept as-is: `lib/useMergedAgents.ts`, `components/fleet/AgentsTable.tsx`, `components/events/RawEventList.tsx`, `routes/feed.tsx`, `routes/governance.tsx`, `routes/onboarding.tsx`, `routes/matrix.tsx` (redirect), `routes/ai.tsx` (kept, unlinked).

**Per-task gate:** `npm run build` (tsc+vite, regenerates `routeTree.gen.ts`) + `npm test` + `npm run lint` must pass before commit. Run from `web/`.

---

### Task 1: Nav model + active-state matcher (pure)

**Files:**
- Create: `web/src/components/shell/navModel.ts`
- Test: `web/src/components/shell/navModel.test.ts`

- [ ] **Step 1: Write the failing test**

```ts
import { describe, expect, it } from 'vitest';
import { NAV_GROUPS, isNavItemActive } from './navModel';

describe('navModel', () => {
  it('exposes Dashboard, Agents, Domains, Feed, Governance (no Console)', () => {
    const tos = NAV_GROUPS.flatMap((g) => g.items).map((i) => i.to);
    expect(tos).toEqual(['/', '/agents', '/domains', '/feed', '/governance']);
  });

  it('matches "/" only exactly', () => {
    expect(isNavItemActive('/', '/')).toBe(true);
    expect(isNavItemActive('/', '/agents')).toBe(false);
  });

  it('matches a section by prefix, including detail routes', () => {
    expect(isNavItemActive('/agents', '/agents')).toBe(true);
    expect(isNavItemActive('/agents', '/agents/aria-operator-bb05ea7a')).toBe(true);
    expect(isNavItemActive('/domains', '/domains/apollo')).toBe(true);
    expect(isNavItemActive('/agents', '/agents-foo')).toBe(false); // prefix must be a path boundary
  });
});
```

- [ ] **Step 2: Run it — expect FAIL** (`npm test -- navModel` → "isNavItemActive is not a function")

- [ ] **Step 3: Implement**

```ts
export interface NavItem {
  to: string;
  label: string;
}
export interface NavGroup {
  /** Group heading; null = no heading (top/secondary groups). */
  heading: string | null;
  items: NavItem[];
}

export const NAV_GROUPS: NavGroup[] = [
  { heading: null, items: [{ to: '/', label: 'Dashboard' }] },
  { heading: 'WHO', items: [{ to: '/agents', label: 'Agents' }] },
  { heading: 'WHAT', items: [{ to: '/domains', label: 'Domains' }] },
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

- [ ] **Step 4: Run it — expect PASS**
- [ ] **Step 5: Commit** — `feat(web): sidebar nav model + active-state matcher`

---

### Task 2: Breadcrumb trail builder (pure)

**Files:**
- Create: `web/src/components/shell/breadcrumbs.ts`
- Test: `web/src/components/shell/breadcrumbs.test.ts`

- [ ] **Step 1: Failing test**

```ts
import { describe, expect, it } from 'vitest';
import { buildCrumbs } from './breadcrumbs';

describe('buildCrumbs', () => {
  it('root → single Dashboard crumb (no link)', () => {
    expect(buildCrumbs('/', {})).toEqual([{ label: 'Dashboard', to: undefined }]);
  });
  it('agents list → Agents (current)', () => {
    expect(buildCrumbs('/agents', {})).toEqual([{ label: 'Agents', to: undefined }]);
  });
  it('agent detail → Agents (link) › id (current)', () => {
    expect(buildCrumbs('/agents/aria-operator-bb05ea7a', { agentId: 'aria-operator-bb05ea7a' })).toEqual([
      { label: 'Agents', to: '/agents' },
      { label: 'aria-operator-bb05ea7a', to: undefined },
    ]);
  });
  it('domains → Domains (current)', () => {
    expect(buildCrumbs('/domains', {})).toEqual([{ label: 'Domains', to: undefined }]);
  });
  it('unknown route → empty', () => {
    expect(buildCrumbs('/feed', {})).toEqual([{ label: 'Feed', to: undefined }]);
  });
});
```

- [ ] **Step 2: Run — expect FAIL**
- [ ] **Step 3: Implement**

```ts
export interface Crumb {
  label: string;
  /** Link target; undefined = current page (not a link). */
  to: string | undefined;
}

const SECTION_LABEL: Record<string, string> = {
  '/': 'Dashboard',
  '/agents': 'Agents',
  '/domains': 'Domains',
  '/feed': 'Feed',
  '/governance': 'Governance',
  '/onboarding': 'Onboarding',
};

/** Build a crumb trail for the current path. `params` supplies detail-id labels. */
export function buildCrumbs(pathname: string, params: Record<string, string>): Crumb[] {
  if (pathname.startsWith('/agents/') && params.agentId) {
    return [
      { label: 'Agents', to: '/agents' },
      { label: params.agentId, to: undefined },
    ];
  }
  const label = SECTION_LABEL[pathname];
  if (label) return [{ label, to: undefined }];
  return [];
}
```

- [ ] **Step 4: Run — expect PASS**
- [ ] **Step 5: Commit** — `feat(web): breadcrumb trail builder`

---

### Task 3: Breadcrumbs component

**Files:**
- Create: `web/src/components/shell/Breadcrumbs.tsx`
- Test: `web/src/components/shell/Breadcrumbs.test.tsx`

- [ ] **Step 1: Failing test** (render with explicit crumbs prop to avoid router context)

```tsx
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
vi.mock('@tanstack/react-router', () => ({
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => <a href={to}>{children}</a>,
}));
import { CrumbTrail } from './Breadcrumbs';

describe('CrumbTrail', () => {
  it('renders links for non-current crumbs and plain text for the current', () => {
    render(<CrumbTrail crumbs={[{ label: 'Agents', to: '/agents' }, { label: 'x', to: undefined }]} />);
    expect(screen.getByRole('link', { name: 'Agents' })).toHaveAttribute('href', '/agents');
    expect(screen.getByText('x')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run — expect FAIL**
- [ ] **Step 3: Implement** — `CrumbTrail` (presentational; takes `crumbs`) + default export `Breadcrumbs` that reads `useRouterState`/`useParams` and calls `buildCrumbs`. Separator `›`. Current crumb = `<span>`; others = `<Link>`. `data-testid="breadcrumbs"`.
- [ ] **Step 4: Run — expect PASS**
- [ ] **Step 5: Commit** — `feat(web): Breadcrumbs component`

---

### Task 4: Sidebar component

**Files:**
- Create: `web/src/components/shell/Sidebar.tsx`
- Test: `web/src/components/shell/Sidebar.test.tsx`

Moves from old `TopBar`: theme toggle, logout, avatar, Onboarding entry. Avatar shows initials from `userSubject` **only if it looks like an email/name**; otherwise a neutral glyph (never a hash slice).

- [ ] **Step 1: Failing test** (mock router `Link`/`useRouterState`, mock auth + toast stores)

```tsx
// renders all 5 nav items grouped, marks the active one, footer has Onboarding + Logout,
// avatar shows a neutral glyph when subject is an opaque hash.
expect(screen.getByTestId('nav-/agents')).toHaveAttribute('aria-current', 'page'); // when pathname=/agents/x
expect(screen.getByTestId('nav-onboarding')).toBeInTheDocument();
expect(screen.getByTestId('logout-item')).toBeInTheDocument();
expect(screen.queryByText(/^73/)).not.toBeInTheDocument(); // no hash-slice initials
```

- [ ] **Step 2: Run — expect FAIL**
- [ ] **Step 3: Implement** — `Sidebar`: logo header; `NAV_GROUPS.map` → groups with optional heading + `<Link data-testid={'nav'+item.to} aria-current={isNavItemActive(item.to, pathname) ? 'page' : undefined}>`; footer with Onboarding `<Link>`, account button (initials-or-glyph) opening a menu (theme toggle + logout). `avatarLabel(subject)`: if `subject` contains `@` or a `.`/`_`/`-`/space separator with letters, derive 2-letter initials; else return `null` → render a neutral `⬡`/user glyph. Reuse auth/toast store hooks from `__root`.
- [ ] **Step 4: Run — expect PASS**
- [ ] **Step 5: Commit** — `feat(web): Sidebar (rail + footer account menu)`

---

### Task 5: AppShell + wire into __root

**Files:**
- Create: `web/src/components/shell/AppShell.tsx`
- Modify: `web/src/routes/__root.tsx`
- Test: `web/src/components/shell/AppShell.test.tsx`

- [ ] **Step 1: Failing test** — `AppShell` renders the sidebar + a content region containing children; content region is the full-height column with the breadcrumb header.

```tsx
// mock Sidebar + Breadcrumbs to simple stubs; assert structure
render(<AppShell><div data-testid="page" /></AppShell>);
expect(screen.getByTestId('app-sidebar-stub')).toBeInTheDocument();
expect(screen.getByTestId('app-content')).toBeInTheDocument();
expect(screen.getByTestId('page')).toBeInTheDocument();
```

- [ ] **Step 2: Run — expect FAIL**
- [ ] **Step 3: Implement**
  - `AppShell.tsx`: `<div className="app-shell-2" style={{display:'flex', height:'100vh'}}><Sidebar/><div data-testid="app-content" style={{flex:1, minWidth:0, display:'flex', flexDirection:'column', height:'100%'}}><header><Breadcrumbs/></header><main style={{flex:1, minHeight:0, overflow:'auto'}}>{children}</main></div></div>`.
  - `__root.tsx`: in the `loggedIn` branch, replace `<TopBar/>...<Outlet/>...<StatusBar/>` with `<AppShell><Outlet/></AppShell>`. Delete the now-unused `TopBar`, `StatusBar`, `NavLink` (move theme/logout/avatar logic into Sidebar). Keep `bootstrapped` splash, `LoginScreen`, OIDC grant effect, `QueryClientProvider`, toast.
- [ ] **Step 4: Run — expect PASS** (`npm test`, then `npm run build`)
- [ ] **Step 5: Commit** — `feat(web): AppShell + mount in root (drop top-nav chrome)`

---

### Task 6: Dashboard route (`/`)

**Files:**
- Modify (rewrite): `web/src/routes/index.tsx`
- Modify (rewrite): `web/src/routes/index.test.tsx`

- [ ] **Step 1: Failing test** — Dashboard renders a Fleet summary (online/total from `useMergedAgents`) and a Work summary, with links to `/agents` and `/domains`.

```tsx
// mock @/api/client (agents), @tanstack/react-router (Link/createFileRoute), explorer tree query source
await waitFor(() => expect(screen.getByTestId('dashboard')).toBeInTheDocument());
expect(screen.getByTestId('dash-fleet-online')).toHaveTextContent('2'); // 2 of 3 online fixture
expect(screen.getByRole('link', { name: /Agents/ })).toHaveAttribute('href', '/agents');
expect(screen.getByRole('link', { name: /Domains/ })).toHaveAttribute('href', '/domains');
```

- [ ] **Step 2: Run — expect FAIL**
- [ ] **Step 3: Implement** — `Dashboard`: `useMergedAgents()` → Fleet card (`online/total`, link `/agents`); a Work card (reuse the explorer tree query — `GET /api/explorer/tree` via apiClient — for domain/mission counts, link `/domains`); a Recent-activity strip from `useEventStream()` (last N events, read-only). Keep it a simple 2-card + activity grid using existing `.tag`/card styles. `data-testid="dashboard"`, `dash-fleet-online`, etc. Remove the old `FleetConsole`/`ViewToggle`/`AgentsTable` import from index (table now lives in `agents.index.tsx`).
- [ ] **Step 4: Run — expect PASS**
- [ ] **Step 5: Commit** — `feat(web): Dashboard landing (fleet + work summary + activity)`

---

### Task 7: Restore `/agents` as a first-class section

**Files:**
- Modify (rewrite): `web/src/routes/agents.tsx` (→ Outlet layout)
- Create: `web/src/routes/agents.index.tsx` (the table)
- Modify: `web/src/routes/agents.test.tsx` (add index-route test; keep detail tests)

- [ ] **Step 1: Failing test** — add to `agents.test.tsx`: `AgentsIndexPage` renders `AgentsTable` from merged agents and navigates to detail on row click.

```tsx
import { AgentsIndexPage } from './agents.index';
// mock apiClient agents + useNavigate; render <AgentsIndexPage/>
await waitFor(() => expect(screen.getByTestId('agents-table')).toBeInTheDocument());
fireEvent.click(screen.getByTestId('agent-row-aria-operator-e8820c0d'));
expect(navigate).toHaveBeenCalledWith({ to: '/agents/$agentId', params: { agentId: 'aria-operator-e8820c0d' } });
```

- [ ] **Step 2: Run — expect FAIL**
- [ ] **Step 3: Implement**
  - `agents.tsx`: `createFileRoute('/agents')({ component: () => <Outlet /> })` (no `beforeLoad` redirect).
  - `agents.index.tsx`: `createFileRoute('/agents/')({ component: AgentsIndexPage })`; `AgentsIndexPage` = `useMergedAgents()` + `<AgentsTable ... onRowClick={(a)=>navigate({to:'/agents/$agentId',params:{agentId:a.public_id}})}/>` (named export for tests).
- [ ] **Step 4: Run — expect PASS** (`npm test`, then `npm run build` to regenerate `routeTree.gen.ts` with the new index route)
- [ ] **Step 5: Commit** — `feat(web): /agents first-class section (index route + outlet layout)`

---

### Task 8: Agent detail — full height, breadcrumb-driven

**Files:**
- Modify: `web/src/routes/agents.$agentId.tsx`
- Modify: `web/src/routes/agents.test.tsx`

- [ ] **Step 1: Failing test** — detail container is full-height and the standalone back-link is gone (breadcrumbs in the shell handle up-nav).

```tsx
// existing "renders a single agent record" test: replace the back-link assertion with:
expect(screen.queryByRole('link', { name: /← Fleet/ })).not.toBeInTheDocument();
expect(screen.getByTestId('agent-detail')).toBeInTheDocument();
```

- [ ] **Step 2: Run — expect FAIL**
- [ ] **Step 3: Implement** — wrap the detail in `<div data-testid="agent-detail" style={{display:'flex', flexDirection:'column', height:'100%', minHeight:0}}>`; remove the `<Link to="/">← Fleet</Link>` (and its now-unused `Link` import if unused); keep the identity/status header + the ACP conversation pane as the flex body (`flex:1, minHeight:0`).
- [ ] **Step 4: Run — expect PASS**
- [ ] **Step 5: Commit** — `feat(web): full-height agent detail; breadcrumb-driven up-nav`

---

### Task 9: Rename Explorer → Domains (+ redirect)

**Files:**
- Rename: `web/src/routes/explorer.tsx` → `web/src/routes/domains.tsx`
- Create: `web/src/routes/explorer.tsx` (redirect)
- Rename: `web/src/routes/explorer.test.tsx` → `web/src/routes/domains.test.tsx`

- [ ] **Step 1:** `git mv web/src/routes/explorer.tsx web/src/routes/domains.tsx` and `git mv web/src/routes/explorer.test.tsx web/src/routes/domains.test.tsx`.
- [ ] **Step 2:** In `domains.tsx` change `createFileRoute('/explorer')` → `createFileRoute('/domains')`. In `domains.test.tsx` update the import path/route id and any `/explorer` literals to `/domains`. Run `npm test -- domains` → PASS.
- [ ] **Step 3:** Create `web/src/routes/explorer.tsx`:

```tsx
import { createFileRoute, redirect } from '@tanstack/react-router';
export const Route = createFileRoute('/explorer')({
  beforeLoad: () => {
    throw redirect({ to: '/domains' });
  },
});
```

- [ ] **Step 4:** `npm run build` (regenerates routeTree with `/domains` + `/explorer` redirect) → PASS; `npm test`; `npm run lint`.
- [ ] **Step 5: Commit** — `feat(web): rename Explorer route to /domains (+ /explorer redirect)`

---

### Task 10: Final integration sweep

**Files:** none new — verification + cleanup.

- [ ] **Step 1:** Grep for stale references: `grep -rn "TopBar\|StatusBar\|FleetConsole\|ViewToggle\|fleet-view-\|nav-tab" web/src` — remove/justify each (the Fleet view-toggle + console are intentionally gone; update any test that referenced them).
- [ ] **Step 2:** `npm run build` — PASS (clean `routeTree.gen.ts`).
- [ ] **Step 3:** `npm test` — all PASS (update counts as needed).
- [ ] **Step 4:** `npm run lint` — PASS (run `npx biome check --write src/` for safe format/import fixes if needed).
- [ ] **Step 5: Commit** — `chore(web): nav redesign Pass 1 integration (green build/test/lint)`

---

## Self-Review (spec coverage)

- Sidebar shell + two spines → Tasks 1,3,4,5. ✓
- Active-state into detail → Task 1 (`isNavItemActive`) + Task 3 (`aria-current`). ✓
- Breadcrumbs replace back-button → Tasks 2,3 + Task 8 (remove link). ✓
- Full-height content (half-page fix) → Task 5 (AppShell) + Task 8 (detail). ✓
- Dashboard landing → Task 6. ✓
- `/agents` first-class + detail preserved → Tasks 7,8. ✓
- Explorer → Domains → Task 9. ✓
- Console dropped from nav → Task 1 (omitted from NAV_GROUPS); `/ai` route untouched/unlinked. ✓
- Onboarding → footer; Feed/Governance secondary → Tasks 1,3. ✓
- Avatar initials-or-glyph (no hash slice) → Task 4 (frontend; backend email exposure tracked separately per spec §8). ✓
- Out of scope (Domains nested routes, realtime transport, Console runtime) → not in any task. ✓
```
