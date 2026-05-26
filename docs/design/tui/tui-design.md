# edgeplane tui — Master Design Document

**Branch:** `feat/tui-v3` (1 commit ahead of `main`)
**Version:** v0.5.0 (first stable)
**Date:** 2026-05-09
**Status:** Shipped binary compiles and runs; navigation fundamentally broken; several screens partially implemented.

---

## Architecture Overview

```
edgeplane tui
 └─ App (app.rs)
     ├─ Screen enum: Agents | Missions | Feed | Approvals | Secrets | Config
     ├─ WorkPool (work.rs) — std::thread + tokio::Handle::current().block_on()
     │   └─ results drained every 50ms in App::tick()
     ├─ DataClient trait (data.rs)
     │   ├─ RemoteDataClient — real HTTP calls to edgeplane-tower
     │   └─ FixtureDataClient — test fixture
     └─ Screens (screens/)
         ├─ agents.rs + edgeplane-tui-widgets/src/agents.rs
         ├─ mission_matrix.rs
         ├─ agent_feed.rs
         ├─ approval_queue.rs
         ├─ secrets.rs + edgeplane-tui-widgets/src/secrets_tree.rs
         └─ config.rs
```

Key invariant: `App::handle_key()` routes keys. Screen `handle_key()` returns `bool` (consumed). Global tab nav fires **only when `false` is returned**. This is the source of all nav conflicts.

---

## Navigation Model (Current — Broken)

### Tab Bar

```
[Agents]  [Missions]  [Feed]  [Approvals]  [Secrets]  [Config]
   a          m          f         p            s          c
```

### Conflict Matrix

| Global Key | Screen That Consumes It | Effect | Blocks Navigation To |
|---|---|---|---|
| `p` | Feed (`agent_feed.rs:92`) | toggles pause | Cannot switch Feed → Approvals |
| `s` | Approvals (`approval_queue.rs:79`) | skips approval | Cannot switch Approvals → Secrets |
| `c` | Feed (`agent_feed.rs:97`) | clears event list | Cannot switch Feed → Config |
| `a` | Secrets (`app.rs` Secrets arm, `matches!` macro) | tree select-all | Cannot switch Secrets → Agents |

The fundamental issue: single-char global nav vs single-char screen-local actions share the same keyspace with no modifier distinction.

### Proposed Fix (not yet implemented)

**Option A — Modifier-based nav:** Require `Alt+<letter>` or `Ctrl+<letter>` for tab switching. Screen-local keys remain unmodified. Cleanest UX but requires updating the tab bar display and all hints.

**Option B — Function keys:** `F1`–`F6` for tabs. Very safe, no conflicts possible. Less discoverable.

**Option C — Reassign conflicting screen-local keys:** 
- Feed: `<space>` = pause (instead of `p`), `x` = clear (instead of `c`)
- Approvals: `k` or `x` = skip (instead of `s`)
- Secrets: `*` or `Enter` = select-all (instead of `a`)

Option C is least disruptive — zero UI paradigm change, just remap 4 keys. Recommended starting point.

**Decision needed from user before implementation.**

---

## Screen Inventory

### 1. Agents (`a`)

**Status: Functional, minor gaps**

What works:
- Lists agents from `/agents` endpoint
- `AgentSummary` deserialization handles both `i32` and `String` ids (wire-safe)
- Status dots (●/○/◌) with color coding
- Detail panel on right (name, id, status, capabilities, last_seen/updated_at)

Gaps / TODOs:
- Operations hint row shows: `give-task` `restart` `clear-ctx` `remove` — **none wired** (no key handlers, no WorkRequest variants for these operations)
- `last_seen` field falls back to `updated_at` but the fallback chain is implicit; no explicit "N/A" display when both are None
- No auto-refresh; data only fetches on tab switch
- Agent detail panel is read-only; no inline editing capability

### 2. Missions (`m`)

**Status: Functional, error display missing**

What works:
- Three-pane layout: Domains | Missions | Tasks (33/33/34%)
- `/` search filter in Domains and Missions panes
- Tab key cycles focus across panes
- Enter on a Domain → dispatches `ListMissions`
- Enter on a Mission → dispatches `ListTasks` (via `missions_enter()`)
- Tasks pane uses canonical auth URL: `/missions/{m}/k/{k}/t`
- Status dots + coloring per status string

Gaps / TODOs:
- `MissionMatrixState.error` is set in `app.rs:114` but **never rendered** — errors silently disappear
- No empty-state message when a domain has zero missions (shows blank pane)
- Task detail panel is absent — selecting a task in the Tasks pane shows nothing additional
- No keyboard shortcut to trigger a task action (start/complete/fail)
- `tree_nodes()` method still exists for legacy compat but is unused in v3 rendering
- `TreeNode` enum and `selected_mission_idx()` are dead code post-v3

### 3. Feed (`f`)

**Status: Functional, display issues**

What works:
- SSE connection to `/sse` endpoint
- Live event streaming with FeedConnected/FeedDisconnected states
- Event list with timestamp, agent_id, mission_id, event_type, data summary
- `p` = pause (stops appending), `c` = clear list
- Status indicator: `● live` / `○ paused` / `✗ disconnected`

Gaps / TODOs:
- Hardcoded "Errors Governance Artifacts Heartbeat" filter label bar is **not functional** — no filter logic wired
- No reconnect backoff — on disconnect, user must manually navigate away and back to reconnect
- Event display truncates data; no expand-on-Enter to see full event JSON
- Max event buffer (likely unbounded Vec) — could grow without limit on active fleets
- Feed SSE endpoint (`/sse`) — needs to confirm this matches edgeplane-tower's actual SSE path; the `stream_feed` function uses it but the controlplane may expose `/events/stream`

### 4. Approvals (`p`)

**Status: Functional, error/history display missing**

What works:
- Lists pending approvals from `/approvals?status=pending`
- `y` = approve, `n` = reject, `s` = skip (advance cursor without acting)
- `take_pending_response()` → dispatches `RespondApproval`
- `ApprovalResponded` tick handler clears item and re-fetches list

Gaps / TODOs:
- `ApprovalQueueState.last_error` is set in `app.rs:158,187` but **never rendered**
- `ApprovalQueueState.history` Vec is allocated but **never appended to** — history panel empty forever
- No auto-polling; list only refreshes after an approve/reject action (or tab re-entry)
- No approval detail panel; only shows action + created_at + requester
- `request_context` JSON field from server never displayed
- **Key conflict:** `s` = skip here, but `s` = global nav to Secrets → user cannot leave Approvals to Secrets with one keystroke

### 5. Secrets (`s`)

**Status: Skeleton functional, no error display**

What works:
- `switch_to_secrets()` loads `~/.ep/infisical_profiles.json` and initializes tree
- Graceful "no profile" error state when profile file absent or malformed
- Tree widget dispatches `LoadSecretFolders` / `LoadSecretNames` work requests
- Keyboard navigation: arrows expand/collapse folders, space = expand

Gaps / TODOs:
- `secrets_tree.rs:70` error field (`error: Option<String>`) is **never displayed to user**
- Profile load parses JSON directly — if `edgeplane secrets infisical add` writes a different shape than `InfisicalProfileMap` expects, it silently falls through to the no-profile error
- No secret values shown (read-only names-only, intentional for v1 — mark as deferred)
- No search/filter within the tree
- **Key conflict:** `a` in the Secrets arm's `matches!` macro returns `true` for select-all → user cannot navigate Secrets → Agents

### 6. Config (`c`)

**Status: Server + Auth panels functional; remaining panels stub**

What works:
- Tab navigates to Config screen
- Ping latency displayed (ms roundtrip to edgeplane-tower)
- `● connected` / `✗ disconnected` indicator
- Server URL, token presence, agent ID shown
- **Auth panel (nav index 1) — fully implemented:**
  - Branding header: "Edgeplane Secure / Team Console"
  - OIDC primary sign-in: Enter triggers browser PKCE flow
  - Testing token section: Down to expand, type token masked, Enter to submit
  - In-flight states: Initiating → AwaitingBrowser (URL display + timer) → TimedOut / Failed
  - Signed-in state shows identity, `edgeplane auth logout` hint
  - Esc steps back (token input → OIDC focus → nav panel)
- Controlplane panel (nav index 0): URL editing, latency test, apply
- Profile panel: context switching
- Infisical panel: add/edit/delete profiles

Gaps / TODOs:
- Nav indices 2–8 (Agent, Display, Sync, Approvals, Feed, Secrets, About) show stub content
- No periodic re-ping; latency only refreshes on panel entry
- No ability to edit non-URL config values in-TUI

---

## Data Layer

### Endpoints Used

| Method | Path | Used By |
|---|---|---|
| GET | `/health` | ping / Config status |
| GET | `/agents` | Agents tab |
| GET | `/missions` | Missions tab |
| GET | `/domains/{d}/missions` | Domains → Missions |
| GET | `/domains/{d}/missions/{m}/tasks` | Missions → Tasks (canonical auth path) |
| GET | `/approvals?status=pending` | Approvals tab |
| POST | `/approvals/{id}/respond` | approve/reject |
| GET | `/sse` | Feed SSE stream |

### Known Endpoint Risks

- `/sse` path needs validation against edgeplane-tower — controlplane may use `/events/stream`
- `/domains/{d}/missions` — confirm edgeplane-tower routing

### AgentSummary Wire Shape

edgeplane-tower returns `id` as `i32`; custom `id_to_string` deserializer handles both `i32` and `String`. Fields not present on the wire default via `#[serde(default)]`.

---

## Tests

### Passing (17 total)

**`tests/test_remote_data_client.rs`** (8 tests — URL shape regression):
- `ping_calls_health`
- `list_missions_calls_missions`
- `list_missions_uses_domain_prefix`
- `list_tasks_uses_canonical_path`
- `list_approvals_includes_status_pending`
- `respond_approval_posts_to_correct_path`
- `list_agents_accepts_integer_id`
- `list_agents_accepts_string_id`

**`tests/test_app_tabs.rs`** (7 tests — tab dispatch):
- `agents_tab_loading_on_switch`
- `missions_tab_sets_loading`
- `approvals_tab_sets_loading`
- `approvals_tab_result_clears_loading`
- `secrets_tab_sets_no_profile_error_when_unconfigured`
- `feed_tab_navigable`
- `config_tab_navigable`

**`src/work.rs` inline** (2 tests):
- `pool_delivers_ping_result`
- `pool_delivers_missions_result`

### Missing Tests

- Nav conflict regression: verify `p` from Feed, `s` from Approvals, etc. (currently no test catches these)
- Error rendering: no test verifies `last_error` / matrix `error` display
- Missions Enter routing by focus (Domains pane Enter vs Missions pane Enter)
- Config nav_selection actually switching panel content

---

## Open Issues (Prioritized)

### P0 — Blocks basic usability

1. **Nav conflicts** — `p`, `s`, `c`, `a` consumed by screen-local handlers; cannot navigate freely between tabs. Resolution strategy decision needed. (See Proposed Fix section above.)

### P1 — Silent failures, user has no feedback

2. **MissionMatrixState.error never rendered** — mission load errors disappear silently
3. **ApprovalQueueState.last_error never rendered** — approve/reject failures disappear silently
4. **Secrets tree error never displayed** — Infisical API errors disappear silently
5. **Config nav_selection has no effect** — 9 items, only 1 panel works

### P2 — Missing functionality

6. **No auto-refresh for Approvals** — stale data unless user acts; polling interval needed
7. **Feed reconnect is manual** — no backoff/retry on disconnect
8. **Feed filter bar is non-functional** — Errors/Governance/Artifacts/Heartbeat labels do nothing
9. **Agent operations not wired** — give-task, restart, clear-ctx, remove show in hints but do nothing
10. **Config panels 2–8 are stubs** — Auth panel (index 1) now fully implemented; Agent, Display, Sync, etc. remain placeholders

### P3 — Polish / deferred

11. **Task detail panel absent** — selecting a task shows nothing additional
12. **Approval request_context not displayed** — JSON payload visible in API but not shown in TUI
13. **Approval history Vec never populated** — history panel always empty
14. **Feed event expand-on-Enter** — cannot see full event JSON
15. **Feed SSE endpoint path** — `/sse` vs `/events/stream` — needs verification against edgeplane-tower
16. **Max event buffer** — Feed Vec can grow unbounded on active fleets
17. **Secrets tree search/filter** — not present
18. **Agent list auto-refresh** — no polling; only loads on tab switch

---

## Deferred (Out of Scope Until Explicitly Scoped)

- Secret *values* display (browse-only shows names; viewing values deferred)
- Any write operations from TUI: creating domains/missions/tasks, editing agents
- Secrets editing via TUI
- RTK integration
- Tailscale integration

---

## Build & Release State

- **Binary:** `crates/edgeplane/target/release/edgeplane` — compiled and confirmed `edgeplane tui --help` works
- **Feature gate:** `default = ["tui"]` in `crates/edgeplane/Cargo.toml`; explicit `--features tui` also works
- **Release workflow:** `.github/workflows/release-edgeplane.yml` updated with explicit `--features tui`
- **Version:** `0.5.0` across `edgeplane`, `edgeplane-tui`, `edgeplane-tui-widgets`

### PR State

PR `feat/tui-v3 → main` created. Branch is 1 commit ahead of main. Title: `feat(tui): first stable release — edgeplane tui v0.5.0 with 6 wired tabs`.

PR should **not merge** until P0 nav conflict is resolved. The binary compiles and the tabs exist but the UX is broken for cross-tab navigation.

---

## Recommended Next Steps (for user annotation)

1. **Decide nav conflict resolution strategy** (Option A/B/C above, or another approach)
2. **Fix P0** — implement chosen nav strategy
3. **Fix P1** — surface errors in UI (low effort, high value: each is ~5-10 LOC render change)
4. **Verify SSE endpoint path** (`/sse` vs `/events/stream`)
5. **Wire auto-refresh for Approvals** (WorkPool timer or periodic Ping-driven trigger)
6. **Fix Config nav_selection** — implement at minimum 3-4 real panel contents (Auth, Agent, About)
7. **Merge + tag v0.5.0** after P0+P1 resolved

Items 3-7 could be one PR on top of the P0 fix, or broken into separate focused PRs.
