# EdgePlane Web v2 — SvelteKit → React Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:subagent-driven-development or superpowers:executing-plans to execute task-by-task. Steps use checkbox (`- [ ]`) syntax. This is a **full rewrite, keep v1 for reference, delete after validation** (owner decision 2026-05-31) — NOT an incremental strangler. Build all of `web2/`, test/debug against the live API, flip the Dockerfile, then delete `web/`.

**Goal:** Rebuild the operator web UI as a production-quality, AI-purpose-built React SPA, served the same way the current SvelteKit app is (static behind the Rust tower's `EP_WEB_DIR`).

**Architecture:** Vite + React 19 + TanStack Router (type-safe SPA), shadcn/Radix + Tailwind v4, TanStack Query v5 (server state) + Zustand (UI/realtime), `utoipa` OpenAPI on the axum backend → `openapi-typescript` + `openapi-fetch` typed client. Cookie+CSRF same-origin auth unchanged. No Node server.

**Why a rewrite, not a patch:** the current SvelteKit app is partly broken (Svelte compile/prerender failure on `main`) and the owner wants a from-scratch, done-right React stack. The migration is mostly a **port**: the Svelte app already uses the direct analogs — `@tanstack/svelte-query`→TanStack Query, `bits-ui`(Radix-for-Svelte)→shadcn/Radix, Tailwind v4→Tailwind v4 (unchanged), `xterm.js`→`xterm.js` (unchanged). The data layer (`web/src/lib/api/*`) is plain framework-agnostic `fetch`.

**Build location:** new top-level `web2/`. `web/` stays untouched as the reference until validation passes, then `git rm -r web/ && git mv web2 web`. All work in the isolated worktree `/tmp/ep-webv2` on branch `feat/web-v2-react`.

---

## Locked decisions (owner-approved 2026-05-31)

- Vite + React 19 + TanStack Router, static SPA, served by tower `EP_WEB_DIR` (unchanged `server.rs`).
- shadcn/ui + Radix + Tailwind v4; `xterm.js` for the terminal.
- TanStack Query v5 (server state) + Zustand (UI + the SSE singleton).
- Type safety: `utoipa` → `openapi-typescript` + `openapi-fetch`. **Front-loaded** (a parallel rust subagent annotates the consumed surface) so frontend phases are pure-frontend against a stable client. `ts-rs`/hand-typing is the documented fallback.
- Auth: port the existing backend OIDC + session-cookie + CSRF flow as-is. No `react-oidc-context` (the flow is backend-driven; the SPA just calls `/api/auth/me` and handles `?oidc_grant=`). The Svelte `authStore.token` is vestigial (always null) — drop it.
- AI console: port the existing **turn-based** `/ai` console now; isolate the conversation behind a swappable `transport` so Vercel AI SDK `useChat` streaming drops in later (needs a future backend token-stream endpoint — not in v2.0).
- Migration: **full rewrite, keep v1 for reference, delete after validation.**
- Tooling: Biome (lint+format), Vitest + React Testing Library, Playwright (E2E, esp. realtime).

## Resolved unknowns (from exploration — these were open risks, now answered)

- **Cutover seam is the tower Dockerfile.** `crates/edgeplane-tower/Dockerfile` lines 1-6 = a `node:22-slim AS web-builder` stage (`npm ci`, `npm run build` on `web/`); line 60 = `COPY --from=web-builder /build/web/build /usr/local/share/edgeplane-web`. **This is the only prod packaging seam** (no helm `EP_WEB_DIR` override; no CI builds web today). Cutover = repoint this stage at `web2/` and change the copied dir from `web/build` (SvelteKit) to `web2/dist` (Vite). No Rust change.
- **SSE `/api/events/stream` is NOT a tower route** — it's proxied to upstream `edgeplaned` via `server.rs` `proxy_fallback`. utoipa on the tower cannot describe it. Same for the WS attach proxy and any other proxied endpoints. → those payloads are **hand-typed** in `manual-types.ts`; utoipa covers tower-owned REST only. (Full coverage would mean adding utoipa to `edgeplaned` — a separate effort, out of scope.)
- **`gen-openapi` must be DB-free.** `main.rs` builds the app with a live `PgPool`; the OpenAPI doc builder must be factored state-free or it won't run in CI. Spike in Phase 0.
- **`/ai` is turn-based** (`/ai/sessions`, `/ai/sessions/{id}/turns`, `.../actions/{id}/approve|reject`, plus a `/ai/sessions/{id}/stream` SSE for events) — confirmed in `routes/ai.rs`. No LLM token streaming today.
- **Auth is cookie+CSRF only** — httpOnly session cookie + `ep_csrf_token` cookie → `X-CSRF-Token` header. No bearer token in practice.

## Default answers to the open design questions (proceed on these unless owner overrides)

- **A. utoipa scope:** tower-owned routes only; hand-type SSE/WS/proxied. (Adding utoipa to `edgeplaned` is a separate future effort.)
- **B. `/api/docs` + `/api/openapi.json`:** behind auth (matches current posture).
- **C. Dev OIDC:** during auth testing, run the SPA through the tower on :8008 (or register a Vite-port redirect URI in Authentik). Resolve concretely in Phase 0.5.
- **D. Cutover packaging:** the tower Dockerfile web-builder stage (resolved above).
- **E. Worktree:** yes — all v2 work in `/tmp/ep-webv2`.

## Reference stack (pin latest stable at scaffold; verify versions)

React 19 · Vite · TanStack Router v1 (`@tanstack/router-plugin` file routes) · TanStack Query v5 · Tailwind v4 (`@tailwindcss/vite`) · shadcn/ui + Radix · Zustand v5 · `openapi-typescript` + `openapi-fetch` · `@xterm/xterm` + `addon-fit` + `addon-web-links` · `streamdown` (streaming-safe markdown; future-proofs AI console) · Biome · Vitest + RTL + jsdom · Playwright.
Backend: `utoipa` (axum_extras, chrono, uuid) + `utoipa-axum` + `utoipa-scalar`.

> Pin `@tanstack/react-query@^5`. The Svelte app uses `@tanstack/svelte-query@^6` — a **separate version track**; do not "match" to v6.

---

## File structure (web2/)

```
web2/
  package.json  vite.config.ts  tsconfig.json  biome.json  index.html  components.json
  src/
    main.tsx
    styles/app.css                 # port web/src/app.css (Tailwind v4 @theme + CSS custom props)
    routes/                        # TanStack Router file routes
      __root.tsx                   # shell: TopBar/StatusBar/Outlet, QueryClientProvider, auth gate, OIDC grant handler
      index.tsx                    # Fleet dashboard (/)
      overview.tsx  agents.tsx  agents.$agentId.tsx  explorer.tsx
      feed.tsx  matrix.tsx  governance.tsx  onboarding.tsx  ai.tsx
    components/
      ui/*                         # shadcn components
      TopBar.tsx  StatusBar.tsx  LoginScreen.tsx  AgentTerminal.tsx  StatusDot.tsx
    features/ai/{useAiSession.ts,Transcript.tsx,Composer.tsx,ApprovalsPane.tsx}
    lib/
      api/http.ts                  # port web/src/lib/api/client.ts (CSRF + credentials + ApiError)
      api/openapi.ts               # openapi-fetch createClient + CSRF middleware
      api/schema.gen.ts            # GENERATED, committed
      api/manual-types.ts          # hand-typed SSE/WS/proxied payloads
      api/{agents,fleet,explorer,governance,oidc,ai}.ts   # port web/src/lib/api/*
      queryKeys.ts                 # port verbatim
      telemetry.ts                 # SSE singleton (port web/src/lib/telemetry.ts)
    stores/{auth.ts,telemetry.ts,ui.ts}   # Zustand
  e2e/                             # Playwright
```

Backend (new/modified):
- `crates/edgeplane-tower/Cargo.toml` — add utoipa deps + `[[bin]] gen-openapi`.
- `crates/edgeplane-tower/src/openapi.rs` (new) — state-free OpenAPI aggregator.
- `crates/edgeplane-tower/src/bin/gen_openapi.rs` (new) — emit `openapi.json`, no DB.
- `crates/edgeplane-tower/src/routes/*.rs` — `#[utoipa::path]` on consumed handlers; `#[derive(ToSchema)]` on their DTOs.
- `crates/edgeplane-tower/src/routes/mod.rs` — migrate `build_router()` → `utoipa-axum` `OpenApiRouter` incrementally.
- `crates/edgeplane-tower/src/server.rs` — mount Scalar `/api/docs` + `/api/openapi.json` (behind auth); **no change** to the `EP_WEB_DIR`/`ServeDir`/SPA-fallback block.
- `.github/workflows/web.yml` (new) — build+test+codegen-drift gate.

---

## Phase 0 — Scaffolding + foundations (ends shippable: login + Governance screen render against live :8008)

- [ ] **0.1 Scaffold web2/** — `npm create vite@latest web2 -- --template react-ts`; add deps from the reference stack; add `@tanstack/router-plugin`.
- [ ] **0.2 Tailwind v4 + shadcn** — port `web/src/app.css` theme tokens verbatim into `web2/src/styles/app.css` (the `@theme` block + every CSS custom prop components reference: `--accent --surface --surface-2 --border --border-2 --ok --warn --err --muted --text --base --dim`); `shadcn init`; add `button input textarea card table tabs dialog popover command dropdown-menu badge scroll-area tooltip toast`.
- [ ] **0.3 Biome** init (2-space; match current); `lint`/`format` scripts.
- [ ] **0.4 Vite dev proxy** — **single `/api` rule**, `changeOrigin:true`, `ws:true`, target `process.env.EP_DEV_API ?? 'http://localhost:8008'`. (Fixes the current per-prefix gap where only `/ws` had `ws:true` — the attach socket `/api/runtime/.../attach` and SSE `/api/events/stream` need it.)
- [ ] **0.5 Auth port** — `web2/src/stores/auth.ts` (Zustand): `bootstrap()` (GET `/api/auth/me`), `startOidcLogin(redirect)` (→ `/api/auth/oidc/start`), `logout()` (DELETE `/api/auth/sessions/current`); grant exchange in `__root.tsx` mount effect (read `oidc_grant` from `location`, POST `/api/auth/oidc/exchange`, scrub via `history.replaceState`). Port from `web/src/lib/auth.ts`. **Resolve dev-OIDC redirect** (run SPA through :8008 or register the Vite port).
- [ ] **0.6 Typed-client foundation** — `lib/api/http.ts` (port `client.ts`: `/api` prefix, CSRF cookie→header, `credentials:'include'`, `ApiError`); `lib/api/openapi.ts` (`createClient<paths>({ baseUrl:'/api', credentials:'include' })` + CSRF middleware on mutations); port `lib/queryKeys.ts` verbatim; QueryClientProvider in `__root.tsx`.
- [ ] **0.7 gen-openapi spike (DB-free)** — add utoipa deps; `openapi.rs` aggregator built from `utoipa-axum` metadata WITHOUT `AppState`; `bin/gen_openapi.rs` writes `openapi.json` to stdout; `cargo run -p edgeplane-tower --bin gen-openapi > web2/openapi.json` must succeed with no DB. Annotate the first endpoints: `GET /health`, `GET /auth/me`, `DELETE /auth/sessions/current`, and Governance (`GET /governance/policy/{active,versions,events}`, `POST /governance/policy/reload`). `npm run codegen` (`openapi-typescript ../openapi.json -o src/lib/api/schema.gen.ts`); commit both files.
- [ ] **0.8 App shell** — `__root.tsx` porting `+layout.svelte`: TopBar (logo, nav tabs Overview/Console/Agents/Explorer/Feed/Governance, theme toggle, avatar menu), StatusBar, LoginScreen (OIDC button); auth gate via TanStack Router `beforeLoad`.
- [ ] **0.9 First screen — Governance** (`routes/governance.tsx`): `useQuery` active policy + versions + audit events (`refetchInterval: 30_000`), reload via `useMutation`+invalidate. Proves the full vertical slice: utoipa→codegen→openapi-fetch→Query→shadcn→live data + a CSRF mutation.
- [ ] **0.10 Exit criteria** — `cd web2 && npm run dev`: OIDC login round-trips against live :8008; Governance renders live data; reload mutation works; Biome clean; one Vitest RTL smoke + one Playwright login test; `gen-openapi` runs DB-free.

**Parallel backend workstream (rust subagent):** annotate the full consumed surface (agents, runtime/fleet, explorer, ai, onboarding, overview metrics, auth/oidc) so Phases 1–5 are pure frontend. Hand-type SSE/WS in `manual-types.ts`.

## Phases 1–5 — Screen port (order: read-only → trees → SSE → terminal → AI)

- [ ] **Phase 1 — Governance** (formalize 0.9) + parity checklist + Playwright happy path.
- [ ] **Phase 2 — read-only:** `/agents` (fleet fan-out: `GET /runtime/nodes`, `/runtime/nodes/:id/agents` + `/agents/:id` detail; shadcn Table; StatusDot), `/agents/[id]` (type-safe param), `/explorer` (recursive tree + node-detail-on-click, lazy node fetch), `/onboarding` (manifest + copy/regenerate). Terminal panes in agents-detail/explorer deferred to Phase 4 (placeholders).
- [ ] **Phase 3 — SSE realtime cluster:** first build `lib/telemetry.ts` + `stores/telemetry.ts` porting `web/src/lib/telemetry.ts` exactly — module-scope singleton `EventSource` → `/api/events/stream`, 60-event ring buffer, backoff 1s→30s (longer first backoff at zero messages), rate-limit aware (`pausedUntil`); exposed via **Zustand** (NOT React Query — SSE is push, not request/response); started once from the `__root` login effect, stopped on logout. `MatrixEvent` is **hand-typed** (proxied endpoint, not in OpenAPI). Then consumers: `/matrix` (raw, per-type filter, JSON debug, color-coding), `/feed` (filters/search/alerts-only/rate counter), and Overview's live pane. Hazard: mounting Feed then Matrix must NOT open a second socket.
- [ ] **Phase 4 — xterm WS terminal (hardest):** `components/AgentTerminal.tsx` porting `AgentTerminal.svelte` — xterm in a `useEffect` with a **ref guard for React 19 StrictMode double-mount** (disposed flag + single construction + cleanup dispose), lazy `import()` of xterm+addons, `@xterm/addon-attach`-style WS to `attachAgentWsUrl()` (`binaryType='arraybuffer'`, keystrokes via `TextEncoder`, `{kind:'resize',cols,rows}` JSON), `@xterm/addon-fit` + `ResizeObserver`, reconnect backoff; remount-on-agent-change via `key={agentId}`. WS contract hand-typed. Then `/` Fleet dashboard (6 profile tabs, Ctrl+1..6 global keydown+cleanup, status dots, terminal pane) + enable Explorer's terminal pane.
- [ ] **Phase 5 — AI console** (`routes/ai.tsx`): turn-based. `features/ai/useAiSession.ts` is **the transport boundary** — today TanStack Query with `refetchInterval` polling of `/ai/sessions/:id`; tomorrow swap internals for `useChat` streaming without view changes. Stable shape `{ transcript, pendingActions, send, approve, reject, busy, error }`. Components: `Transcript` (pin-to-bottom + jump-to-latest), `Composer` (Enter-to-send), `ApprovalsPane`, event-debug toggle, backend-unavailable state. Markdown via `streamdown`. The `payload`/`policy`/`content` blobs are hand-typed islands. Port mutation `queryClient.setQueryData` (server returns the whole session).

Each screen: parity checklist (same fields/columns, refetch cadence, error/empty states, actions wired) + a Playwright happy path. Per the full-rewrite decision, screens don't each need to be the *served* app — validation happens against the live API with `EP_WEB_DIR=web2/dist`.

## Phase 6 — Cutover (after all 9 screens validated)

- [ ] **6.1** `.github/workflows/web.yml`: `npm ci && npm run build` on `web2/`; plus the codegen-drift gate (below). Closes the "no CI builds web" gap.
- [ ] **6.2** Flip the tower Dockerfile web-builder stage: `COPY web2/package.json web2/package-lock.json` / `COPY web2 ./` / `npm run build`, and change line 60 to `COPY --from=web-builder /build/web2/dist /usr/local/share/edgeplane-web` (Vite emits `dist/`, not SvelteKit's `build/`). `server.rs` ServeDir + SPA-fallback unchanged (already serves `index.html` for client routes).
- [ ] **6.3** Build the image, validate all 9 screens + login + SSE + terminal + AI against the live tower.
- [ ] **6.4** Retire Svelte: `git rm -r web/ && git mv web2 web`; update Dockerfile/CI/justfile paths `web2`→`web` and `openapi.json`/`schema.gen.ts` script paths in the same commit.
- [ ] **6.5 Rollback:** until the `git rm`, reverting the Dockerfile web-builder stage to `web/`+`web/build` restores the Svelte UI with no Rust rebuild. Keep `web/` through soak.

## utoipa CI sync gate (codegen drift)

`.github/workflows/web.yml` job:
1. `cargo run -p edgeplane-tower --bin gen-openapi > /tmp/openapi.json` → `git diff --no-index /tmp/openapi.json web2/openapi.json` (fail on drift).
2. `cd web2 && npm ci && npm run codegen && git diff --exit-code src/lib/api/schema.gen.ts` (fail if TS types stale).
3. `cd web2 && npm run lint && npm test && npm run build`.

## Verification

- **Per-phase:** screen served behind `EP_WEB_DIR=web2/dist` against the live tower; parity checklist green; Vitest RTL (render + mocked client + one mutation); Biome clean; codegen + openapi drift checks green.
- **End-to-end Playwright (by cutover):** (1) OIDC login → cookie → `/api/auth/me`, logout clears + stops SSE; (2) feed receives/filters events, reconnects, ring cap 60; (3) terminal WS connects, keystrokes echo, resize sent, **no double-socket under StrictMode**, Ctrl+1..6 switch; (4) AI console create session, send turn, approve+reject, backend-unavailable; (5) type-safe nav across all 9 routes.
- **Type safety as a gate:** `npm run codegen` + `tsc --noEmit` + the CI drift guard prove the client matches and stays in sync with the Rust API.

## Risks & hazards

1. **utoipa ≠ whole API** — SSE/WS/proxied endpoints are upstream `edgeplaned`, never in tower OpenAPI. Hand-type them; don't claim full coverage.
2. **xterm + React 19 StrictMode double-mount** — ref guard + cleanup dispose; verify exactly one prod socket. (Phase 4)
3. **SSE singleton vs React lifecycle** — module-scope singleton + Zustand; never React Query; one shared socket across Feed/Matrix/Overview.
4. **OIDC cookie through the Vite dev proxy** — `changeOrigin:true` + `SameSite`/`Secure` on `http://localhost`; the start→IdP→`?oidc_grant=` round-trip must land back on the dev origin. Test in Phase 0, not at cutover.
5. **`gen-openapi` DB-free** — factor the doc builder state-free or CI can't run it. Phase 0 spike.
6. **Vite outDir `dist/` ≠ SvelteKit `build/`** — the Dockerfile COPY path must change at cutover (easy to miss).
7. **Shared fleet working tree** — isolated worktree `/tmp/ep-webv2`; `web2/` avoids path collisions; coordinate the final `git mv` when no other session is mid-op.

## Critical files (port-from references)

- `web/src/{app.html,app.css}`, `web/vite.config.ts`, `web/svelte.config.js`
- `web/src/lib/{auth.ts,telemetry.ts,queryKeys.ts}`, `web/src/lib/api/*`
- `web/src/lib/components/AgentTerminal.svelte`, `web/src/routes/**`
- `crates/edgeplane-tower/src/{server.rs,routes/mod.rs,routes/*.rs}`, `crates/edgeplane-tower/Dockerfile`
