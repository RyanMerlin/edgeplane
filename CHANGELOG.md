# Changelog

All notable changes to mc + mcd are recorded here. mc-controlplane has its own version cadence (currently 0.6.0).

This project follows semantic versioning where possible, but pre-1.0 minor bumps may include breaking changes when the cost of a major bump outweighs the signal value.

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
