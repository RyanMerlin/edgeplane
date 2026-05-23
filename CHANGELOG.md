# Changelog

All notable changes to edgeplane, edgeplaned, and edgeplane-tower are recorded here. Starting with 0.11.0 the three binaries ship in lockstep — `/VERSION` is the source of truth and `scripts/set-version.sh <new>` bumps all three in one step.

This project follows semantic versioning where possible, but pre-1.0 minor bumps may include breaking changes when the cost of a major bump outweighs the signal value.

## [0.15.11] — 2026-05-21

### Added — `dispatch = "bash"` cron tier

Third dispatch mode for edgeplaned cron jobs, alongside the existing `signal` (inject prompt into a profile session) and `goose` (run prompt through local Qwen3.6-27B). `dispatch = "bash"` treats the `prompt` field as a literal shell script and executes via `bash -c`. No session, no signal, no LLM round-trip, no prompt-injection cost — pure subprocess execution.

Designed for the cron jobs that have always been pure shell work — `aria vault mirror`, `aria browser screenshot`, `edgeplane mesh msg list` etc. Previously these had to be wrapped in either a `signal` dispatch (overkill, polluted a profile session) or a `goose` dispatch (waste, LLM categorizing trivial output). Now they have a tier that matches what they actually are.

**Execution:**
- Command: `bash -c "<prompt>"`
- Captured: stdout/stderr → edgeplaned cron log; exit code → cron history (`last_status: "ok"` / `"failed"`)
- Timeout: 5 minutes (same as goose)
- Env: standard env + `MC_CRON_JOB_NAME`, `MC_CRON_FIRE_TS`, `MC_CRON_DISPATCH=bash`
- Works with both `kind = "cron"` and `kind = "heartbeat"` timing tiers

**Migrated `vault-mirror` job** as validation:
```toml
# Before:
[[job]]
name     = "vault-mirror"
schedule = "0 3 * * *"
session  = "operator"
prompt   = "Bash: aria vault mirror — log any errors to .learnings/ERRORS.md, no output needed on success"

# After:
[[job]]
name     = "vault-mirror"
schedule = "0 3 * * *"
dispatch = "bash"
prompt   = """aria vault mirror 2>>/home/merlin/code/aria/.learnings/ERRORS.md"""
```

Other candidates (`mesh-inbox-sweep`, `browser-session-refresh`) can migrate when their owners are ready.

## [0.15.10] — 2026-05-21

### Added — `POST /work/tasks/{id}/dispatched` admin transition endpoint

Single-call transition `ready` → `finished` for admin or task owner. Designed specifically for the triage routing pattern: the triage layer creates a child meshtask under the routed kluster (which carries the work) and the intake task itself needs to transition to a terminal state without the claim-then-complete dance that `complete_task` requires (because `complete_task` only allows `claimed`/`running`/`waiting_review` transitions). Idempotent on already-finished tasks.

### Changed — P3 triage `route_task` now uses the new endpoint

Replaces the 4-call temp-agent dance (enroll triage agent + claim + complete + delete agent) with a single `POST /work/tasks/{id}/dispatched`. Net diff: `-70` lines in `task_worker.rs`, no behavior change, no Goose-side migration needed. The intake task's terminal state is still `finished`; the child task still carries the work; `parent_task_id` chain still intact.

## [0.15.9] — 2026-05-21

### Removed — Dead `provision_home_for_node` helper (per-node home mission carcass)

Deleted `provision_home_for_node` and its helper `slug_hostname` from `crates/edgeplane-tower/src/routes/runtime.rs`, plus the test module `home_mission_tests` and the callsite in `register_node`. All were dead code after the walk-back of the per-node home-mission pattern across 0.15.4 and 0.15.8.

The helper was:
- Set `Mission.kind='home'` (a column we soft-deprecated in 0.15.4)
- Auto-created `home-{slug(hostname)}` missions on node registration (a pattern we walked back in favor of edgeplaned's bootstrap module creating a single global `home` mission)
- Has had zero in-production callers since no runtime nodes have ever been registered against this deployment

Net diff: -160 lines from `routes/runtime.rs`. No new tests; the deleted ones tested deleted code.

**Not removed in this commit (separate follow-up):** `edgeplane daemon agent enroll-home` CLI subcommand in `crates/edgeplane/src/daemon_ctl.rs`. It's a standalone mirror that writes to the local registry directly (doesn't go through the controlplane helper we deleted), so it still compiles and "works" — but it encodes the same retired per-node pattern. Should be retired or repurposed in a follow-up. The edgeplaned-side references to it (in `daemon.rs` comments) are pointers at this still-functional CLI command.

## [0.15.8] — 2026-05-21

### Changed — Aria/MC separation (second walk-back)

Decoupled edgeplaned from Aria-specific defaults. Same kind of cleanup as the `Mission.kind='home'` walk-back in 0.15.4: MC is a generic platform; Aria is one consumer. Defaults must not leak the consumer's identity into the platform.

**Bootstrap default mission name renamed.**
- `DEFAULT_HOME_MISSION_NAME`: `"aria-fleet-ops"` → `"home"`. The word "home" is generic, conceptually correct (this IS the default home for unscoped work), and aligns with the existing `Agent.home_mission_id` column.
- Env override renamed: `MC_OPS_MISSION_NAME` → `EP_HOME_MISSION_NAME`.
- Aria deployments that want to keep the old name set `EP_HOME_MISSION_NAME=aria-fleet-ops` in their edgeplaned env.

**Triage surface decoupled from Aria vault.**
- P3's triage previously hardcoded `aria vault note append --path mc-engineer/inbox.md ...` for the low-confidence surface. That baked Aria's vault layout and a specific Aria profile name into MC's default behavior.
- New: edgeplaned always marks the intake task as `blocked` (the MC-native surface — discoverable via `edgeplane task ls --status blocked`). If a deployment configures `task_worker_surface_command: Option<Vec<String>>`, edgeplaned ALSO invokes that command with `<task_id> <title> <reason>` appended, so deployments can chain external alerts (vault notes, Slack, GitHub Issues, email) without MC encoding any particular interface.
- Aria's edgeplaned config keeps current behavior with:
  ```toml
  task_worker_surface_command = [
    "aria", "vault", "note", "append",
    "--path", "mc-engineer/inbox.md",
    "--section", "Triage Inbox"
  ]
  ```
  but that's a deployment concern, not an MC default.

**Net result:** MC ships generic. Aria configures its specifics in env + edgeplaned config. The platform/consumer boundary is now honest.

## [0.15.7] — 2026-05-21

### Added — Capability enforcement (P4) — completes the ephemeral subagent build

`edgeplaned::capabilities` — translates `meshtask.required_capabilities` into a `claude -p --allowed-tools` flag at subagent spawn time. The final phase of the ephemeral task subagent build (see `docs/design/ephemeral-task-subagents.md` § Decision 3). Per-task tool surface is now restricted to what the dispatcher declared.

**Capability vocabulary (v1, hardcoded):**

| Capability | Coverage |
|------------|----------|
| `shell:read` | Read-only shell: `ls`, `cat`, `head`, `tail`, `grep`, `find`, `pwd`, `echo`, `date` |
| `shell:write` | Full bash (`Bash(*)`) |
| `fs:read` | `Read`, `Glob`, `Grep` |
| `fs:write` | `Read`, `Write`, `Edit`, `Glob`, `Grep` |
| `vault:read` | `aria vault note read`, `aria vault note list`, `aria vault search` |
| `vault:write` | Subsumes vault:read + `aria vault note write/create/patch/append` |
| `edgeplane:read` | `edgeplane agent ls`, `edgeplane daemon agent ls`, `edgeplane daemon task ls`, `edgeplane agent cron list/describe`, `edgeplane status` |
| `edgeplane:write` | Subsumes edgeplane:read + `edgeplane daemon task submit`, `edgeplane daemon agent enroll`, `edgeplane agent signal` |
| `web:fetch` | `WebFetch`, `WebSearch` |
| `gh:read` | `gh repo view`, `gh issue view/list`, `gh pr view/list`, `gh run list/view`, `gh api` |
| `gh:write` | Subsumes gh:read + `gh issue create/comment`, `gh pr create/comment` |

Required-capabilities format: JSON array of strings in the `meshtask.required_capabilities` TEXT column, e.g. `'["fs:read","vault:write"]'`. Subsuming caps deduplicate automatically.

**Strict vs lenient mode** (new config flag `task_worker_strict_capabilities`, default `false`):
- **Lenient (default):** missing or empty `required_capabilities` → use `task_worker_default_capabilities` (default `["fs:read", "shell:read"]` — read-only fs + read-only shell). Safe default for casual dispatchers.
- **Strict:** missing or empty → FAIL the task with reason "task missing required_capabilities — dispatcher must declare blast radius." The long-term target — forces dispatchers to think before submitting work.

**Unknown capability** → task fails immediately with `unknown capability: <name>` error. No silent passthrough.

**Verified `--allowed-tools` syntax** via `claude --help`: accepts both `--allowed-tools` and `--allowedTools`; comma OR space-separated; `Bash(<prefix> *)` form for granular Bash subcommand restriction, `Bash(*)` for unrestricted bash.

**Audit:** the resolved tool set is logged at INFO level at each subagent spawn — a per-task record of "this subagent was allowed to call X, Y, Z."

### Status — ephemeral task subagent build complete

All four phases shipped:
- ✅ P1 (bootstrap) — 0.15.4
- ✅ P2 (claimer loop) — 0.15.5
- ✅ P3 (triage) — 0.15.6
- ✅ P4 (capability enforcement) — 0.15.7 (this)

The end-to-end loop: dispatcher submits meshtask → triage routes (via goose categorization if unscoped) → claim → spawn ephemeral `claude -p` subagent with restricted tool surface → complete → MeshAgent deleted (FK preserves AgentRun audit). Each phase remains independently disable-able via config. Activation comes when something dispatches tagged work into the intake kluster.

**Open follow-ups** (not blocking; tracked elsewhere):
- `dispatch = "bash"` cron tier (vault note in `mc-engineer/projects/`)
- `provision_home_for_node` cleanup (still writes `kind='home'` dormant)
- Admin `POST /work/tasks/{id}/triaged` endpoint to simplify the P3 4-call routing dance

## [0.15.6] — 2026-05-21

### Added — Triage loop (P3)

`edgeplaned::task_worker::run_triage_loop` — a second long-running tokio task spawned alongside the P2 claim loop. Picks up unscoped tasks from the intake kluster (created by P1 bootstrap), categorizes them via local Qwen3.6-27B (via `aria goose`), and either routes them to the appropriate profile or surfaces them for human review.

**Three-tier triage** (per `docs/design/ephemeral-task-subagents.md` § Decision 5):
1. **Rule (P2):** task has `claim_policy.target_profile` set → P2's claim loop handles it directly. Skipped by triage.
2. **Goose (P3):** task is in intake kluster, status=`ready`, no `target_profile`. Triage builds a categorization prompt listing supervised profiles, invokes `aria goose "<prompt>" --timeout 30`, parses the response. If `confidence >= triage_confidence_threshold` (default `0.85`) AND target_profile is supervised on this node → ROUTE.
3. **Surface (P3):** low confidence, missing target_profile, unparseable response, or unsupervised profile → BLOCK + write to `mc-engineer/inbox.md`.

**Routing mechanic** (per § Decision 5 / S2 — child task, NOT kluster_id rebind):
- Create child meshtask in the same intake kluster with `claim_policy.target_profile` set, `parent_task_id` pointing at the intake task.
- Mark intake task `status=finished` so it's not re-triaged. Because the controlplane's `complete_task` requires the task to be in `claimed`/`running`/`waiting_review` state, the triage loop briefly enrolls a temp agent, claims the intake task, completes it, then deletes the temp agent (FK preserves audit). 4 extra HTTP calls per route; acceptable at triage scale (max 5/cycle).
- The child task lives in the intake kluster (per Decision 5 — we walked back per-profile scratch klusters). P2's claim loop picks it up by polling for `target_profile`-set tasks across all klusters.

**Surface mechanic** (low-confidence path):
- `aria vault note append --path mc-engineer/inbox.md --section "Triage Inbox" "<entry>"` with timestamp + task id + title + goose's reasoning. Append, not overwrite.
- Intake task transitioned to `status=blocked` via `POST /work/tasks/{id}/block` (which has no status-precondition, unlike complete). Blocked tasks are skipped by both loops on subsequent polls. Human resolves by editing `claim_policy.target_profile` and unblocking.

**Skip-already-triaged check:** the triage filter is "status=ready AND no target_profile in claim_policy." Once routed (finished) or surfaced (blocked), the intake task no longer matches → no re-triage. No metadata flag needed.

**New config keys (with `#[serde(default)]`):**
- `task_worker_triage_enabled: bool` (default `true`)
- `task_worker_triage_poll_interval_secs: u64` (default `60` — half the claim cadence)
- `task_worker_triage_confidence_threshold: f64` (default `0.85`)
- `task_worker_max_triage_per_cycle: usize` (default `5`)
- `task_worker_goose_timeout_secs: u64` (default `30`)

**Activation conditions:** triage activates when any dispatcher submits an unscoped meshtask into the intake kluster. The combined P2+P3 system is now feature-complete except for capability enforcement (P4). All upstream wiring (`fleet-self-health`, future health-fold-into-briefing, etc.) is still TBD.

### Finding (controlplane API quirk worth flagging)

`POST /work/tasks/{id}/complete` requires the task to be in `claimed`/`running`/`waiting_review` state — cannot complete a `ready` task directly. Triage works around this via the temp-agent + claim + complete sequence. A direct admin transition endpoint (`POST /work/tasks/{id}/triaged` or similar) would simplify, but isn't critical at current scale.

## [0.15.5] — 2026-05-21

### Added — Ephemeral task subagent claimer loop (P2)

`edgeplaned::task_worker` — long-running tokio task spawned at daemon startup that polls the controlplane for claimable meshtasks, enrolls ephemeral MeshAgents, spawns `claude -p` subprocesses in per-task worktrees, and cleans up on completion. See `docs/design/ephemeral-task-subagents.md` § Decision 4 (edgeplaned module, per-node sharding) and § Decision 1 (delete MeshAgent on completion).

**Behavior:**
- Polls every `task_worker_poll_interval_secs` (default 30) for meshtasks with status='ready' whose `claim_policy` contains `{"target_profile": "<name>"}` matching a profile supervised on this node.
- Enrolls an ephemeral MeshAgent under the parent profile's identity (`labels.role=task-subagent, labels.ephemeral=true, labels.task_id=<id>`).
- Claims the task, opens an `AgentRun`, spawns `claude -p` with cwd = `~/.ep/worktrees/<task_id>/`, captures the result, completes the AgentRun and meshtask, deletes the MeshAgent (FK preserves AgentRun audit).
- Concurrency cap at `task_worker_max_concurrent` (default 3).
- Soft-fail throughout — any task error logs a warning and continues; daemon never crashes.

**Scope limit:** P2 only handles tasks with `target_profile` set explicitly. Tasks landing in the `intake` kluster without that field are skipped (P3 triage handles those).

**No capability enforcement yet:** subagents currently receive the full claude tool surface. P4 will restrict via `claude -p --allowed-tools` driven by `required_capabilities`.

**New config keys (with `#[serde(default)]`, no migration needed):**
- `task_worker_enabled: bool` (default `true`)
- `task_worker_poll_interval_secs: u64` (default `30`)
- `task_worker_max_concurrent: usize` (default `3`)
- `task_worker_subagent_command: String` (default `"claude"`)

The loop is dormant in practice until something dispatches `target_profile`-tagged tasks — polls find nothing, sleep, repeat. Activation comes when P3 wires triage or a dispatcher (e.g. future health-fold-into-briefing) submits tagged work.

### Documented

- `ExecutionSession` step removed from the lifecycle diagram in the design doc. The entity exists in the controlplane (`routes/runtime.rs:423`) but is shaped for attachable PTY sessions with attach tokens — overkill for headless `claude -p`. AgentRun + OS process is sufficient audit. Prototype confirmed this empirically.

## [0.15.4] — 2026-05-21

### Added

- **edgeplaned bootstrap module** — `crates/edgeplaned/crates/edgeplaned/src/bootstrap.rs`. On daemon startup (after `fleet_import`), idempotently ensures a fleet operations mission (default `aria-fleet-ops`, overridable via `MC_OPS_MISSION_NAME`) and an `intake` kluster under it exist in the controlplane. Soft-fails on controlplane unreachable. Unblocks Phase 2 of the ephemeral task subagent build (`docs/design/ephemeral-task-subagents.md`).
- **`KlusterCreate.workstream_md`** — `crates/edgeplane-tower/src/models/kluster.rs` accepts the workstream narrative on creation (optional, default empty). `create_kluster` handler now persists it and stamps the workstream metadata fields (created_by / created_at) when provided. Previously the field was silently dropped by serde.

### Deprecated

- **`Mission.kind`** (column added in migration 0006). Soft-deprecated. Audit determined the column was set by exactly one code path (`provision_home_for_node`) and read by zero — a write-only tag leaking an Aria-specific operational pattern into MC's schema. New code MUST NOT write or filter on it. `Agent.home_mission_id` was never constrained to `kind='home'` and is unaffected. See `docs/architecture/entities.md` § Mission and `docs/design/ephemeral-task-subagents.md` decision log.

### Architecture

- **Walked back per-node `home-{hostname}` missions** in favor of a single fleet-level operations mission. Per-node coordination, when needed in a multi-node future, will use per-node *klusters* under one mission instead of per-node missions. Cleaner aggregation, single source of truth.
- **Decision log added** to `docs/design/ephemeral-task-subagents.md` capturing locks from the review session: ephemeral delete (not archive), no resume tokens v1, restricted capabilities via `--allowed-tools`, edgeplaned module for the spawner, fleet-ops + intake replacing per-node home, child meshtasks for routing, and CLI (not MCP) for missing write surfaces.

## [0.15.3] — 2026-05-21

### Added

- **`DELETE /work/agents/{agent_id}`** — admin-or-owner endpoint to delete a meshagent row directly, independent of runtime-node assignment. Unblocks the ephemeral task subagent model (see `docs/design/ephemeral-task-subagents.md`) where the spawner needs to clean up its own meshagents without runtimenode bookkeeping. Authorized for `principal.is_admin` OR `meshagent.enrolled_by_subject == principal.subject`. FK on `agentrun.mesh_agent_id` is `ON DELETE SET NULL`, so the audit trail survives. `crates/edgeplane-tower/src/routes/work.rs`.

### Fixed

- **`/runs` POST silently dropped column-named FK fields.** `StartRunRequest` declared `agent_id` / `task_id` but bound to `agentrun.mesh_agent_id` / `mesh_task_id` columns server-side. Callers using column names (which is the natural convention) got NULL FKs because serde dropped the unknown keys. Added `#[serde(alias = "mesh_agent_id")]` / `#[serde(alias = "mesh_task_id")]`; both naming conventions now work. `crates/edgeplane-tower/src/models/run.rs`.

### Documented

- **Ephemeral task subagent identity model** — full design + schema audit + lifecycle walkthrough at `docs/design/ephemeral-task-subagents.md`. Companion prototype that walks the lifecycle end-to-end against a live controlplane at `scripts/proto/ephemeral-subagent.sh`. Resolves the architectural question of how MC spawns dispatched work without polluting persistent profile sessions.

## [0.15.2] — 2026-05-20

### Fixed

- **CI: `build-image.yml` was bumping the gitops manifest to the wrong tag.** `docker/metadata-action` with `type=sha` publishes images as `:sha-<short>` (default prefix), but the workflow's gitops `sed` was rewriting the image to `:<short>` (no prefix). Every recent CI-driven deployment landed on an image that didn't exist (ImagePullBackOff). The fix matches the actual published tag.

### Added

- **Cron eval regression tests** for two scenarios that we'd flagged as suspected bugs on 2026-05-20 but turned out to be uptime / config-edit artifacts (no eval bug). Tests guard against the same symptom shape reappearing:
  - `daily_9am_with_recent_afternoon_anchor_is_not_due`
  - `daily_530am_with_prior_afternoon_anchor_is_due_next_morning`

  See `mc-engineer/projects/2026-05-20-...-edgeplaned-cron-eval-bugs-...md` (vault) for the diagnosis trail.

## [0.15.1] — 2026-05-20

### Fixed

- **`edgeplaned.service` unit now sets `Environment=PATH=`** explicitly, so `dispatch = "goose"` cron jobs can find the `aria` binary (and any other tool installed under `~/.cargo/bin` or `~/.local/bin`). systemd user services get a minimal PATH by default, which was breaking goose-dispatch fires silently. `crates/edgeplaned/systemd/edgeplaned.service`.

### Changed

- **Heartbeat cron jobs no longer require a `schedule = ""` field.** `CronJob::schedule` is now `#[serde(default)]`; heartbeat jobs can omit the field entirely. Cron jobs explicitly reject empty `schedule` with a clear error message. Existing files with `schedule = ""` on heartbeat jobs continue to parse unchanged. `crates/edgeplaned/.../cron_config.rs`.
- **`edgeplane agent cron list` and `describe` now render heartbeat cadence.** Heartbeat jobs show `heartbeat: 30m` in the SCHEDULE column instead of empty; describe surfaces a dedicated `kind:` line and switches between `schedule:` and `interval:` accordingly. The `agent.cron.list` / `agent.cron.describe` JSON-RPC responses now include `kind` and `interval`. `crates/edgeplane/src/agent_cron.rs`, `crates/edgeplaned/.../mgmt_gateway.rs`.

## [0.15.0] — 2026-05-20

### Added — Cron heartbeat tier (`kind = "heartbeat"`)

`~/.ep/edgeplaned/cron.toml` now accepts heartbeat-style jobs that fire on a duration cadence instead of a cron expression. Useful for periodic polls where exact clock alignment doesn't matter (queue sweeps, commitment checks, drift detection).

```toml
[[job]]
name     = "my-heartbeat"
kind     = "heartbeat"
interval = "30m"          # "Ns" / "Nm" / "Nh" / "Nd" or compound "2h30m"
session  = "operator"
prompt   = "run /my-check"
```

Fires whenever `now - last_fire >= interval`. Bootstrap: a heartbeat with no prior fire is due immediately on the next dispatcher tick. Tolerant of the 60s dispatcher tick — a `30m` heartbeat actually fires every 30–31 minutes in practice. The `schedule` field is ignored for heartbeats (operators may omit or leave empty).

`schema_version` stays at `1` — the change is additive (existing files using `kind = "cron"` continue to work unchanged).

### Added — Cron goose dispatch (`dispatch = "goose"`)

Jobs can now route through `aria goose "<prompt>"` for local LLM execution (Qwen3.6-27B via LiteLLM, zero API cost). No agent attachment needed — useful for periodic computation that doesn't belong to any specific profile: auto-summaries, log digests, drift checks.

```toml
[[job]]
name     = "nightly-fleet-summary"
schedule = "0 22 * * *"
dispatch = "goose"
session  = "operator"     # metadata tag only when dispatch="goose"; not a real agent attachment
prompt   = """
Summarize today's fleet activity from /var/log/...
"""
```

- Honors `MCD_GOOSE_BIN` env override; defaults to `aria` from PATH.
- 5-minute subprocess timeout (goose runs are short-lived; for longer work use `dispatch = "signal"` to a real agent).
- Failure surfaces in `edgeplane agent cron history` with the actual goose stderr.

### Fixed — Empty `systemd_service` in `unit_restarted` events

The `SupervisorEvent::UnitRestarted` published by `agent.supervise.restart` was shipping with `systemd_service: ""`. The restart handler's blocking task resolved the service name internally but didn't return it to the event-publish path. Now threads the value through. Validated end-to-end on excalibur (work profile).

### Internals

- `cron_config::parse_duration("2h30m")` → `Duration` — accepts `Ns`/`Nm`/`Nh`/`Nd` and compound forms. Rejects empty, missing unit, unknown unit, and zero values
- Loader requires `interval` when `kind = "heartbeat"`; rejects unknown `kind` / `dispatch` values with clear errors
- `eval_job` branches on `kind` (cron vs heartbeat due-check) and `dispatch` (signal vs goose firing path)
- Telemetry: cron fire log records the dispatch outcome regardless of mode; status field unchanged

### Tests

- `parse_duration_handles_compound` + `parse_duration_rejects_bad_input`
- `accepts_heartbeat_with_interval` + `rejects_heartbeat_without_interval`
- `accepts_goose_dispatch`

edgeplane 133/133, **edgeplaned 285/285** (+3 from this PR), edgeplane-tower 48/48 — all pass.

### Migration

None required — pure additive change. Existing `cron.toml` files with `kind = "cron"` and `dispatch = "signal"` work unchanged.

## [0.14.0] — 2026-05-20

### Added — Live fleet dashboard (`edgeplane agent supervise watch`)

Phase B (v0.13.0) wired `events.subscribe` to a streaming consumer protocol; this release adds the first interactive consumer that actually renders the stream.

- **`edgeplane agent supervise watch [--poll-secs N] [--tail-size N]`** — ratatui TUI with:
  - **Top pane:** agent table (one row per supervised agent), polled from `agent.supervise.list` every `--poll-secs` (default 5s). Columns: AGENT, SYSTEMD, STATE (color-coded green/red/yellow by unit_state), PAUSED, LAST (most recent event timestamp + label).
  - **Bottom pane:** scrolling event tail, fed by `events.subscribe`. Newest at the bottom. Color-coded by kind (red=dead, yellow=restart, cyan=pause/resume, magenta=nightly).
  - **Header:** stream status indicator (connecting / live / closed / error) + snapshot age.
  - **Footer:** keybinds + agent/event counts.
- **Resilience:** the streamer reconnects automatically if edgeplaned hangs up. Snapshot poller continues independently of stream state, so even if streaming breaks the table still refreshes. Both connections surface their state in the header.
- **State preservation across snapshot polls:** the "last event" column tracks events that have already scrolled off the tail — set by the streamer and preserved across snapshot merges.

### Connection model

The watch command opens two concurrent Unix-socket connections to edgeplaned: one for `agent.supervise.list` polling (closed and reopened each poll cycle), one long-lived for `events.subscribe` streaming. This sidesteps the connection-mode-switch constraint added in v0.13.0 (streaming hijacks the connection — no further JSON-RPC requests on a streaming connection) by using separate connections for the two concerns.

### Internals

- New module `crates/edgeplane/src/agent_supervise_watch.rs` (~500 LOC). Self-contained; reuses no code from `agent_supervise.rs` so the watch surface can evolve without affecting the one-shot CLI verbs.
- Shared state behind `std::sync::Mutex<State>` — locks held briefly for clone-in/clone-out. No async lock needed (no await points under the lock).
- Render loop runs on `tokio::task::spawn_blocking`; crossterm event reads and ratatui draws are sync. Background tokio tasks own the I/O.

### Tests

- `apply_event_updates_agent_row_and_tail` — synthetic event frame updates both the per-agent last-event pointer and the tail.
- `apply_event_trims_tail_to_capacity` — FIFO trim at the configured tail size.
- `merge_snapshot_preserves_last_event` — fresh snapshots overwrite poll-side fields but preserve stream-side `last_event_*`.
- `short_time_extracts_hms` — RFC3339 timestamp → HH:MM:SS rendering.

edgeplane 133/133 (was 129, +4 from this PR), edgeplaned 282/282, edgeplane-tower 48/48 — all pass.

### Try it

```bash
edgeplane agent supervise watch
# In another terminal:
edgeplane agent supervise restart work
# Watch the TUI: the event flows into the tail + updates the "LAST" column for `work`.
```

### Out of scope (next layers)

- **Web-portal consumer.** This is the TUI; a browser dashboard is a separate phase.
- **Server-side event filtering** (filter to one agent). Client-side filtering on `--json` event output still works for now.
- **Replay buffer.** The tail starts empty when the TUI launches. For history, use `edgeplane agent supervise history`.

## [0.13.1] — 2026-05-20

### Fixed — version-sync CI workflow

The version-sync workflow added in 0.11.0 has been silently failing on every push since it landed (duration 0s, "workflow file issue") — GitHub's workflow file parser couldn't handle the triple-nested escapes in the inline python regex. The invariant has been valid throughout (verified manually each release); the workflow was the broken thing. Rewrote the parser in pure awk; no nested escapes, no external interpreter. version-sync now runs to completion (~7s) and reports the per-crate version alongside `/VERSION`.

Patch bump from 0.13.0 with no functional changes — just the workflow fix and the bump itself.

## [0.13.0] — 2026-05-20

### Added — SupervisorEvent streaming via mgmt-gateway

Phase 5 (edgeplaned v0.10.0) added a `tokio::sync::broadcast::Sender<SupervisorEvent>` publisher inside the unit-health loop, with no consumer. Phase B of the 1.0.0 path wires the consumer.

- **New JSON-RPC method `events.subscribe`** on the mgmt-gateway. When invoked, the gateway hijacks the connection: sends one ack frame (`{"jsonrpc":"2.0","id":N,"result":{"subscribed":true}}`), then pushes newline-delimited `SupervisorEvent` JSON frames as they fire on the broadcast channel. The stream terminates on client disconnect, edgeplaned shutdown, or fatal broadcast lag (subscriber too slow — the channel is bounded at 256). On lag, the gateway emits `{"ok":false,"error":"lag","skipped":N}` and closes the connection.
- **New CLI: `edgeplane agent supervise events [--json]`** — opens the edgeplaned Unix socket, subscribes, and prints frames as they arrive. Pretty mode is one human-readable line per event; `--json` passes raw frames through (for piping into `jq`, log shippers, etc).
- **Wire format** is unchanged from what `edgeplaned-core::types::SupervisorEvent` already serialized via serde: tagged-enum JSON with a `kind` discriminator (`unit_dead_detected`, `unit_restarted`, `supervise_paused`, `supervise_resumed`, `nightly_restart_fired`).

### Connection model

`events.subscribe` is the first **streaming** method on the gateway. All previous methods follow strict request → single-response over the same line-delimited JSON-RPC framing. Subscriptions are mode-switched per-connection: once subscribed, no further JSON-RPC requests are accepted on that connection. Open a separate connection for non-streaming calls. This keeps the existing request/response surface untouched.

### Out of scope (next layers)

- **TUI / web-portal consumers.** The streaming surface is now wired; the actual fleet-dashboard rendering on top of it is a separate UX phase.
- **Replay / history.** `agent.supervise.history` already serves the persisted log via the `unit_restart_log` table; the live stream is for "what's happening right now", not "what happened yesterday".
- **Per-agent filtering on the server side.** Subscribers receive all events; client-side filtering with `jq` works for now.

### Tests

- `events_subscribe_streams_event_after_ack` — full duplex test with `tokio::io::duplex`: subscribe, fire a `SupervisorEvent::UnitRestarted` via the broadcast `Sender`, assert ack + event frames arrive in order with the right discriminator.
- `events_subscribe_errors_when_no_sender_wired` — `Option<&Sender>::None` returns JSON-RPC error `-32603` and closes cleanly.
- edgeplaned full suite: 282/282 pass (was 280/280, +2 from this PR).

## [0.12.0] — 2026-05-20

### Changed — Repo layout: `integrations/` → `crates/`

The directory holding edgeplane, edgeplaned, and edgeplane-tower has been renamed:

- `integrations/edgeplane/` → `crates/edgeplane/`
- `integrations/edgeplaned/` → `crates/edgeplaned/`
- `integrations/edgeplane-tower/` → `crates/edgeplane-tower/`

The old name was a holdover from when this repo was Python-first and the Rust binaries were peripheral. They are now the platform; `crates/` is the Rust-idiomatic name and matches the layout most Rust monorepos use.

**No behaviour change.** All HTTP routes — `/integrations/slack/events`, `/integrations/teams/events`, `/integrations/google-chat/events`, and other webhook endpoints — are unchanged. Slack, Teams, Google Chat, and other upstream callers do not need to update their configured URLs.

### Migration

For local clones:

```bash
git pull --rebase   # the rename is a normal commit; no filter-repo
# Update any local scripts that referenced `integrations/...`:
grep -rln "integrations/m" your-scripts/ | xargs sed -i \
  -e 's|integrations/edgeplane-tower|crates/edgeplane-tower|g' \
  -e 's|integrations/edgeplaned|crates/edgeplaned|g' \
  -e 's|integrations/edgeplane\b|crates/edgeplane|g'
```

For CI / external automation: any reference to `integrations/edgeplane`, `integrations/edgeplaned`, or `integrations/edgeplane-tower` (whether as a build context, working directory, or doc path) needs to update to `crates/...`. Webhook URLs do **not** change.

### Internal scope

- `scripts/set-version.sh` paths updated
- `.github/workflows/{version-sync,ci,release-edgeplane,build-image,ci-migrations}.yml` updated
- `docker-compose*.yml` build contexts updated
- ~52 files swept; HTTP route literals (5 files) verified untouched

## [0.11.0] — 2026-05-20

### Removed — Deprecation aliases (Phase 6.5)

Cold removal of two deprecation aliases that have been live since v0.8.0:

- **`edgeplane signal <id>`** (top-level) → use `edgeplane agent signal <id> --content "..."` (auto-resolves local vs controlplane, or pass `--remote` explicitly).
- **`edgeplane agent remote <verb>`** subtree → use `edgeplane agent <verb> --remote`. The verb surfaces are equivalent; `--remote` forces the controlplane path that `remote <verb>` used to imply.

Migration is mechanical; no behaviour change in the replacement commands. Operator scripts and systemd units that called either alias must update.

### Changed — Unified versioning across all three binaries

`edgeplane`, `edgeplaned`, and `edgeplane-tower` now share a single version number:

- `edgeplane-tower` jumps **0.6.0 → 0.11.0** with no functional changes; the catch-up brings all three into lockstep.
- `/VERSION` at repo root is the source of truth.
- `scripts/set-version.sh <new>` updates all three `[workspace.package]` versions atomically.
- New CI job `version-sync` (`.github/workflows/version-sync.yml`) asserts on every PR that `/VERSION` and the three Cargo.toml versions agree. Drift fails the build.

Going forward every release moves all three binaries, even when only one changed; release notes per binary will note "no functional changes" where applicable. Trade: noisier changelogs in exchange for a single number operators reason about.

### Migration

```bash
# Operator scripts: replace the alias forms.
sed -i 's/edgeplane signal /edgeplane agent signal /g' <your-scripts>
sed -i 's/edgeplane agent remote message/edgeplane agent signal --remote/g' <your-scripts>
# (edgeplane agent remote sessions/list/start/end → edgeplane agent <verb> --remote)

# Verify your binaries are on 0.11.0:
edgeplane --version            # 0.11.0
edgeplaned --version           # 0.11.0
edgeplane-tower --version   # 0.11.0
```

### Out of scope (followed up separately)

- Renaming `integrations/` → `crates/` — landed in 0.12.0.

## [0.10.0] — 2026-05-20

### Added — Watchdog absorption

edgeplaned absorbs the systemd-unit liveness loop that used to live in
`aria-watchdog-rs`. After this release, `aria-rs` has zero long-running
daemons — the lanes-decision goal is met.

- **`UnitHealthLoop`** ticks every 60s. For each agent with
  `systemd_service` set in `agent_launch_context`:
  - Run `systemctl --user is-active <service>`.
  - If dead and not in 90s post-restart grace and not throttled
    (30-min default retry window): issue `systemctl --user restart`.
  - Defaults match `aria-watchdog-rs` exactly (60s tick, 1800s retry,
    90s grace, 03:00 nightly).
- **Optional nightly restart at 03:00** — hygiene against memory
  creep. Configurable hour or `None` to disable.
- **Operator pause** — `supervise_paused` column on
  `agent_launch_context`; persists across edgeplaned restarts. Pause survives
  re-import (the fleet importer does not clobber operator state).
  Pause is orthogonal to `restart` — a paused agent stays paused
  after `edgeplane agent supervise restart`; resume separately.
- **`SupervisorEvent` broadcast channel** in `edgeplaned-core` for future
  TUI / web-portal consumers. Variants:
  `UnitDeadDetected | UnitRestarted | SupervisePaused | SuperviseResumed | NightlyRestartFired`.
  Phase 5 ships the publisher; subscription via mgmt-gateway streaming
  is a follow-up.

### Added — `edgeplane agent supervise` CLI

- `edgeplane agent supervise list [--json]`
- `edgeplane agent supervise status <id> [--limit N] [--json]`
- `edgeplane agent supervise restart <id>` — logged as `reason=manual`
- `edgeplane agent supervise pause [<id>] [--all]`
- `edgeplane agent supervise resume [<id>] [--all]`
- `edgeplane agent supervise history [--agent-id <id>] [-n N] [--json]`

### Added — Versioned SQLite migrations

Refactored `LocalRegistry::migrate(conn)` into a forward-only
migration framework (`apply_migrations` walks from the stamped
`schema_version` to `CURRENT_SCHEMA_VERSION`). Adding a new
migration is "increment the constant, write `migrate_to_vN`, wire
it into the walker." Each step runs in a transaction.

- **v1** — Phase 1 + 4 baseline (agent, agent_launch_context,
  agent_cron_*).
- **v2** — Phase 5: `agent_launch_context` gains `systemd_service` +
  `supervise_paused` columns; new `unit_restart_log` table.

`add_column_if_missing` helper guards `ALTER TABLE ADD COLUMN` via
`PRAGMA table_info` so migrations are idempotent on re-run.

### Migration

For nodes running the Aria fleet:

```bash
# Phase 5 edgeplaned absorbs systemd unit liveness; aria-watchdog-rs is dead code.
systemctl --user restart edgeplaned
systemctl --user disable --now aria-watchdog-rs.service
edgeplane agent supervise list      # verify all 6 fleet agents show "active"
```

### Deprecated

- `aria watchdog` + `aria-watchdog-rs.service` — fully superseded by
  edgeplaned's `unit_health` loop. Full removal in Phase 6.

### Out of scope (Phase 6)

- aria-rs `aria fleet` + `aria cron` source removal (Phase 6)
- aria-watchdog-rs source removal (Phase 6)
- Home Assistant notifications from aria-watchdog (operator-specific;
  optional follow-up if needed — broadcast channel exists)
- Streaming subscription to `SupervisorEvent` via mgmt-gateway
  (broadcast publisher exists; consumer wiring is a follow-up)

## [0.9.0] — 2026-05-20

### Added — Phase 4 daemon-absorption (PR #31)

edgeplaned absorbs `aria-cron.toml` + the `aria cron` dispatcher. After this release, `~/.ep/edgeplaned/cron.toml` is the canonical cron config; edgeplaned runs its own 1-minute tick loop, dispatches via `runtime.signal`, and stores every fire in SQLite with bounded retention.

- **File-as-config:** edgeplaned reads `~/.ep/edgeplaned/cron.toml` on startup + on `edgeplane agent cron reload`. File schema is byte-compatible with `aria-cron.toml` (add `schema_version = 1` at top; migration is `cp`). Schema_version forwards-compat: files newer than `MCD_SUPPORTED_CRON_SCHEMA` are refused with a clear "upgrade edgeplaned" message.
- **New `edgeplane agent cron` CLI** (read + inspect only — file edits go through `$EDITOR`):
  - `edgeplane agent cron list` — all jobs from file + last-fire status
  - `edgeplane agent cron describe <name> [--limit N]` — one job + recent history
  - `edgeplane agent cron reload` — poke edgeplaned to re-parse the file
  - `edgeplane agent cron history [--name N] [-n N]` — recent fires across all (or one) job
  - `edgeplane agent cron gc-now [--history-days N] [--max-rows-per-job N]` — force a retention sweep
- **New mgmt-gateway methods** (edgeplaned-side): `agent.cron.list / describe / reload / history / gc_now`
- **New SQLite tables (telemetry only):**
  - `agent_cron_state` — latest state per job (last_fired_at, last_status, last_error)
  - `agent_cron_fire_log` — append-only fire history, GC'd per retention policy
- **Configurable retention** in `[retention]` section of `cron.toml`:
  - `history_days = 30` — drop log rows older than this (0 = keep forever)
  - `max_rows_per_job = 500` — per-job cap regardless of age
  - `gc_interval_minutes = 60` — background GC task cadence
- **Resilient reload:** parse errors during reload keep the previously-loaded config in memory; edgeplaned logs loudly and waits for a fix.
- **Recovery semantics:** missed iterations during daemon downtime fire once on recovery (not N times for N missed iterations).

### Migration

```bash
cp ~/code/aria/aria-cron.toml ~/.ep/edgeplaned/cron.toml
# Add `schema_version = 1` at the top of the file
systemctl --user restart edgeplaned
edgeplane agent cron list                                # verify
systemctl --user disable --now aria-cron.timer    # when satisfied
```

### Deprecated

- `aria-cron.toml` (in aria-rs) — superseded by `~/.ep/edgeplaned/cron.toml`. `aria cron` dispatcher and `aria-cron.timer` operationally dead; full removal in Phase 6 (the aria-rs deprecation phase).

### Out of scope (Phase 5+)

- aria-watchdog-rs absorption (Phase 5)
- aria-rs `aria fleet` + `aria cron` removal (Phase 6)
- `kind = "heartbeat"` schedules (none currently used)
- `dispatch = "goose"` execution (none currently used)
- Inotify auto-reload (explicit `edgeplane agent cron reload` works fine)
- CLI-driven cron.toml mutations (operator edits the file directly)

## [0.8.0] — 2026-05-19

### Added — Phase 1–3 daemon-absorption (PRs #25, #28, #29)

The Aria fleet (operator/research/work/merlinlabs/mc-engineer/publisher profiles) is now first-class addressable through `edgeplane agent`. This is the first release where `edgeplane agent signal work --content "..."` lands a prompt in the work profile's Zellij pane.

- **New `edgeplane agent` verb-first surface** with local-or-controlplane auto-resolve:
  - `edgeplane agent signal <id> --content "..."` — send a prompt
  - `edgeplane agent cancel <id>` — interrupt (Ctrl c for ZellijHosted)
  - `edgeplane agent list [--source local|remote|all] [--json]`
  - `edgeplane agent describe <id> [--json]`
  - `edgeplane agent attach <id> [--web]` — dispatches on runtime kind; ZellijHosted execs `zellij attach`, with `--web` prints the `zellij web` URL
  - `--local` / `--remote` flags on each command for explicit override
- **New edgeplaned runtime: `RuntimeKind::ZellijHosted`** — long-running agents hosted in a Zellij pane. Externally managed (systemd + aria-watchdog own the session lifecycle); edgeplaned addresses panes via `zellij action` subprocess calls. Per-agent mutex serialises concurrent signals.
- **New mgmt-gateway JSON-RPC methods** — `agent.local.signal`, `agent.local.list`, `agent.describe_local`. Used by edgeplane's auto-resolve to find local agents before falling through to controlplane.
- **New SQLite table `agent_launch_context`** — joined 1:1 to `agent` by composite FK with `ON DELETE CASCADE`. Carries declarative launch params (`vault_folder`, `state_dir_spec`, `zellij_session`) populated by the fleet importer.
- **New types in edgeplaned-core** — `StateDirSpec` enum (Persistent / Ephemeral with TTL) for declarative work-dir lifecycle; `LaunchContext.zellij_session`, `LaunchContext.vault_folder`, `LaunchContext.state_dir_spec` fields.
- **New `edgeplaned doctor` subcommand** — read-only health check (lock state, port reachability, registry, runtime resolution). Safe to run while the daemon is misbehaving (PR #24).
- **Kernel-enforced singleton via flock(2)** — `~/.ep/edgeplaned/edgeplaned.lock`. Operator escape hatches: `--kill-existing`, `--allow-degraded` (PR #24).

### Added — Fleet profile importer

- Aria-style `fleet-profiles.toml` is read at edgeplaned startup and each `[[profile]]` is upserted as a `ZellijHosted` agent with `source = "fleet_import"`. Idempotent: re-runs upsert in place. Default path `~/code/aria/fleet-profiles.toml`; override via `MCD_FLEET_PROFILES_FILE` env var or `DaemonConfig.fleet_profiles_file`.

### Added — CI quality gates

- `cargo nextest` adopted across all three workspaces (PR #26). edgeplaned CI now runs `cargo nextest run --workspace` instead of `cargo check`, closing a gap where test-compile failures (state.rs `Option<String>`, `SingletonLock` Debug derive) shipped without being caught.

### Added — Release workflow improvements

- `actions/upload-artifact` v4→v7, `actions/download-artifact` v4→v8 (PR #27).
- `Swatinem/rust-cache@v2` replaces the bare `actions/cache@v5` for Cargo caching; ~2–4× faster matrix builds on cache hits.
- `taiki-e/install-action@v2` for `cross` install caching (~30s saved per cross-target build).
- **Bug fix:** `build-extras` artifacts (Windows, macOS x86_64 — `workflow_dispatch + include_extras=true`) now actually make it into the GitHub release. Previously they were uploaded as run artifacts but never attached.

### Changed

- **Auth: OIDC-only** (PR #21). The `EP_TOKEN` env auth path is removed; OIDC is the only flow. Session TTL cap raised from 30 days to 1 year.
- **`actions/checkout`-style action versions bumped** in `build-image.yml` (PRs #16, #17): `docker/metadata-action` 5→6, `docker/setup-buildx-action` 3→4.

### Deprecated

- `edgeplane signal <id>` (top-level) — alias for `edgeplane agent signal --remote`. Removed in a future cleanup; use `edgeplane agent signal` instead.
- `edgeplane agent remote …` — controlplane-only verbs. Use `edgeplane agent <verb>` with optional `--remote` instead.

### Migration notes

- After upgrading edgeplaned to 0.8.0 on a node that runs the Aria fleet, the next edgeplaned start will:
  1. Open the local SQLite registry (existing — no schema migration risk; `CREATE TABLE IF NOT EXISTS` is additive)
  2. Import 6 fleet profiles into `agent_launch_context` (idempotent)
  3. Resolve them through the new `ZellijHostedRuntime`; `launch()` warns (not errors) if a profile's Zellij session is not currently running
- No action required for nodes that don't run the Aria fleet — the importer skips when no manifest is found.
- Existing ACP agents (operator/publisher/research/merlinlabs/work/mc-engineer as `aria-*-<hex>` public_ids) continue routing via webhook relay; no behaviour change.

## [0.7.0] and earlier

Pre-changelog. See git history.
