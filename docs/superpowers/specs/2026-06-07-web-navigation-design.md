# EdgePlane Web — Navigation / IA Redesign (Pass 1)

**Status:** DESIGN — awaiting owner review
**Date:** 2026-06-07
**Branch (design + likely impl base):** `feat/web-tab-consolidation`
**Supersedes:** the in-page Fleet `Conversations | Agents` toggle and the per-agent "Conversations console" introduced in the 2026-06-07 tab-consolidation commits (see "Relationship to prior work").

---

## 1. Problem

The web app reached a stable baseline (deps refreshed, 8 tabs → 5) but a live review exposed that the navigation model itself is wrong, not just cosmetically off:

- Detail views (`/agents/$agentId`) show **no active nav item** — you lose your place.
- The only "back" is a **tiny top-left link**; there is no consistent up/back affordance.
- The detail page renders at **half window height** with blank space below — the shell doesn't own layout.
- The app is a flat set of top-nav routes with **ad-hoc detail pages**, with no structure to hold drill-down.

The owner's directive: a real navigation **design pass, no patchwork**. The organizing principle (chosen during brainstorming) is **two co-equal spines**: **WHO** (the agents you operate/converse with) and **WHAT** (the Domain → Mission → Task work hierarchy), with conversation and telemetry woven in.

This is the EdgePlane orchestration console; per `EDGEPLANE_PHILOSOPHY.md` it "is not a chatbot UI" — it is a coordination surface over a fleet and its work.

## 2. Goals / Non-goals

**Goals (Pass 1)**
- A persistent **left-sidebar shell** that always shows both spines and stays active during drill-down.
- **Breadcrumbs** as the canonical up/back affordance.
- **Full-height content** everywhere (kill the half-page bug structurally).
- **Agent detail** as a proper full-height view with its live conversation as the body.
- Remove dead/redundant surfaces from the primary nav (Console).

**Non-goals (Pass 1)**
- Rebuilding Domains into nested routes (Domain/Mission/Task pages) — **Pass 2**.
- Fixing realtime *transport* (SSE feed, ACP conversation connectivity) — separate workstream; see §9.
- Fixing the AI-session runtime — out of scope (Console is being dropped from nav).
- Renaming decisions beyond what the new structure resolves (the ambiguous "Fleet" label is retired by the Dashboard/Agents split; no further renames this pass).

## 3. Locked decisions (from brainstorming)

| Decision | Outcome |
|---|---|
| Primary job / spine | **Both, tightly linked** — WHO (Agents) ↔ WHAT (Domains/Missions/Tasks) |
| Shell | **Persistent left sidebar** + content area |
| Console (`/ai`) | **Drop from nav** (routes + `aisession` schema preserved, dormant) — see §4.1 |
| Pass 1 scope | **Shell + nav + agent detail**; Domains keeps today's Explorer tree under the "Domains" slot |

## 4. Information architecture

```
┌─────────────┐
│  EdgePlane  │  header / logo
├─────────────┤
│ ◇ Dashboard │  landing — fleet summary + work summary + recent activity (the WHO↔WHAT pivot)
│ WHO ········│  group label
│  Agents     │  merged cp+mesh agent list → agent detail (info + live ACP conversation)
│ WHAT ·······│  group label
│  Domains    │  Domain › Mission › Task browser (today's Explorer, re-homed; nested routes in Pass 2)
│ ─────────── │
│  Feed       │  telemetry (Live | Raw), secondary
│  Governance │  policy, secondary
├─────────────┤  footer
│ ⚙ Onboarding│  setup/settings (was a top tab)
│  RM ▾       │  account menu (avatar)
└─────────────┘
```

### 4.1 Surface map (today → new)

| Today | New home | Notes |
|---|---|---|
| `/` Fleet (table+console toggle) | **Dashboard** (`/`) | `/` becomes the two-spine summary landing. Supersedes the "default to Agents table" tweak. |
| `/agents` (+ `/agents/$id`) | **Agents** (WHO) | `/agents` is its own sidebar section again (not a redirect). Conversation lives in `/agents/$id` detail. The `Conversations\|Agents` toggle and the per-agent "Conversations console" are **removed**. |
| `/explorer` | **Domains** (WHAT) | Re-home + relabel; route renamed `/explorer` → `/domains` with a redirect. Internals unchanged in Pass 1. |
| `/feed` (+ absorbed Matrix) | **Feed** | Unchanged; relocated to secondary group. |
| `/governance` | **Governance** | Unchanged; relocated. |
| `/onboarding` | **footer** | Out of primary nav; still routable. |
| `/ai` Console | **dropped from nav** | Route + `aisession` schema preserved (dormant). Rationale below. |
| `/matrix` | already redirects to `/feed` | No nav entry. |

### 4.2 Why Console is dropped (investigation summary)

Per `docs/architecture/entities.md` §AISession, an AISession is an **owner-scoped** logical AI conversation — no `agent_id`/`domain_id`/`mission_id` — architecturally **parallel** to the fleet-agent ACP conversation. Investigation of `crates/edgeplane-tower/src/routes/ai.rs` found `create_turn` records the user message and emits an event but **never invokes any runtime** — no assistant response is generated; `EP_AI_PROVIDER`/`EP_AI_MODEL` have zero callsites. Live state: 2 sessions, `runtime_kind:"opencode"`, `runtime_session_id:null`, `turns:[]` — a non-functional shell.

The fleet agents (the **WHO** spine) already provide the real, wired conversational surface (ACP over WebSocket, `agents.$agentId.tsx`, Phase 4) and ARE the owner's personal-agent layer. Console duplicates the interaction surface, adds nothing today, and a nav slot that silently accepts input and never responds erodes trust — and contradicts the "not a chatbot UI" identity. **Drop from nav; preserve backend.** The one future that would justify reviving it — a *governance/approval inbox* for actions fleet agents queue on the owner's behalf — is distinct, unbuilt, and would belong in the Governance area, not primary nav. Revisit then.

## 5. Shell mechanics — and how each reported bug dies

| Mechanism | Fixes |
|---|---|
| **Persistent shell**: `Sidebar` (fixed width, full height, own scroll) + `Content` (flex child, `height:100%`, own scroll). | The half-page/blank-below bug — content fills height everywhere, not per-page. |
| **Active state by route prefix**: a sidebar item is active when the current path is under it (`/agents/*` keeps "Agents" lit). | "No active tab on the detail view." |
| **Breadcrumbs** in the content header: clickable trail (`Agents › aria-operator`; later `Domains › Apollo › Mission X › Task Y`). | The "tiny back button"; gives a consistent up/back affordance. |
| **Agent detail** = full-height: identity + status + breadcrumb header; **ACP conversation pane as the body**. | The cramped/half detail view; makes conversation the point of the detail page. |

## 6. Route map (Pass 1)

| Route | Component | Change |
|---|---|---|
| `/` | `Dashboard` | **new** — two-spine summary (replaces merged Fleet) |
| `/agents` | `AgentsPage` (renders `AgentsTable`) | own section again (the consolidation's `/agents`→`/` redirect is removed) |
| `/agents/$agentId` | `AgentDetailPage` | full-height layout + breadcrumb (no tiny back link) |
| `/domains` | existing Explorer component | `/explorer` renamed → `/domains`; `/explorer` redirects |
| `/feed` | `FeedPage` | unchanged |
| `/governance` | `GovernancePage` | unchanged |
| `/onboarding` | `OnboardingPage` | unchanged route; nav moves to footer |
| `/ai`, `/matrix` | unchanged | not in nav (`/matrix` still redirects to `/feed`) |

## 7. Component structure

**New (`components/shell/`)**
- `AppShell.tsx` — sidebar + content layout; owns the full-height flex frame. Replaces the `TopBar`-based shell body in `__root.tsx`.
- `Sidebar.tsx` — the rail: Dashboard, WHO/Agents, WHAT/Domains, Feed, Governance, footer (Onboarding + avatar menu). Active-state by route prefix. Hosts the avatar/account menu (Onboarding lives here; logout/theme move here from the old TopBar).
- `Breadcrumbs.tsx` — content-header trail; per-route crumb definitions.

**Changed**
- `__root.tsx` — render `<AppShell>` (auth gate + OIDC grant handling unchanged); drop `TopBar`/`StatusBar` nav.
- `routes/index.tsx` — becomes `Dashboard`; the merged Fleet table/console is retired (table → `/agents`, per-agent console dropped).
- `routes/agents.tsx` — revert from redirect-layout back to the Agents list (`AgentsTable` + `useMergedAgents`); keep `/agents/$agentId` child.
- `routes/agents.$agentId.tsx` — full-height layout, breadcrumb header (remove the standalone "← Fleet" link).
- `routes/explorer.tsx` → `routes/domains.tsx` (rename) + `/explorer` redirect.

**Kept from the consolidation work**
- `lib/useMergedAgents.ts` (shared cp+mesh merge) — still the agent data source for Agents + Dashboard.
- `components/fleet/AgentsTable.tsx`, `components/events/RawEventList.tsx`, the Feed Live|Raw toggle, the Onboarding-into-menu move.

**Dashboard (`/`) content, Pass 1 (kept simple):** a **Fleet** summary card (online/total from `useMergedAgents`, link to Agents), a **Work** summary card (domain/mission counts from the existing Explorer tree query, link to Domains), and a **Recent activity** strip from the event stream. No new backend endpoints.

## 8. Dependency: avatar initials ("RM", not an ID)

`GET /api/auth/me` returns `{subject: <opaque hash>, auth_type, session_id}` — **no email/display name** — so the SPA cannot derive "RM". Fix requires the tower to expose `email` and/or `display_name` on `/auth/me` (the value exists in the OIDC session per `prod.json`, pending confirmation it's stored on the usersession), then the sidebar avatar computes initials from that. **In Pass 1, contingent on the email being available server-side**; if a tower change is needed it's a small, isolated addition. If unavailable, fall back to a neutral glyph (not a hash slice) and track the backend exposure separately.

## 9. Explicitly out of scope (separate workstreams)

- **Feed SSE not delivering / ACP conversation "waiting for agent"** — observed in the dev preview; the 6 `aria-*` agents are attachable (online, `node_id=excalibur`) and federated attach is live in prod, so this is likely a vite-dev-proxy SSE/WS artifact, but must be confirmed against prod. Not a navigation concern; do not patch here.
- **Console runtime** (AI-session) — dropped from nav; reviving is a future product decision.
- **Domains nested routes** — Pass 2.

## 10. Testing

- Shell unit tests: sidebar renders both spines; active-state highlights by route prefix (incl. detail routes); footer hosts Onboarding + avatar.
- Breadcrumbs: correct trail per route; crumbs navigate.
- Agent detail: full-height layout asserted (container fills); breadcrumb present; conversation pane mounts.
- Preserve/adapt existing tests (`useMergedAgents`, `AgentsTable`, `RawEventList`, Feed toggle); update `index`/`agents` tests for the Dashboard/Agents split.
- Keep the gate green: `npm run build` + `npm test` + `npm run lint`.

## 11. Risks / open questions

- **Reverting the `/agents`→`/` consolidation** is churn on a branch that just consolidated; acceptable because the sidebar IA is the agreed end-state and the shared hook/components survive.
- **Avatar email availability** (§8) — confirm before committing to a Pass-1 fix vs. fallback.
- **Sidebar on narrow widths** — Pass 1 targets desktop operator use; a collapse/hamburger is a follow-up, not now.

## 12. Relationship to prior work

This builds on the 2026-06-07 tab-consolidation (deps refresh; 8→5 tabs; shared `useMergedAgents`; Feed absorbs Matrix; Onboarding→menu). It **keeps** the shared hook, `AgentsTable`, `RawEventList`, the Feed toggle, and the Onboarding relocation. It **supersedes** the Fleet `Conversations|Agents` toggle and the per-agent Conversations console (conversation now lives in agent detail), and re-establishes `/agents` as a first-class section rather than a redirect to `/`.
