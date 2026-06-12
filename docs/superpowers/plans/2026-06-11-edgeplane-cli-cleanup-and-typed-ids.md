# EdgePlane CLI Cleanup + Typed-ID Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement each workstream task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clean up the `edgeplane` CLI surface (descriptions, bare-invocation, colorized/ordered command tree, moderate consolidation), correct the stale auth/token UX, add a Web UI admin page to mint join tokens, and migrate entity IDs to a typed-prefix scheme (`d_`/`m_`/`t_`) including the legacy `k_` rows in prod.

**Architecture:** Seven independent, separately-shippable PRs. The mechanical CLI/auth fixes ship first and standalone. The two structural/irreversible changes — command-tree consolidation (WS-3) and the typed-ID migration (WS-6) — are each **gated behind a design artifact that Merlin signs off before any code lands**, because both break existing usage or rewrite live prod data. Renames ship with hidden back-compat aliases; the ID migration uses a dual-read window (emit-new / accept-old) then a background data migration.

**Tech Stack:** Rust (clap CLI in `crates/edgeplane/`, Axum tower in `crates/edgeplane-tower/`), Python backend + Alembic (entity persistence), React 19 + TanStack Router + Vite (`web/`), `cargo nextest` for Rust tests.

**Decisions locked (2026-06-11):**
- **Command tree:** Moderate consolidation (~37 → ~25 top-level) + all UX fixes.
- **Entity IDs:** Typed prefixes (`d_`/`m_`/`t_`) going forward + dual-read migration of legacy `k_`.
- **Token flag:** Rename `--bootstrap-token` → `--join-token`, keep `--bootstrap-token` as a hidden alias.
- **Packaging:** Independent phased PRs (this doc's workstreams).
- **`signal` CLI regression:** its own PR (WS-7, carried from the 2026-06-11 diagnosis).

**Cross-cutting verification:** every Rust workstream ends with `cargo build -p edgeplane` (or the relevant crate) + `cargo nextest run -p <crate>` + `cargo clippy -p <crate> -- -D warnings`. Per the engineer profile, flag for review before merge to main; never auto-merge.

**Reminder for all workers:** ripgrep is recursive by default — use `rg -n "pat"`. Never `rg -r` (that is `--replace` and silently rewrites matches). See `[[feedback_ripgrep_replace_flag]]`.

---

## Workstream sequencing & dependencies

```
Independent, low-risk, ship first (any order, parallelizable):
  WS-1  CLI text fixes
  WS-4  Token/auth UX cleanup
  WS-7  signal CLI register_agent regression
  WS-5  Admin → Create Join Token (Web UI)   [backend already exists]

Gated on design sign-off:
  WS-3  Command-tree consolidation   ── design gate ──▶ implement (+ WS-2 tree render reflects new groups)
  WS-2  Colorized/ordered command tree   (do with/after WS-3 so grouping matches final structure)
  WS-6  Typed-ID migration   ── design gate ──▶ phased rollout   [highest risk, do last]
```

WS-2 depends on WS-3 (the colorized tree should render the *consolidated* grouping). WS-6 is sequenced last because it touches live prod data across 24 tables.

---

## WS-1: CLI text fixes (about string, `init`, stale "MC" sweep)

**Risk:** Trivial. Pure string edits, no behavior change.

**Files:**
- Modify: `crates/edgeplane/Cargo.toml:6` (the `description` clap reads for `about`)
- Modify: `crates/edgeplane/src/cli_schema.rs:202` and `:248` (hardcoded `.about(...)`)
- Modify: `crates/edgeplane/src/commands.rs:135` (`init` doc comment)
- Modify: `crates/edgeplane/src/main.rs:32` ("MC token" error string)
- Modify: `crates/edgeplane/src/evolve.rs:7` (module doc, surfaces in `agent evolve --help`)
- Modify: `crates/edgeplane/src/tui/screens/config.rs:773`, `:789`, `:822` ("MC URL" TUI labels)

- [ ] **Step 1: Replace the `about` description in all three sites.** Choose the canonical one-liner (proposed: `"EdgePlane — fleet control plane CLI"`). Edit `Cargo.toml:6` `description = "..."`, and both `cli_schema.rs:202` / `:248` `.about("...")`. All three MUST match (`cli_schema.rs:248` is a test helper — keep it in sync or the discover schema test drifts).

- [ ] **Step 2: De-stale `init` and the remaining "MC" hits.** `commands.rs:135` → `/// Initialize EdgePlane profile state for first-time usage.` Update `main.rs:32`, `evolve.rs:7`, and the three `tui/screens/config.rs` "MC URL" labels → "EdgePlane URL" (or "Controlplane URL").

- [ ] **Step 3: Verify no user-facing "MC"/"MissionControl" remain in help/TUI.** Run `rg -n "MissionControl|\bMC\b" crates/edgeplane/src crates/edgeplane/Cargo.toml`. Triage each remaining hit: user-facing strings/doc-comments → fix; pure internal comments → optional.

- [ ] **Step 4: Build + verify help text.** Run `cargo build -p edgeplane && ./target/debug/edgeplane --help | head -5`. Expected: new description, no "MCP bridge". Run `./target/debug/edgeplane agent evolve --help` — no "MC's".

- [ ] **Step 5: Commit.** `git commit -m "fix(cli): de-stale CLI about/init/help strings (MC → EdgePlane)"`

**Acceptance:** `edgeplane --help` shows the new description; no user-facing "MC"/"MissionControl" in `--help` output or the TUI config screen; discover schema test passes.

---

## WS-2: Bare `edgeplane` prints a colorized, ordered command tree

**Risk:** Low-medium. New render path; must not regress `edgeplane discover` JSON (consumed by the MCP `discover` meta-tool per ADR 0006).

**Files:**
- Modify: `crates/edgeplane/src/commands.rs:33` (add `arg_required_else_help`)
- Modify: `crates/edgeplane/src/cli_schema.rs:84-154` (`build_node` walker) and `:198` (`run`) — add a colorized human render mode
- Modify: `crates/edgeplane/src/commands.rs:81+` (`EdgeplaneCommand` enum order) — reorder to the agreed grouping

**Design note:** `build_node` currently emits **plain JSON only** (`cli_schema.rs:84-154`). Two options for the colorized tree: (a) bare `edgeplane` → clap's default help via `arg_required_else_help = true` (cheap, ordered, but clap help isn't grouped/colored beyond clap's own styling); (b) bare `edgeplane` → a new colorized grouped tree rendered from the `CapabilityNode` tree. **Recommendation: (b)** — render grouped sections with ANSI color, since clap's flat help won't express the consolidated grouping. Keep `discover` emitting JSON unchanged; add a sibling human renderer.

- [ ] **Step 1: Decide grouping = the WS-3 consolidated structure.** The tree's section headers come from WS-3's mapping. **This workstream must land with or after WS-3.** If WS-3 isn't ready, ship only `arg_required_else_help = true` (commands.rs:33) as an interim so bare `edgeplane` stops erroring, and defer the colorized grouped tree.

- [ ] **Step 2: Add a `render_tree_colored(root: &Command) -> String` helper** alongside `build_node` in `cli_schema.rs`. Group top-level commands by the WS-3 categories, ANSI-color group headers + command names (use a small color helper; respect `NO_COLOR` env and non-TTY → no color). Unit-test it renders all top-level commands and is plain when `NO_COLOR=1`.

- [ ] **Step 3: Wire bare invocation.** When no subcommand is given, print `render_tree_colored`. (Either via a custom check in `main.rs` before clap dispatch, or keep `arg_required_else_help = true` for the error path and add an explicit `edgeplane` no-arg branch that prints the tree.)

- [ ] **Step 4: Verify.** `cargo build -p edgeplane && ./target/debug/edgeplane` (no args) prints the grouped colored tree; `NO_COLOR=1 ./target/debug/edgeplane` prints uncolored; `edgeplane discover` still emits unchanged JSON (run the discover schema test).

- [ ] **Step 5: Commit.** `git commit -m "feat(cli): bare edgeplane prints colorized grouped command tree"`

**Acceptance:** bare `edgeplane` prints a grouped, colorized, ordered tree; `NO_COLOR`/non-TTY degrades to plain; `edgeplane discover` JSON contract unchanged.

---

## WS-3: Command-tree consolidation (Moderate, ~37 → ~25) — **DESIGN GATE**

**Risk:** High blast radius. Moving/renaming top-level commands breaks muscle memory, shell scripts, `~/.edgeplane/edgeplaned/cron.toml` job definitions, aria-repo CLAUDE.md/skills, and any docs. Mitigated by hidden back-compat aliases + a deprecation window.

**Entity note:** `domain`, `mission`, `task` are the canonical entity hierarchy (`docs/architecture/entities.md` §§ Domain, Mission, Task) and stay as top-level commands; only `ops` folds under `domain`.

- [ ] **Step 1 (DESIGN GATE — deliverable, requires Merlin sign-off before any code):** Produce `docs/superpowers/specs/2026-06-11-edgeplane-cli-tree.md` containing the complete old→new mapping for **all 37 top-level commands**, based on the approved Moderate target:
  - `status` absorbs `doctor`, `health`, `version`, `config`
  - `domain` absorbs `ops`
  - `agent` absorbs `run` (→ `agent run`)
  - `workspace` absorbs `release` (→ `workspace release`)
  - `capabilities` absorbs `exec` (→ `capabilities exec`) and `receipts` (→ `capabilities receipts`)
  - everything else stays top-level
  The spec must specify, per moved command: the new path, whether a **hidden deprecation alias** is kept, the deprecation message, and the impact on `edgeplane discover` JSON (consumers must not hard-break). Include a grep-derived list of every call site in this repo + the aria repo + cron.toml that references a moved command.

- [ ] **Step 2: Implement per the approved spec.** Restructure the clap `EdgeplaneCommand` enum (`commands.rs:81+`); add hidden alias subcommands (`#[command(hide = true)]`) that forward to the new location and print a one-line deprecation notice to stderr.

- [ ] **Step 3: Update in-repo + aria-repo references** found in Step 1 (cron.toml job commands, CLAUDE.md skill dispatch tables, docs). Coordinate the aria-repo edits — shared working tree (`[[feedback_shared_working_tree_git_collisions]]`).

- [ ] **Step 4: Verify.** `cargo build -p edgeplane`; spot-check moved commands work at new path and old path (alias → deprecation notice + still functions); `edgeplane discover` JSON regenerated and the schema test updated intentionally.

- [ ] **Step 5: Commit.** `git commit -m "refactor(cli): moderate command-tree consolidation with back-compat aliases"`

**Acceptance:** new ~25-command tree; every moved command still works via a hidden, deprecation-warning alias; discover schema updated; all repo/cron/CLAUDE.md references updated.

---

## WS-4: Token / auth UX cleanup

**Risk:** Low. One stale string + one flag rename with alias.

**Files:**
- Modify: `crates/edgeplane/src/daemon_ctl.rs:2325` (empty-state hint)
- Modify: `crates/edgeplane/src/daemon_ctl.rs:363-367` (`--bootstrap-token` arg def)
- Audit: `crates/edgeplane/src/daemon_ctl.rs:2014` (`--token` passed to `edgeplaned run`)

**Finding:** `daemon profile add` has **no `--token` flag** — the hint references a nonexistent flag. Auth is OIDC (`mcs_*`); the real node-enrollment credential is `--bootstrap-token` (one-time join token from `edgeplane node ... join-token create` → `POST /runtime/nodes/register`).

- [ ] **Step 1: Fix the empty-state hint.** `daemon_ctl.rs:2325` → `"No profiles saved. Add one: edgeplane daemon profile add <name> --url <url>  (add --join-token <tok> to also enroll this node)"`.

- [ ] **Step 2: Rename `--bootstrap-token` → `--join-token`.** Add `#[arg(long, visible_alias = ...)]` — actually keep `--bootstrap-token` as a **hidden** alias: `#[arg(long = "join-token", alias = "bootstrap-token")]` so existing callers don't break. Update the arg's help text to describe it as a one-time node join token.

- [ ] **Step 3: Audit `daemon_ctl.rs:2014`.** Confirm whether `edgeplaned run` still accepts/uses the passed `--token`. If edgeplaned now reads auth solely from `state.json`, remove the stale arg pass-through; if still used, leave with a clarifying comment. Document the finding in the commit.

- [ ] **Step 4: Verify.** `cargo build -p edgeplane`; `edgeplane daemon profile add --help` shows `--join-token` (not `--token`/`--bootstrap-token` in the visible help); `edgeplane daemon profile add x --url y --bootstrap-token z` still parses (hidden alias); `edgeplane daemon profile list` (empty) prints the corrected hint.

- [ ] **Step 5: Commit.** `git commit -m "fix(cli): correct daemon profile hint; rename --bootstrap-token to --join-token (alias kept)"`

**Acceptance:** empty-state hint shows only valid flags; `--join-token` is the documented flag; `--bootstrap-token` still works silently; `:2014` audited and resolved.

---

## WS-5: Admin → Create Join Token (Web UI)

**Risk:** Low. Backend endpoint already exists (`POST /runtime/tokens` → `create_join_token`, `crates/edgeplane-tower/src/routes/runtime.rs:568`, requires an authenticated Principal). Net-new frontend only.

**Files:**
- Modify: `web/openapi.json` (add `POST /api/runtime/tokens` path + `JoinTokenCreate`/response schemas), then run `npm run gen:api` → regenerates `web/src/api/schema.gen.ts`
- Create: `web/src/routes/admin.tsx` (layout route `createFileRoute('/admin')`, `<Outlet/>`)
- Create: `web/src/routes/admin.index.tsx` (the create-join-token page: form + `useMutation` POST + `useQuery` list via `GET /runtime/tokens/{id}` patterns; show raw `token` once on success — it is only returned at creation)
- Modify: `web/src/components/shell/navModel.ts:10-21` (add `{ to: '/admin', label: 'Admin' }`)
- Modify: `web/src/components/shell/Sidebar.tsx` (`NAV_ICON` map ~line 36, add `/admin` icon)
- Modify: `web/src/lib/queryKeys.ts` (add `joinTokens` namespace)
- Auto-regenerated (do not hand-edit): `web/src/routeTree.gen.ts`

- [ ] **Step 1: Add the endpoint to the typed client.** Add `POST /api/runtime/tokens` (body: `expires_in_seconds`, `upgrade_channel`, `desired_version`, `config`) and its response to `web/openapi.json`; run `npm run gen:api`; confirm `schema.gen.ts` now has the path. (Interim fallback if schema work is deferred: use the untyped `api.post(...)` helper from `web/src/lib/api/http.ts`.)

- [ ] **Step 2: Create the admin routes.** `admin.tsx` layout + `admin.index.tsx` page. The page: a form (expiry, channel), `useMutation` → create, success panel that displays `token` once with a copy button and a "won't be shown again" warning, and a list of existing tokens (id/status/expires_at) via `useQuery`.

- [ ] **Step 3: Wire nav.** Add the Admin entry to `navModel.ts` + icon in `Sidebar.tsx`. Start the dev server (`npm run dev` in `web/`) so `routeTree.gen.ts` regenerates; confirm `/admin` resolves.

- [ ] **Step 4: Verify end-to-end against the tower.** With a logged-in session, create a token on `/admin`; confirm the raw token appears once, a `runtimejointoken` row is created (status `active`), and it works as a `--join-token` for `edgeplane daemon profile add`.

- [ ] **Step 5: Commit.** `git commit -m "feat(web): admin page to mint node join tokens"`

**Acceptance:** `/admin` page mints a working join token via the existing backend; token shown once; existing tokens listed; nav entry present.

---

## WS-6: Typed entity-ID prefixes + legacy migration — **DESIGN GATE, highest risk**

**Risk:** High. The prefix is the **stored primary key**, referenced as a string across **24 tables** (mostly without FK enforcement; only `agent.home_domain_id` / `agent.current_domain_id` have FK constraints to `domain(id)`). Live prod data exists (the observed `k_69bce6a2f4e5` mission). Touches Rust id-gen, Python persistence, Alembic, URL routing, S3 artifact paths, and MCP `context.json`.

**Entity note (`docs/architecture/entities.md`):** Domain (§ Domain, org boundary), Mission (§ Mission, workstream — the legacy `k_`/"kluster" rows), Task (§ Task), MeshTask (§ MeshTask, UUID today). Current Rust id-gen emits **bare 12-hex, no prefix** (`domains.rs:30`, `missions.rs:38`); `k_` is pre-fork Python-era legacy data only.

- [ ] **Step 1 (DESIGN GATE — deliverable, requires Merlin sign-off before any code):** Produce `docs/superpowers/specs/2026-06-11-typed-entity-ids.md` covering:
  - **Prefix scheme:** `d_` domain, `m_` mission, `t_` task; decide whether MeshTask (currently UUID, `work.rs:563`) gets a prefix or stays UUID.
  - **Emit-new:** change `new_hash_id()` call sites per entity (`domains.rs:30`, `missions.rs:38`, and `tasks.rs` public_id) to prepend the type prefix.
  - **Dual-read window:** at every path-param handler accepting `domain_id`/`mission_id`/`task_id`, accept both prefixed and legacy/bare forms (DB lookup passes the raw stored value; no rewriting in app code).
  - **Data migration:** the Alembic/SQL plan to rewrite legacy `k_` mission IDs → `m_` and add `d_` to existing domain IDs, across all 24 tables enumerated below, in dependency order, with the two FK-constrained domain columns handled via deferred constraints. Include verification queries (`SELECT count(*) ... WHERE id LIKE 'k_%'` → 0) and a rollback plan.
  - **Side effects:** S3 artifact path layout (`domains/{domain_id}/missions/{mission_id}/...`) and any stored `artifact.uri`; stale `active_domain_id`/`active_mission_id` in `~/.edgeplane/instances/*/edgeplane/context.json`; Web UI URL assembly (`web/src/routes/domains.$domainId.tsx`).
  - **24 tables to migrate:** approvalrequest, artifact, doc, domainpersistencepolicy, domainpersistenceroute, domainrolemembership, epic, evolvedomain, evolverun, feedbackentry, ingestionjob, ledgerevent, meshagent, meshmessage, meshtask, mission, publicationrecord, runtimejob, skillbundle, skilllocalstate, skillsnapshot, slackchannelbinding, task, workspacelease (+ `agent.home_domain_id`, `agent.current_domain_id`).

- [ ] **Step 2: Phase A — emit-new + dual-read shim.** Implement prefix emission on new rows + accept-old normalization at read sites. Ship and verify new entities get prefixes and existing `k_`/bare IDs still resolve. (Detailed bite-sized steps written against the approved Step-1 spec.)

- [ ] **Step 3: Phase B — background data migration.** Run the table-by-table migration from the spec on prod (backup first); verify zero legacy rows remain.

- [ ] **Step 4: Phase C — remove the dual-read shim** once `SELECT count(*) WHERE id LIKE 'k_%'` is 0 across mission + the 24 tables.

- [ ] **Step 5: Verify + commit each phase separately.** Each phase is its own commit/PR; never bundle the data migration with code changes.

**Acceptance:** new entities carry typed prefixes; all legacy `k_` rows migrated to `m_`; domains carry `d_`; URLs == stored PKs; no orphaned references; S3 paths and context.json reconciled.

---

## WS-7: `signal` CLI `register_agent` regression (carryover)

**Risk:** Low, mechanical, fully diagnosed 2026-06-11.

**Files:**
- Modify: `crates/edgeplane/src/signal.rs:123-145` (`ensure_sender_agent`)

**Finding:** `ensure_sender_agent` POSTs `register_agent` to `/mcp/call`, but ADR 0006 deleted that MCP arm → every `edgeplane agent signal` (default and `--remote`) hard-fails with `register_agent failed: unknown_tool` before the message is posted. Both `POST /agents` and `POST /agents/{id}/message` exist on the tower (`routes/agents.rs:24`, `:31`).

- [ ] **Step 1: Repoint `ensure_sender_agent` to `POST /agents`.** Replace the `/mcp/call`+`register_agent` body with a `POST /agents` call mirroring `agent_ops.rs::run_register` (`{name, capabilities}`); read `public_id` (fallback `id`) from the response.

- [ ] **Step 2: Verify end-to-end.** `cargo build -p edgeplane`; run `edgeplane agent signal <recipient-public-id> --content "test"` and confirm it registers the sender, posts the message, and the recipient's ACP session picks it up (`session/prompt`). This last hop was NOT yet live-tested — confirm it here.

- [ ] **Step 3: On success, update the interim guidance.** Revert `.claude/rules/ep-registration.md` (aria repo) to document `signal` as the working primitive again (keep the mesh path documented as the alternative). Update `[[feedback_mesh_handoff_use_send_mesh_message]]`.

- [ ] **Step 4: Commit.** `git commit -m "fix(cli): signal sender registration via POST /agents (ADR 0006 regression)"`

**Acceptance:** `edgeplane agent signal` works end-to-end (default + `--remote`); recipient receives the prompt; interim mesh-path workaround documentation reverted.

---

## Self-review

- **Spec coverage:** every item from Merlin's list maps to a workstream — about string→WS-1, bare-cmd→WS-2, command order→WS-2, init desc→WS-1, colorize→WS-2, consolidation→WS-3, `--token` hint→WS-4, admin join-token UI→WS-5, `--token`/`--join_token` rename→WS-4, `k_`→`m_`/`d_`→WS-6. Plus carryover signal fix→WS-7.
- **Reframes surfaced:** `daemon profile add` has no `--token` (WS-4); domains have no `m_` prefix today, only legacy `k_` missions (WS-6) — both reflected in the tasks, not silently "fixed" to the original wording.
- **Gates:** the two irreversible workstreams (WS-3, WS-6) require a signed-off design spec before code; their Step-1 deliverables are concrete artifacts, not placeholders.
- **Back-compat:** renames (WS-3, WS-4) ship hidden aliases; the ID migration (WS-6) uses a dual-read window — no hard breaks.
