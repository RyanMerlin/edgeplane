# EdgePlane Web — Linear-anchored Design Build-out Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development (Sonnet implementers, mockup-first locked). Steps use `- [ ]`. **The visual source of truth is the committed mockup:** `docs/superpowers/mockups/2026-06-07-edgeplane-linear-mockup.html` — open it; match it.

**Goal:** Conform the live React app (the Pass-1 sidebar shell + screens) to a Linear-anchored, rust-warm design system that is information-clear and artful — replacing the v1 GitHub-dark/monospace look.

**Architecture:** A single token system in `app.css` drives everything (existing var *names* keep, values change → most components reskin for free). Then component-by-component conformance to the mockup. No IA change (sidebar IA already approved). Built on `feat/web-tab-consolidation`.

**Tech stack:** React 19, TanStack Router/Query, Vitest+RTL, Biome. Inter Variable (web font). Spec: `docs/superpowers/specs/2026-06-07-linear-design-system-pivot.md`.

---

## Design doctrine (the rules every task obeys)

1. **Lean on Linear's patterns, differentiate on identity.** Adopt Linear's layout, density, and interaction patterns freely (sidebar rail, breadcrumbs, list density, detail views, keyboard-first, restrained accent). Make it unmistakably EdgePlane via: the **rust-warm palette**, the **⬡ hex motif**, our **own iconography**, and our **domain content** (agents/domains/missions/tasks, not "issues"). Patterns are shared craft; identity is ours. Not a clone.
2. **In-theme, not a merlin-site port.** Rust-orange family (`#e75b2a`) + warm-coherent neutrals. Tune freely for this app's needs.
3. **Color earns its place (this is how "key info & priorities" pop).** Neutrals carry ~90% of the surface. The **rust accent is reserved** for: the active/primary action, active nav, focus, and key emphasis — so rust always means "this matters." **Status hues are a small, fixed, legible vocabulary used ONLY for status, never decoration.** This makes scanning effortless.
4. **Artful & premium.** Border-based depth (no shadows in-app), warm-coherent neutrals, Inter weights 510/590, tight-but-breathable spacing, subtle hover/active micro-interactions. No generic-AI slop.

## Color system (semantic roles — the authoritative palette)

```css
:root[data-theme="dark"] {           /* warm-coherent; depth via bg steps, not shadows */
  --base:#0c0a09; --frame:#141210; --surface:#16130f; --card:#110e0c;
  --raised:rgba(255,240,230,0.055); --raised-2:rgba(255,240,230,0.10); --input:rgba(255,240,230,0.025);
  --border:rgba(255,238,224,0.09); --border-subtle:rgba(255,238,224,0.06); --border-strong:#2a2420;
  --text:#F2EDE6; --text-2:#D6CDC2; --muted:#9A8F84; --dim:#6E6459;
  --accent:#e75b2a; --accent-dim:rgba(231,91,42,0.16); --accent-deep:#b8451e;  /* RESERVED for primary/active/key */
  --ok:#57d08a; --warn:#E6B355; --err:#F2685C; --info:#4a9eda; --purple:#8B5CF6; /* status ONLY */
  --font:"InterVariable","Inter",-apple-system,sans-serif;
  --mono:"JetBrains Mono","SF Mono",ui-monospace,monospace;  /* machine data only */
  --r-xs:2px; --r-md:6px; --r-lg:8px; --r-xl:12px; --r-pill:9999px; --sidebar:232px;
}
```

**Usage rules (enforced in review):**
- **Rust accent** = primary button, active nav item indicator, focused input ring, links, the ⬡ mark, key-figure emphasis. Never a background wash, never decorative.
- **Status vocabulary** (consistent everywhere — table, dashboard, detail, feed): `--ok` online/active/done · `--warn` working/busy/proposed · `--err` error/offline/failed/blocked · `--info` in-progress/running · `--purple` governance. A 7px dot + a square `--r-xs` tag is the canonical status token.
- **Hierarchy via neutrals + weight, not color:** `--text` (primary) > `--text-2` (labels/rows) > `--muted` (secondary) > `--dim` (meta/timestamps). Weight 590 for headings/names, 510 for nav/labels, 300–400 for body/conversation.

---

## File map

- **Foundation:** `web/src/styles/app.css` (`:root` dark+light, base `body` font, button/tag/input primitives), `web/index.html` (preconnect + Inter Variable `<link>`), `web/package.json` (optional `@fontsource-variable/inter` dep instead of CDN).
- **Shell:** `components/shell/navModel.ts` (drop WHO/WHAT headings), `Sidebar.tsx` (+ test), `AppShell.tsx`, `Breadcrumbs.tsx`.
- **Screens:** `routes/index.tsx` (Dashboard), `routes/agents.index.tsx` + `components/fleet/AgentsTable.tsx`, `routes/agents.$agentId.tsx` + `components/conversation/*`, `routes/domains.tsx`, plus `routes/feed.tsx`/`governance.tsx`/`onboarding.tsx` (token conformance).

**Per-task gate (from `web/`):** `npm run build` + `npm test` + `npm run lint` green before commit. Tests assert testids/behavior, not pixels — most reskins keep tests green; update only where structure/text changes (e.g., Onboarding leaving the rail).

---

## Phase A — Foundation (tokens + type + primitives)

### Task A1 — Token system + Inter Variable
- [ ] **Load Inter Variable:** add to `web/index.html` `<head>`: `<link rel="preconnect" href="https://rsms.me/">` + `<link rel="stylesheet" href="https://rsms.me/inter/inter.css">`. (Or `npm i @fontsource-variable/inter` and `import` in `main.tsx` — prefer this for offline/CI determinism; pick one, note which.)
- [ ] **Replace `app.css` `:root` dark tokens** with the color system above (KEEP existing var names so consumers reskin automatically; ADD new ones: `--frame --card --raised-2 --text-2 --border-subtle --accent-dim --accent-deep`). Update the **light** `:root[data-theme="light"]` to a warm-coherent light variant (cream `--base:#faf8f5`, warm borders, same rust accent, `--accent-deep:#b8451e` for bold).
- [ ] **Base typography:** `body { font-family: var(--font); font-size: 13px; }` (Inter); set `font-feature-settings:"cv11","ss01"`; keep `--mono` only on data classes. Remove the "monospace-only" comment block.
- [ ] Build + test + lint. Manually eyeball the running app (dev server) — every screen should already look warmer/Linear-ish from the token swap alone. Commit: `feat(web): Linear+rust token system + Inter Variable`.

### Task A2 — Primitive components (buttons, tags, inputs)
- [ ] In `app.css`, restyle base `button` + add `.btn`/`.btn.primary`/`.btn.ghost` to Linear pills (`--r-pill`, primary = off-white `#E5E5E6` on `--base` text OR rust `--accent` — match mockup: primary action uses rust; light "invert" pill is secondary), `.tag` to square (`--r-xs`, `--raised` bg, 11px/510, leading status dot), inputs to warm (`--input` bg, `--border`, focus ring `--accent`). Match the mockup's button/tag/input CSS.
- [ ] Build + test + lint. Commit: `feat(web): Linear pill buttons, square status tags, warm inputs`.

## Phase B — Shell conformance

### Task B1 — navModel: drop WHO/WHAT headings
- [ ] In `navModel.ts`, set all group `heading` to `null` (no section labels — Linear-style). Keep the items/order. Update `navModel.test.ts` if it asserts heading text (it asserts `to` order — likely unaffected; verify).
- [ ] Build + test. Commit: `feat(web): drop WHO/WHAT sidebar group labels`.

### Task B2 — Sidebar to mockup (+ Onboarding into Account→Settings)
- [ ] Reskin `Sidebar.tsx` to the mockup: 232px, ⬡ hex logo + "EdgePlane" wordmark, the search row (`⌘K` affordance — static for now), nav rows 28px/510 with hover `--raised` + active `--raised-2`/`--text`, **no group labels**.
- [ ] **Move Onboarding OUT of the rail** into the bottom Account control's popover, **nested under a "Settings" item** (Account → Settings → Onboarding), alongside theme toggle + Logout. The account control is a left-justified nav-row (avatar + name + chevron), matching the mockup. Avatar uses `avatarLabel` (initials or ⬡ glyph).
- [ ] Update `Sidebar.test.tsx`: assert Onboarding is **not** a top-level rail item; assert it appears in the account menu under Settings (open menu → Settings → Onboarding); keep nav active-state + logout assertions.
- [ ] Build + test + lint. Commit: `feat(web): Sidebar conform to Linear mockup; Onboarding under Account→Settings`.

### Task B3 — AppShell + Breadcrumbs polish
- [ ] AppShell: 44px header (`border-bottom: --border-subtle`), content padding to match mockup, full-height retained. Breadcrumbs: `›` separators, current = `--text`, links = `--muted`→hover `--text-2`. Match mockup.
- [ ] Build + test + lint. Commit: `feat(web): breadcrumb header + content frame polish`.

## Phase C — Screen conformance (match the mockup)

### Task C1 — Dashboard (`index.tsx`)
- [ ] Cards (`--card`, `--r-lg`, `--border-subtle`), `h3` uppercase 11px/590 `--dim`, big stat 28px/590 with `--ok` online figure, rust `→` links, activity rows with status tags + mono timestamps. Match mockup. Keep data wiring.
- [ ] Build + test + lint. Commit.

### Task C2 — Agents table (`AgentsTable.tsx`)
- [ ] Linear density (row ~36px, 9px/12px padding), `--border-subtle` row separators, status = dot+square tag, `public_id` mono `--accent`, node mono `--dim`, source tag. Hover `--raised`. Match mockup. Keep sort + row-click→detail.
- [ ] Build + test + lint. Commit.

### Task C3 — Agent detail + conversation (`agents.$agentId.tsx`, `components/conversation/*`)
- [ ] Identity strip (status dot, name 15px/590, mono pid `--accent`, status tag, node, last-seen). Conversation pane = Claude-style: message list (role avatar 26px `--r-md`; agent avatar `--accent-dim`/`--accent`; user neutral; body 14px/300), composer (`--input`, `--border`, rust send button). Full-height. Match mockup. Keep `useAcpConversation`/`ConversationView` wiring + not-attachable/404 states.
- [ ] Build + test + lint. Commit.

### Task C4 — Domains tree (`domains.tsx`)
- [ ] Linear list density: nested `tnode` rows (domain/mission/task indent), expand chevrons `--dim`, names `--text`/`--text-2`, status dots on tasks, mono counts `--dim`, hover `--raised`. Match mockup. Keep tree data + expand logic.
- [ ] Build + test + lint. Commit.

## Phase D — Secondary screens (token conformance)

### Task D1 — Feed, Governance, Onboarding
- [ ] Apply the token system + primitives to `feed.tsx` (Live|Raw toggle as Linear segmented control), `governance.tsx` (policy cards + status), `onboarding.tsx` (now reached via Account→Settings; clean card layout). No new behavior — visual conformance.
- [ ] Build + test + lint. Commit.

## Phase E — Linear pattern enhancements ("leverage as much as possible") — STAGED, optional after D

These deepen the Linear adoption beyond static conformance. Each is independent; do as capacity/appetite allows, mockup-first for any that are visual.

- [ ] **E1 — ⌘K command palette:** global command/search overlay (navigate, jump to agent/domain, actions). Linear's signature interaction. (New component + keybinding.)
- [ ] **E2 — Keyboard navigation:** j/k row movement, Enter to open, Esc to back — operator-grade.
- [ ] **E3 — Peek / quick-detail:** open an agent or task in a side peek panel without losing the list (Linear's peek).
- [ ] **E4 — Detail right-properties panel:** on agent/mission/task detail, a right rail of properties (status, owner, node, capabilities) — Linear's issue layout, our entities.
- [ ] **E5 — Collapsible sidebar sections + counts**, refined empty states, subtle load/stagger micro-interactions.

> Scope note: **Phases A–D are the shippable build-out** (the mockup made real). Phase E is the "leverage Linear further" track — enumerate, then pull items in deliberately (each visual one previewed in the mockup first). Do NOT silently expand A–D into E.

## Phase F — Integration + review
- [ ] Grep for leftover v1 styling (old hardcoded GitHub hexes, `monospace` body, `nav-tab`). Full build+test+lint. Visual pass on the dev server across all screens. Final adversarial review (Opus reviewer) of the whole design diff. Commit.

---

## Self-review (coverage)
- Rust-warm token system + Inter → A1; primitives → A2.
- WHO/WHAT removed → B1. Onboarding into Account→Settings (the 3×-requested fix) → B2. Alignment (left-justified account row) → B2.
- Every screen conformed to the mockup → C1–C4, D1.
- "Leverage Linear more" → Phase E (staged). "Not a clone / identity" → doctrine §1 + rust theme + ⬡ motif.
- "Key info & priorities pop / artful" → color-system usage rules (rust reserved; fixed status vocabulary; hierarchy via neutrals+weight), enforced in per-task review.
- Out of scope (unchanged): realtime transport, Console runtime, Domains nested routes, `/auth/me` email for real initials.
