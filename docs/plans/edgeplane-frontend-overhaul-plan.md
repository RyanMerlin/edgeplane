# Edgeplane Frontend Overhaul — Implementation Plan

**Date:** 2026-05-04  
**Status:** Draft implementation plan  
**Scope:** Refactor the current SvelteKit frontend from a monolithic tab-based page into a route-based, typed, testable, componentized application.

---

## 0. Important Version Guidance

The original goose dump included package versions that may be stale or inconsistent with upstream package state. Do **not** blindly hard-pin those versions.

Before implementation, verify current versions directly from npm:

```bash
cd web
npm info @tanstack/svelte-query version
npm info bits-ui version
npm info tailwindcss version
npm info vitest version
```

Recommended install shape:

```bash
cd web
npm i @tanstack/svelte-query bits-ui clsx tailwind-merge
npm i -D tailwindcss autoprefixer vitest @testing-library/svelte
```

Use the latest stable versions unless there is a concrete compatibility issue with the current SvelteKit/Svelte/Vite stack.

---

## 1. Current State

| Area | Current Reality |
|---|---|
| Framework | SvelteKit 2.52 / Svelte 5 |
| Architecture | Single monolithic `+page.svelte`, around 700 lines |
| Navigation | Tab-based UI inside one route |
| Component library | None; custom CSS only |
| API layer | Single `lib/api.ts`, around 354 lines, raw `fetch` calls |
| Server state | Manual polling, including 2.5s AI session refresh |
| Client state | Svelte stores for auth and telemetry |
| Realtime | SSE via custom `EventSource` wrapper in `lib/telemetry.ts` |
| Design system | Custom CSS variables, glass-panel aesthetic, light/dark themes |
| Testing | No frontend test baseline |
| Build | Static adapter, no SSR |

**Core problem:** AI Console, Matrix, Explorer, Onboarding, and Governance all live in one component with too many reactive variables, async functions, and template branches.

---

## 2. Inspiration Projects — Takeaways

### Paperclip — Structural Model

| Pattern | What to Adopt |
|---|---|
| Typed API client | Generic `request<T>` with `ApiError` to eliminate repeated `throw Error(res.statusText)` patterns |
| Domain API modules | Split `lib/api.ts` by feature domain |
| Centralized query keys | Use a cache key hierarchy to prevent stale data bugs |
| TanStack Query | Use `@tanstack/svelte-query` for caching, background refetch, invalidation, and optimistic updates |
| Scoped global state | Keep Svelte stores, but structure them by typed domain boundaries |
| shadcn-style primitives | Use `bits-ui` as the Svelte-native headless primitive layer |
| Custom hooks pattern | Translate to Svelte actions and composable stores |
| Atomic components | Components such as `ApprovalCard`, `StatusBadge`, and `ActivityRow`, each with one job |
| `cn()` utility | Use `clsx` + `tailwind-merge` for class composition |
| Testing | Add Vitest unit tests and Playwright E2E tests |

### Mission-Control / OpenClaw — Panel-Based Model

| Pattern | What to Adopt |
|---|---|
| Granular state store | Typed interfaces per domain, even if Svelte stores replace Zustand |
| Panel components | Large but feature-scoped panels; each owns its own data boundary and UI |
| Variant styling | Use component variants, e.g. `StatusBadge variant="success"` |
| Route-based navigation | Replace major tabs with URLs |
| Accessible primitives | Pair headless primitives with Tailwind-compatible styling |

---

## 3. Target Architecture

Move toward this structure:

```txt
web/src/
  app.css
  app.html
  routes/
    +layout.svelte
    +layout.ts
    +page.svelte
    ai/+page.svelte
    explorer/+page.svelte
    governance/+page.svelte
    onboarding/+page.svelte
    matrix/+page.svelte
  lib/
    api/
      client.ts
      ai.ts
      explorer.ts
      governance.ts
      evolve.ts
      jobs.ts
      index.ts
    components/
      ui/
        button.svelte
        badge.svelte
        card.svelte
        dialog.svelte
        dropdown-menu.svelte
        input.svelte
        textarea.svelte
        select.svelte
        skeleton.svelte
        toast.svelte
      ai/
        transcript.svelte
        composer.svelte
        approval-panel.svelte
        event-pill.svelte
      explorer/
        mission-tree.svelte
        node-detail.svelte
      governance/
        policy-card.svelte
        event-list.svelte
      matrix/
        event-stream.svelte
        status-chip.svelte
      onboarding/
        onboarding-form.svelte
        manifest-preview.svelte
    stores/
      auth.ts
      matrix.ts
    queryKeys.ts
    utils.ts
```

---

## 4. Implementation Phases

## Phase 0 — Foundation

**Goal:** Add infrastructure without changing UI behavior.

### Tasks

1. Install dependencies.

```bash
cd web
npm i @tanstack/svelte-query bits-ui clsx tailwind-merge
npm i -D tailwindcss autoprefixer vitest @testing-library/svelte
```

2. Add `src/lib/utils.ts`.

```ts
import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
```

3. Add `src/lib/api/client.ts`.

```ts
export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
    public body: unknown,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

async function parseBody(res: Response): Promise<unknown> {
  const text = await res.text();
  if (!text) return null;

  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

export async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);

  if (!headers.has('Content-Type') && !(init.body instanceof FormData)) {
    headers.set('Content-Type', 'application/json');
  }

  const res = await fetch(path, {
    ...init,
    headers,
    credentials: 'include',
  });

  if (!res.ok) {
    const body = await parseBody(res);
    const message =
      typeof body === 'object' && body && 'error' in body
        ? String((body as { error: unknown }).error)
        : `Request failed: ${res.status}`;

    throw new ApiError(message, res.status, body);
  }

  if (res.status === 204) return undefined as T;

  return parseBody(res) as Promise<T>;
}

export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'POST', body: body === undefined ? undefined : JSON.stringify(body) }),
  put: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'PUT', body: body === undefined ? undefined : JSON.stringify(body) }),
  patch: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'PATCH', body: body === undefined ? undefined : JSON.stringify(body) }),
  delete: <T>(path: string) => request<T>(path, { method: 'DELETE' }),
};
```

4. Add `src/lib/queryKeys.ts`.

```ts
export const queryKeys = {
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
    node: (type: string, id: string) =>
      [...queryKeys.explorer.all, 'node', type, id] as const,
  },
  governance: {
    all: ['governance'] as const,
    policy: () => [...queryKeys.governance.all, 'policy'] as const,
    events: () => [...queryKeys.governance.all, 'events'] as const,
  },
  evolve: {
    all: ['evolve'] as const,
    mission: (id: string) => [...queryKeys.evolve.all, 'mission', id] as const,
  },
  jobs: {
    all: ['jobs'] as const,
    list: () => [...queryKeys.jobs.all, 'list'] as const,
    detail: (id: number) => [...queryKeys.jobs.all, 'detail', id] as const,
  },
};
```

5. Configure Tailwind only if it is compatible with the current SvelteKit/Vite setup. Keep existing `app.css` as the theme source of truth.

6. Add `vitest.config.ts` and a smoke test for `cn()` or `ApiError`.

### Acceptance Criteria

```bash
cd web
npm run check
npm run build
npm test
```

All pass. No UI changes.

---

## Phase 1 — API Layer Split

**Goal:** Replace `lib/api.ts` with typed domain modules while preserving compatibility.

### Target Files

```txt
src/lib/api/
  client.ts
  ai.ts
  explorer.ts
  governance.ts
  evolve.ts
  jobs.ts
  index.ts
```

### Tasks

1. Move shared fetch/error logic into `client.ts`.
2. Create one API module per product domain.
3. Move current API functions into the correct modules without changing endpoint paths.
4. Add response/request types near the domain module unless they already exist elsewhere.
5. Keep backward-compatible exports until the route/component migration is complete.

Example:

```ts
// src/lib/api/index.ts
export * from './client';
export * from './ai';
export * from './explorer';
export * from './governance';
export * from './evolve';
export * from './jobs';
```

### Acceptance Criteria

- No raw `fetch()` calls outside `client.ts`, except SSE/EventSource code.
- Existing imports still resolve.
- Build passes.
- Runtime behavior unchanged.

---

## Phase 2 — Server State with TanStack Query

**Goal:** Replace manual polling and ad hoc refresh functions with query/mutation primitives.

### Tasks

1. Add `QueryClientProvider` in `src/routes/+layout.svelte`.

```svelte
<script lang="ts">
  import { QueryClient, QueryClientProvider } from '@tanstack/svelte-query';

  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: true,
        retry: 1,
      },
    },
  });
</script>

<QueryClientProvider client={queryClient}>
  <slot />
</QueryClientProvider>
```

2. Convert reads to queries.

| Current Pattern | Replacement |
|---|---|
| `initAi()` | query for session list |
| `refreshActiveSession()` | session query with `refetchInterval` |
| `refreshTree()` | explorer tree query |
| `refreshPolicy()` | governance policy query |
| `refreshPolicyEvents()` | governance events query |

3. Convert writes to mutations.

| Current Pattern | Replacement |
|---|---|
| `sendAiMessage()` | mutation + invalidate active session |
| `approve()` | mutation + optimistic session update |
| `reject()` | mutation + optimistic session update |
| job create/update/delete | mutations + invalidate jobs list |

4. Keep Svelte stores for client-only state.

```txt
stores/
  auth.ts      auth token/session state
  matrix.ts    SSE connection status and event buffer
```

### Acceptance Criteria

- Manual AI polling interval removed.
- Session data refreshes through TanStack Query.
- Mutations invalidate or update the correct query keys.
- SSE remains separate from query cache.

---

## Phase 3 — Route-Based Navigation

**Goal:** Replace tab-based navigation with SvelteKit routes.

### Route Target

```txt
src/routes/
  +layout.svelte
  +layout.ts
  +page.svelte
  ai/+page.svelte
  explorer/+page.svelte
  governance/+page.svelte
  onboarding/+page.svelte
  matrix/+page.svelte
```

### Tasks

1. Move global shell into `+layout.svelte`.

Responsibilities:

- Header
- Branding
- Theme toggle
- Auth/logout controls
- Primary navigation
- Shared layout frame

2. Make root route redirect to `/ai`.
3. Move each old tab body into its own route page.
4. Keep each route initially dumb: copy existing UI first, then refactor components in Phase 5.
5. Remove tab state after all routes are working.

### Acceptance Criteria

- `/ai`, `/explorer`, `/governance`, `/onboarding`, and `/matrix` load independently.
- Browser back/forward works.
- Auth gate applies consistently.
- No old tab selector state remains.

---

## Phase 4 — UI Primitives

**Goal:** Introduce accessible reusable components without a visual rewrite.

### Target Components

```txt
src/lib/components/ui/
  button.svelte
  badge.svelte
  card.svelte
  dialog.svelte
  dropdown-menu.svelte
  input.svelte
  textarea.svelte
  select.svelte
  checkbox.svelte
  skeleton.svelte
  toast.svelte
```

### Tasks

1. Wrap `bits-ui` primitives where accessibility behavior matters.
2. Use `cn()` for class merging.
3. Preserve existing CSS variables and visual identity.
4. Migrate repeated markup in this order:
   1. buttons
   2. badges/status pills
   3. cards/glass panels
   4. inputs/textareas
   5. dialogs/toasts/dropdowns

Example `button.svelte`:

```svelte
<script lang="ts">
  import { cn } from '$lib/utils';

  export let variant: 'primary' | 'ghost' | 'icon' = 'ghost';
  export let disabled = false;
</script>

<button
  class={cn(
    'inline-flex items-center justify-center rounded-full transition-colors',
    variant === 'primary' && 'bg-accent text-white hover:opacity-90',
    variant === 'ghost' && 'border border-panel-border hover:bg-white/5',
    variant === 'icon' && 'size-9 rounded-full',
    disabled && 'cursor-not-allowed opacity-50',
  )}
  {disabled}
  on:click
>
  <slot />
</button>
```

### Acceptance Criteria

- Common UI patterns use shared components.
- No accessibility regression.
- Visual styling remains recognizably Edgeplane.
- Build passes.

---

## Phase 5 — Feature Component Refactor

**Goal:** Make each route page a thin orchestrator.

### AI Console

```txt
src/lib/components/ai/
  transcript.svelte
  composer.svelte
  approval-panel.svelte
  event-pill.svelte
  session-list.svelte
```

`routes/ai/+page.svelte` should only:

- Load sessions
- Load active session
- Wire mutations
- Pass props to components

### Explorer

```txt
src/lib/components/explorer/
  mission-tree.svelte
  node-detail.svelte
  node-status-badge.svelte
```

`routes/explorer/+page.svelte` should only:

- Load tree
- Track selected node
- Pass selected node into detail panel

### Governance

```txt
src/lib/components/governance/
  policy-card.svelte
  event-list.svelte
  policy-editor.svelte
```

### Matrix

```txt
src/lib/components/matrix/
  event-stream.svelte
  status-chip.svelte
  rate-limit-card.svelte
```

### Onboarding

```txt
src/lib/components/onboarding/
  onboarding-form.svelte
  manifest-preview.svelte
  validation-summary.svelte
```

### Acceptance Criteria

- No route page exceeds roughly 150 lines.
- Components have one clear responsibility.
- API calls stay in route/page orchestration or domain query modules.
- Presentation components do not directly mutate global state.

---

## Phase 6 — Tests and Polish

**Goal:** Add safety rails before deeper visual iteration.

### Unit Tests

Cover:

- `ApiError`
- `request<T>` success/error behavior
- query key generation
- auth store transitions
- matrix store transitions
- UI primitive rendering

### E2E Tests

Cover:

- Login/auth path
- Route navigation
- AI session load
- AI message submission
- Explorer tree load and node select
- Governance policy/event display
- Matrix event stream basic render

### Accessibility Checks

- Keyboard navigation
- Focus management
- Dialog close behavior
- Button/input labels
- Status badge semantics
- Color contrast sanity pass

### Acceptance Criteria

- `npm test` passes.
- E2E smoke suite passes.
- No route-level regressions.
- No obvious keyboard-navigation traps.

---

## 5. Risk Register

| Risk | Likelihood | Mitigation |
|---|---:|---|
| Dependency version mismatch | Medium | Verify with `npm info` immediately before install. Avoid stale pinned versions. |
| Tailwind migration complexity | Medium | Add Tailwind incrementally. Preserve `app.css` as theme base. Do not rewrite all CSS at once. |
| TanStack Svelte Query API mismatch | Medium | Start with one read-only query. Confirm Svelte 5 compatibility before broad migration. |
| Route split breaks shared state | Medium | Move auth/shell first. Use query cache for server state, stores only for client state. |
| SSE behavior regression | Low | Keep SSE implementation unchanged until Matrix route is isolated. |
| Visual drift | Medium | Create UI primitives that preserve existing tokens before redesigning. |
| Scope creep | High | Each phase must land independently with build/check/test passing. |

---

## 6. PR Breakdown

### PR 1 — Foundation

- Add dependencies
- Add `utils.ts`
- Add `api/client.ts`
- Add `queryKeys.ts`
- Add Vitest smoke test

### PR 2 — API Split

- Create domain API modules
- Re-export compatibility layer
- Remove duplicate fetch/error handling

### PR 3 — Query Integration

- Add QueryClient provider
- Convert AI session reads/writes first
- Convert Explorer/Governance reads next

### PR 4 — Routes

- Add layout shell
- Add `/ai`, `/explorer`, `/governance`, `/onboarding`, `/matrix`
- Remove tab navigation

### PR 5 — UI Primitives

- Add shared components
- Replace repeated buttons, badges, cards, inputs

### PR 6 — Feature Components

- Extract AI, Explorer, Governance, Matrix, Onboarding components
- Keep route pages thin

### PR 7 — Tests and Cleanup

- Unit tests
- E2E smoke tests
- Remove dead code
- Tighten types

---

## 7. Immediate Next Commands

```bash
cd /home/merlin/code/edgeplane/web
npm info @tanstack/svelte-query version
npm info bits-ui version
npm info tailwindcss version
npm info vitest version
npm i @tanstack/svelte-query bits-ui clsx tailwind-merge
npm i -D tailwindcss autoprefixer vitest @testing-library/svelte
npm run check
npm run build
```

Then add these files first:

```txt
web/src/lib/utils.ts
web/src/lib/api/client.ts
web/src/lib/queryKeys.ts
web/vitest.config.ts
web/src/lib/utils.test.ts
```

Start with Phase 0 only. Do not touch the UI until the foundation builds cleanly.
