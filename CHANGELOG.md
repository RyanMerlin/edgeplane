# Changelog

All notable changes to mc, mcd, and mc-controlplane are recorded here. Starting with 0.11.0 the three binaries ship in lockstep — `/VERSION` is the source of truth and `scripts/set-version.sh <new>` bumps all three in one step.

This project follows semantic versioning where possible, but pre-1.0 minor bumps may include breaking changes when the cost of a major bump outweighs the signal value.

## [0.15.2] — 2026-05-20

### Fixed

- **CI: `build-image.yml` was bumping the gitops manifest to the wrong tag.** `docker/metadata-action` with `type=sha` publishes images as `:sha-<short>` (default prefix), but the workflow's gitops `sed` was rewriting the image to `:<short>` (no prefix). Every recent CI-driven deployment landed on an image that didn't exist (ImagePullBackOff). The fix matches the actual published tag.

### Added

- **Cron eval regression tests** for two scenarios that we'd flagged as suspected bugs on 2026-05-20 but turned out to be uptime / config-edit artifacts (no eval bug). Tests guard against the same symptom shape reappearing:
  - `daily_9am_with_recent_afternoon_anchor_is_not_due`
  - `daily_530am_with_prior_afternoon_anchor_is_due_next_morning`

  See `mc-engineer/projects/2026-05-20-...-mcd-cron-eval-bugs-...md` (vault) for the diagnosis trail.

## [0.15.1] — 2026-05-20

### Fixed

- **`mcd.service` unit now sets `Environment=PATH=`** explicitly, so `dispatch = "goose"` cron jobs can find the `aria` binary (and any other tool installed under `~/.cargo/bin` or `~/.local/bin`). systemd user services get a minimal PATH by default, which was breaking goose-dispatch fires silently. `crates/mcd/systemd/mcd.service`.

### Changed

- **Heartbeat cron jobs no longer require a `schedule = ""` field.** `CronJob::schedule` is now `#[serde(default)]`; heartbeat jobs can omit the field entirely. Cron jobs explicitly reject empty `schedule` with a clear error message. Existing files with `schedule = ""` on heartbeat jobs continue to parse unchanged. `crates/mcd/.../cron_config.rs`.
- **`mc agent cron list` and `describe` now render heartbeat cadence.** Heartbeat jobs show `heartbeat: 30m` in the SCHEDULE column instead of empty; describe surfaces a dedicated `kind:` line and switches between `schedule:` and `interval:` accordingly. The `agent.cron.list` / `agent.cron.describe` JSON-RPC responses now include `kind` and `interval`. `crates/mc/src/agent_cron.rs`, `crates/mcd/.../mgmt_gateway.rs`.

## [0.15.0] — 2026-05-20

### Added — Cron heartbeat tier (`kind = "heartbeat"`)

`~/.mc/mcd/cron.toml` now accepts heartbeat-style jobs that fire on a duration cadence instead of a cron expression. Useful for periodic polls where exact clock alignment doesn't matter (queue sweeps, commitment checks, drift detection).

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
- Failure surfaces in `mc agent cron history` with the actual goose stderr.

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

mc 133/133, **mcd 285/285** (+3 from this PR), mc-controlplane 48/48 — all pass.

### Migration

None required — pure additive change. Existing `cron.toml` files with `kind = "cron"` and `dispatch = "signal"` work unchanged.

## [0.14.0] — 2026-05-20

### Added — Live fleet dashboard (`mc agent supervise watch`)

Phase B (v0.13.0) wired `events.subscribe` to a streaming consumer protocol; this release adds the first interactive consumer that actually renders the stream.

- **`mc agent supervise watch [--poll-secs N] [--tail-size N]`** — ratatui TUI with:
  - **Top pane:** agent table (one row per supervised agent), polled from `agent.supervise.list` every `--poll-secs` (default 5s). Columns: AGENT, SYSTEMD, STATE (color-coded green/red/yellow by unit_state), PAUSED, LAST (most recent event timestamp + label).
  - **Bottom pane:** scrolling event tail, fed by `events.subscribe`. Newest at the bottom. Color-coded by kind (red=dead, yellow=restart, cyan=pause/resume, magenta=nightly).
  - **Header:** stream status indicator (connecting / live / closed / error) + snapshot age.
  - **Footer:** keybinds + agent/event counts.
- **Resilience:** the streamer reconnects automatically if mcd hangs up. Snapshot poller continues independently of stream state, so even if streaming breaks the table still refreshes. Both connections surface their state in the header.
- **State preservation across snapshot polls:** the "last event" column tracks events that have already scrolled off the tail — set by the streamer and preserved across snapshot merges.

### Connection model

The watch command opens two concurrent Unix-socket connections to mcd: one for `agent.supervise.list` polling (closed and reopened each poll cycle), one long-lived for `events.subscribe` streaming. This sidesteps the connection-mode-switch constraint added in v0.13.0 (streaming hijacks the connection — no further JSON-RPC requests on a streaming connection) by using separate connections for the two concerns.

### Internals

- New module `crates/mc/src/agent_supervise_watch.rs` (~500 LOC). Self-contained; reuses no code from `agent_supervise.rs` so the watch surface can evolve without affecting the one-shot CLI verbs.
- Shared state behind `std::sync::Mutex<State>` — locks held briefly for clone-in/clone-out. No async lock needed (no await points under the lock).
- Render loop runs on `tokio::task::spawn_blocking`; crossterm event reads and ratatui draws are sync. Background tokio tasks own the I/O.

### Tests

- `apply_event_updates_agent_row_and_tail` — synthetic event frame updates both the per-agent last-event pointer and the tail.
- `apply_event_trims_tail_to_capacity` — FIFO trim at the configured tail size.
- `merge_snapshot_preserves_last_event` — fresh snapshots overwrite poll-side fields but preserve stream-side `last_event_*`.
- `short_time_extracts_hms` — RFC3339 timestamp → HH:MM:SS rendering.

mc 133/133 (was 129, +4 from this PR), mcd 282/282, mc-controlplane 48/48 — all pass.

### Try it

```bash
mc agent supervise watch
# In another terminal:
mc agent supervise restart work
# Watch the TUI: the event flows into the tail + updates the "LAST" column for `work`.
```

### Out of scope (next layers)

- **Web-portal consumer.** This is the TUI; a browser dashboard is a separate phase.
- **Server-side event filtering** (filter to one agent). Client-side filtering on `--json` event output still works for now.
- **Replay buffer.** The tail starts empty when the TUI launches. For history, use `mc agent supervise history`.

## [0.13.1] — 2026-05-20

### Fixed — version-sync CI workflow

The version-sync workflow added in 0.11.0 has been silently failing on every push since it landed (duration 0s, "workflow file issue") — GitHub's workflow file parser couldn't handle the triple-nested escapes in the inline python regex. The invariant has been valid throughout (verified manually each release); the workflow was the broken thing. Rewrote the parser in pure awk; no nested escapes, no external interpreter. version-sync now runs to completion (~7s) and reports the per-crate version alongside `/VERSION`.

Patch bump from 0.13.0 with no functional changes — just the workflow fix and the bump itself.

## [0.13.0] — 2026-05-20

### Added — SupervisorEvent streaming via mgmt-gateway

Phase 5 (mcd v0.10.0) added a `tokio::sync::broadcast::Sender<SupervisorEvent>` publisher inside the unit-health loop, with no consumer. Phase B of the 1.0.0 path wires the consumer.

- **New JSON-RPC method `events.subscribe`** on the mgmt-gateway. When invoked, the gateway hijacks the connection: sends one ack frame (`{"jsonrpc":"2.0","id":N,"result":{"subscribed":true}}`), then pushes newline-delimited `SupervisorEvent` JSON frames as they fire on the broadcast channel. The stream terminates on client disconnect, mcd shutdown, or fatal broadcast lag (subscriber too slow — the channel is bounded at 256). On lag, the gateway emits `{"ok":false,"error":"lag","skipped":N}` and closes the connection.
- **New CLI: `mc agent supervise events [--json]`** — opens the mcd Unix socket, subscribes, and prints frames as they arrive. Pretty mode is one human-readable line per event; `--json` passes raw frames through (for piping into `jq`, log shippers, etc).
- **Wire format** is unchanged from what `mcd-core::types::SupervisorEvent` already serialized via serde: tagged-enum JSON with a `kind` discriminator (`unit_dead_detected`, `unit_restarted`, `supervise_paused`, `supervise_resumed`, `nightly_restart_fired`).

### Connection model

`events.subscribe` is the first **streaming** method on the gateway. All previous methods follow strict request → single-response over the same line-delimited JSON-RPC framing. Subscriptions are mode-switched per-connection: once subscribed, no further JSON-RPC requests are accepted on that connection. Open a separate connection for non-streaming calls. This keeps the existing request/response surface untouched.

### Out of scope (next layers)

- **TUI / web-portal consumers.** The streaming surface is now wired; the actual fleet-dashboard rendering on top of it is a separate UX phase.
- **Replay / history.** `agent.supervise.history` already serves the persisted log via the `unit_restart_log` table; the live stream is for "what's happening right now", not "what happened yesterday".
- **Per-agent filtering on the server side.** Subscribers receive all events; client-side filtering with `jq` works for now.

### Tests

- `events_subscribe_streams_event_after_ack` — full duplex test with `tokio::io::duplex`: subscribe, fire a `SupervisorEvent::UnitRestarted` via the broadcast `Sender`, assert ack + event frames arrive in order with the right discriminator.
- `events_subscribe_errors_when_no_sender_wired` — `Option<&Sender>::None` returns JSON-RPC error `-32603` and closes cleanly.
- mcd full suite: 282/282 pass (was 280/280, +2 from this PR).

## [0.12.0] — 2026-05-20

### Changed — Repo layout: `integrations/` → `crates/`

The directory holding mc, mcd, and mc-controlplane has been renamed:

- `integrations/mc/` → `crates/mc/`
- `integrations/mcd/` → `crates/mcd/`
- `integrations/mc-controlplane/` → `crates/mc-controlplane/`

The old name was a holdover from when this repo was Python-first and the Rust binaries were peripheral. They are now the platform; `crates/` is the Rust-idiomatic name and matches the layout most Rust monorepos use.

**No behaviour change.** All HTTP routes — `/integrations/slack/events`, `/integrations/teams/events`, `/integrations/google-chat/events`, and other webhook endpoints — are unchanged. Slack, Teams, Google Chat, and other upstream callers do not need to update their configured URLs.

### Migration

For local clones:

```bash
git pull --rebase   # the rename is a normal commit; no filter-repo
# Update any local scripts that referenced `integrations/...`:
grep -rln "integrations/m" your-scripts/ | xargs sed -i \
  -e 's|integrations/mc-controlplane|crates/mc-controlplane|g' \
  -e 's|integrations/mcd|crates/mcd|g' \
  -e 's|integrations/mc\b|crates/mc|g'
```

For CI / external automation: any reference to `integrations/mc`, `integrations/mcd`, or `integrations/mc-controlplane` (whether as a build context, working directory, or doc path) needs to update to `crates/...`. Webhook URLs do **not** change.

### Internal scope

- `scripts/set-version.sh` paths updated
- `.github/workflows/{version-sync,ci,release-mc,build-image,ci-migrations}.yml` updated
- `docker-compose*.yml` build contexts updated
- ~52 files swept; HTTP route literals (5 files) verified untouched

## [0.11.0] — 2026-05-20

### Removed — Deprecation aliases (Phase 6.5)

Cold removal of two deprecation aliases that have been live since v0.8.0:

- **`mc signal <id>`** (top-level) → use `mc agent signal <id> --content "..."` (auto-resolves local vs controlplane, or pass `--remote` explicitly).
- **`mc agent remote <verb>`** subtree → use `mc agent <verb> --remote`. The verb surfaces are equivalent; `--remote` forces the controlplane path that `remote <verb>` used to imply.

Migration is mechanical; no behaviour change in the replacement commands. Operator scripts and systemd units that called either alias must update.

### Changed — Unified versioning across all three binaries

`mc`, `mcd`, and `mc-controlplane` now share a single version number:

- `mc-controlplane` jumps **0.6.0 → 0.11.0** with no functional changes; the catch-up brings all three into lockstep.
- `/VERSION` at repo root is the source of truth.
- `scripts/set-version.sh <new>` updates all three `[workspace.package]` versions atomically.
- New CI job `version-sync` (`.github/workflows/version-sync.yml`) asserts on every PR that `/VERSION` and the three Cargo.toml versions agree. Drift fails the build.

Going forward every release moves all three binaries, even when only one changed; release notes per binary will note "no functional changes" where applicable. Trade: noisier changelogs in exchange for a single number operators reason about.

### Migration

```bash
# Operator scripts: replace the alias forms.
sed -i 's/mc signal /mc agent signal /g' <your-scripts>
sed -i 's/mc agent remote message/mc agent signal --remote/g' <your-scripts>
# (mc agent remote sessions/list/start/end → mc agent <verb> --remote)

# Verify your binaries are on 0.11.0:
mc --version            # 0.11.0
mcd --version           # 0.11.0
mc-controlplane --version   # 0.11.0
```

### Out of scope (followed up separately)

- Renaming `integrations/` → `crates/` — landed in 0.12.0.

## [0.10.0] — 2026-05-20

### Added — Watchdog absorption

mcd absorbs the systemd-unit liveness loop that used to live in
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
  `agent_launch_context`; persists across mcd restarts. Pause survives
  re-import (the fleet importer does not clobber operator state).
  Pause is orthogonal to `restart` — a paused agent stays paused
  after `mc agent supervise restart`; resume separately.
- **`SupervisorEvent` broadcast channel** in `mcd-core` for future
  TUI / web-portal consumers. Variants:
  `UnitDeadDetected | UnitRestarted | SupervisePaused | SuperviseResumed | NightlyRestartFired`.
  Phase 5 ships the publisher; subscription via mgmt-gateway streaming
  is a follow-up.

### Added — `mc agent supervise` CLI

- `mc agent supervise list [--json]`
- `mc agent supervise status <id> [--limit N] [--json]`
- `mc agent supervise restart <id>` — logged as `reason=manual`
- `mc agent supervise pause [<id>] [--all]`
- `mc agent supervise resume [<id>] [--all]`
- `mc agent supervise history [--agent-id <id>] [-n N] [--json]`

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
# Phase 5 mcd absorbs systemd unit liveness; aria-watchdog-rs is dead code.
systemctl --user restart mcd
systemctl --user disable --now aria-watchdog-rs.service
mc agent supervise list      # verify all 6 fleet agents show "active"
```

### Deprecated

- `aria watchdog` + `aria-watchdog-rs.service` — fully superseded by
  mcd's `unit_health` loop. Full removal in Phase 6.

### Out of scope (Phase 6)

- aria-rs `aria fleet` + `aria cron` source removal (Phase 6)
- aria-watchdog-rs source removal (Phase 6)
- Home Assistant notifications from aria-watchdog (operator-specific;
  optional follow-up if needed — broadcast channel exists)
- Streaming subscription to `SupervisorEvent` via mgmt-gateway
  (broadcast publisher exists; consumer wiring is a follow-up)

## [0.9.0] — 2026-05-20

### Added — Phase 4 daemon-absorption (PR #31)

mcd absorbs `aria-cron.toml` + the `aria cron` dispatcher. After this release, `~/.mc/mcd/cron.toml` is the canonical cron config; mcd runs its own 1-minute tick loop, dispatches via `runtime.signal`, and stores every fire in SQLite with bounded retention.

- **File-as-config:** mcd reads `~/.mc/mcd/cron.toml` on startup + on `mc agent cron reload`. File schema is byte-compatible with `aria-cron.toml` (add `schema_version = 1` at top; migration is `cp`). Schema_version forwards-compat: files newer than `MCD_SUPPORTED_CRON_SCHEMA` are refused with a clear "upgrade mcd" message.
- **New `mc agent cron` CLI** (read + inspect only — file edits go through `$EDITOR`):
  - `mc agent cron list` — all jobs from file + last-fire status
  - `mc agent cron describe <name> [--limit N]` — one job + recent history
  - `mc agent cron reload` — poke mcd to re-parse the file
  - `mc agent cron history [--name N] [-n N]` — recent fires across all (or one) job
  - `mc agent cron gc-now [--history-days N] [--max-rows-per-job N]` — force a retention sweep
- **New mgmt-gateway methods** (mcd-side): `agent.cron.list / describe / reload / history / gc_now`
- **New SQLite tables (telemetry only):**
  - `agent_cron_state` — latest state per job (last_fired_at, last_status, last_error)
  - `agent_cron_fire_log` — append-only fire history, GC'd per retention policy
- **Configurable retention** in `[retention]` section of `cron.toml`:
  - `history_days = 30` — drop log rows older than this (0 = keep forever)
  - `max_rows_per_job = 500` — per-job cap regardless of age
  - `gc_interval_minutes = 60` — background GC task cadence
- **Resilient reload:** parse errors during reload keep the previously-loaded config in memory; mcd logs loudly and waits for a fix.
- **Recovery semantics:** missed iterations during daemon downtime fire once on recovery (not N times for N missed iterations).

### Migration

```bash
cp ~/code/aria/aria-cron.toml ~/.mc/mcd/cron.toml
# Add `schema_version = 1` at the top of the file
systemctl --user restart mcd
mc agent cron list                                # verify
systemctl --user disable --now aria-cron.timer    # when satisfied
```

### Deprecated

- `aria-cron.toml` (in aria-rs) — superseded by `~/.mc/mcd/cron.toml`. `aria cron` dispatcher and `aria-cron.timer` operationally dead; full removal in Phase 6 (the aria-rs deprecation phase).

### Out of scope (Phase 5+)

- aria-watchdog-rs absorption (Phase 5)
- aria-rs `aria fleet` + `aria cron` removal (Phase 6)
- `kind = "heartbeat"` schedules (none currently used)
- `dispatch = "goose"` execution (none currently used)
- Inotify auto-reload (explicit `mc agent cron reload` works fine)
- CLI-driven cron.toml mutations (operator edits the file directly)

## [0.8.0] — 2026-05-19

### Added — Phase 1–3 daemon-absorption (PRs #25, #28, #29)

The Aria fleet (operator/research/work/merlinlabs/mc-engineer/publisher profiles) is now first-class addressable through `mc agent`. This is the first release where `mc agent signal work --content "..."` lands a prompt in the work profile's Zellij pane.

- **New `mc agent` verb-first surface** with local-or-controlplane auto-resolve:
  - `mc agent signal <id> --content "..."` — send a prompt
  - `mc agent cancel <id>` — interrupt (Ctrl c for ZellijHosted)
  - `mc agent list [--source local|remote|all] [--json]`
  - `mc agent describe <id> [--json]`
  - `mc agent attach <id> [--web]` — dispatches on runtime kind; ZellijHosted execs `zellij attach`, with `--web` prints the `zellij web` URL
  - `--local` / `--remote` flags on each command for explicit override
- **New mcd runtime: `RuntimeKind::ZellijHosted`** — long-running agents hosted in a Zellij pane. Externally managed (systemd + aria-watchdog own the session lifecycle); mcd addresses panes via `zellij action` subprocess calls. Per-agent mutex serialises concurrent signals.
- **New mgmt-gateway JSON-RPC methods** — `agent.local.signal`, `agent.local.list`, `agent.describe_local`. Used by mc's auto-resolve to find local agents before falling through to controlplane.
- **New SQLite table `agent_launch_context`** — joined 1:1 to `agent` by composite FK with `ON DELETE CASCADE`. Carries declarative launch params (`vault_folder`, `state_dir_spec`, `zellij_session`) populated by the fleet importer.
- **New types in mcd-core** — `StateDirSpec` enum (Persistent / Ephemeral with TTL) for declarative work-dir lifecycle; `LaunchContext.zellij_session`, `LaunchContext.vault_folder`, `LaunchContext.state_dir_spec` fields.
- **New `mcd doctor` subcommand** — read-only health check (lock state, port reachability, registry, runtime resolution). Safe to run while the daemon is misbehaving (PR #24).
- **Kernel-enforced singleton via flock(2)** — `~/.mc/mcd/mcd.lock`. Operator escape hatches: `--kill-existing`, `--allow-degraded` (PR #24).

### Added — Fleet profile importer

- Aria-style `fleet-profiles.toml` is read at mcd startup and each `[[profile]]` is upserted as a `ZellijHosted` agent with `source = "fleet_import"`. Idempotent: re-runs upsert in place. Default path `~/code/aria/fleet-profiles.toml`; override via `MCD_FLEET_PROFILES_FILE` env var or `DaemonConfig.fleet_profiles_file`.

### Added — CI quality gates

- `cargo nextest` adopted across all three workspaces (PR #26). mcd CI now runs `cargo nextest run --workspace` instead of `cargo check`, closing a gap where test-compile failures (state.rs `Option<String>`, `SingletonLock` Debug derive) shipped without being caught.

### Added — Release workflow improvements

- `actions/upload-artifact` v4→v7, `actions/download-artifact` v4→v8 (PR #27).
- `Swatinem/rust-cache@v2` replaces the bare `actions/cache@v5` for Cargo caching; ~2–4× faster matrix builds on cache hits.
- `taiki-e/install-action@v2` for `cross` install caching (~30s saved per cross-target build).
- **Bug fix:** `build-extras` artifacts (Windows, macOS x86_64 — `workflow_dispatch + include_extras=true`) now actually make it into the GitHub release. Previously they were uploaded as run artifacts but never attached.

### Changed

- **Auth: OIDC-only** (PR #21). The `MC_TOKEN` env auth path is removed; OIDC is the only flow. Session TTL cap raised from 30 days to 1 year.
- **`actions/checkout`-style action versions bumped** in `build-image.yml` (PRs #16, #17): `docker/metadata-action` 5→6, `docker/setup-buildx-action` 3→4.

### Deprecated

- `mc signal <id>` (top-level) — alias for `mc agent signal --remote`. Removed in a future cleanup; use `mc agent signal` instead.
- `mc agent remote …` — controlplane-only verbs. Use `mc agent <verb>` with optional `--remote` instead.

### Migration notes

- After upgrading mcd to 0.8.0 on a node that runs the Aria fleet, the next mcd start will:
  1. Open the local SQLite registry (existing — no schema migration risk; `CREATE TABLE IF NOT EXISTS` is additive)
  2. Import 6 fleet profiles into `agent_launch_context` (idempotent)
  3. Resolve them through the new `ZellijHostedRuntime`; `launch()` warns (not errors) if a profile's Zellij session is not currently running
- No action required for nodes that don't run the Aria fleet — the importer skips when no manifest is found.
- Existing ACP agents (operator/publisher/research/merlinlabs/work/mc-engineer as `aria-*-<hex>` public_ids) continue routing via webhook relay; no behaviour change.

## [0.7.0] and earlier

Pre-changelog. See git history.
