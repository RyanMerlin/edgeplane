# Web Design Checkpoint — v1

**Status:** Static mockups complete, pending review  
**Date:** 2026-05-17  
**Location:** `/home/merlin/code/missioncontrol/docs/design/web/`  
**Start here:** Open `index.html` — design system reference + page navigator

---

## What Was Built

8 static HTML mockup pages representing a complete frontend redesign. No SvelteKit yet — these are pixel-accurate design targets to review and iterate on before touching any framework code.

| File | Page | Notes |
|------|------|-------|
| `index.html` | Design System | Color palette, typography, components, page navigator |
| `01-login.html` | Auth | OIDC primary + token fallback, no topbar |
| `02-dashboard.html` | Overview | **New page** — doesn't exist yet; metrics strip + fleet summary |
| `03-ai-console.html` | Console | Session sidebar + flat transcript + approvals panel |
| `04-agents.html` | Agents | Node sidebar + dense table + bottom detail drawer |
| `05-explorer.html` | Explorer | 3-pane missions / klusters / tasks |
| `06-feed.html` | Feed | Renamed from "Matrix"; filter bar + 5-col grid + detail panel |
| `07-governance.html` | Governance | Left nav + policy + flags + action rules + audit log |

---

## Core Design Rules (Hard Constraints)

These were non-negotiable from the start. Any iteration must preserve them.

1. **Zero pill shapes.** `border-radius` max `3px` on buttons, `2px` on tags. Everything else sharp. No `border-radius: 999px` anywhere.
2. **Monospace-first.** `'JetBrains Mono', 'Cascadia Code', 'Fira Code', 'SF Mono', ui-monospace, monospace` — no switching to a proportional font for "normal" text.
3. **Multi-pane horizontal splits.** Every page uses `display: flex` with `border-right: 1px solid #21262d` separators. No floating cards, no grid of rounded panels.
4. **Viewport-locked.** `100vh`, `overflow: hidden` on `html/body`. Layout = `34px topbar + flex:1 content + 24px statusbar`. Per-pane scroll only.
5. **Flat rows, not cards.** Event rows / transcript turns / task rows are `border-bottom: 1px solid` table rows. State is communicated with left-border color (`3px solid <semantic-color>`) not background fills or rounded containers.
6. **No glass-morphism.** Flat surface colors only: `#161b22` surface, `1px solid #21262d` or `#30363d` borders. No `backdrop-filter: blur()`, no radial gradients on body.
7. **Status dots, not badge chips.** `●/○/⟳/✓/✗/▲` inline. No `.status-chip { border-radius: 999px }`.

---

## Color System

| Token | Value | Usage |
|-------|-------|-------|
| `base` | `#0d1117` | Page background |
| `surface` | `#161b22` | Topbar, pane headers, panels, cards |
| `surface-2` | `#1c2938` | Selected row background |
| `border` | `#21262d` | Pane separators, row dividers |
| `border-2` | `#30363d` | Stronger borders, indent guides |
| `muted` | `#8b949e` | Secondary labels, metadata |
| `dim` | `#484f58` | Tertiary, timestamps, dimmed rows |
| `text` | `#e6edf3` | Primary content |
| `accent` | `#58a6ff` | Links, active indicators, running state |
| `ok` | `#3fb950` | Success, done, healthy |
| `warn` | `#d29922` | Warning state |
| `err` | `#f85149` | Error, failed state |
| `purple` | `#bc8cff` | Governance, AI, approval-required |

---

## Proposed Navigation Changes

Current nav (from `+layout.svelte`): **AI Console → Agents → Missions → Matrix**

Proposed nav: **Overview → Console → Agents → Explorer → Feed → Governance**

Changes:
- "Overview" added as the first tab (new dashboard page)
- "AI Console" → "Console" (shorter)
- "Missions" → "Explorer" (more accurate — it's mission + kluster + task browsing)
- "Matrix" → "Feed" (clearer)
- "Governance" added as last tab

---

## What Each Page Doesn't Have Yet (Iteration Targets)

### 01-login
- OAuth provider selection (multiple OIDC providers)
- "Remember this device" checkbox
- Error state (wrong token, network failure)

### 02-dashboard (Overview)
- Sparkline / mini trend graphs in metric cells
- Clickable metric cells that deep-link to filtered views
- "Recent activity" timeline vs flat event list
- Mission health breakdown (% complete across all missions)

### 03-console (AI Console)
- Streaming cursor animation while assistant is generating
- Session search / filter
- Multi-session view (tabs or split)
- Attachment / artifact upload area in composer
- Keyboard shortcut panel

### 04-agents
- Agent detail full page (vs bottom drawer) — deeper history, full config, session replay
- Bulk actions (select multiple, restart selected, reassign)
- Agent enrollment flow
- Capability tag filtering

### 05-explorer
- Task detail panel (right side, like Feed's detail panel)
- Dependency graph visualization (for a kluster's tasks)
- "New task" inline form
- Drag-to-reorder task priority

### 06-feed
- Time range picker / jump-to-time
- Saved filter presets
- Alert acknowledgment flow
- Column visibility toggle

### 07-governance
- Rule editor / new rule form
- Policy version diff view
- Approval queue detail (expand an audit item to full decision flow)
- Agent-level permission overrides

---

## Coherent Data Universe

All pages share the same fictional fleet to make the design feel like a real system:

- **5 nodes:** excalibur, bastion, atlas, relay, vault-1 (vault-1 offline)
- **19 agents across all nodes**
- **4 active missions:** web-revamp, research-q3, infra-hardening, platform-ops
- **Task IDs:** TSK-0139 through TSK-0147 (web-revamp / design-phase cluster)
- **Agent names:** aria-mc-engineer, codex-impl-1/2, goose-deploy-1, analysis-agent-1/2/3, etc.

Any new pages or iteration must use these same entities for consistency.

---

## Current Frontend (the Before)

Key problems the design solves, for reference during iteration:

| Problem | Location | Fix Applied |
|---------|----------|-------------|
| `border-radius: 999px` on tabs, buttons, status chips | `app.css` | Max 3px everywhere |
| Glass-morphism panels with `backdrop-filter: blur(10px)` | `app.css` | Flat `#161b22` surfaces |
| Radial gradient background on `body` | `app.css` | Solid `#0d1117` |
| `Space Grotesk` as primary font | `app.css` | Monospace stack |
| Events displayed as `.event-pill` rounded cards | `ai/+page.svelte` | Flat rows with left-border |
| No multi-pane layout anywhere | all routes | Horizontal flex splits |
| Root redirects to `/ai`, no overview page | `+layout.svelte` | New `/` dashboard |
| Navigation label "Matrix" (unclear) | `+layout.svelte` | Renamed to "Feed" |

---

## Next Steps

1. **Review all 8 pages in browser** — open `index.html` and click through
2. **Flag specific pages for iteration** — note what works, what doesn't, what's missing
3. **Iterate on HTML mockups** before touching any SvelteKit code
4. **Once designs are locked** — implement in SvelteKit: new `app.css`, updated `+layout.svelte`, route-by-route replacement
