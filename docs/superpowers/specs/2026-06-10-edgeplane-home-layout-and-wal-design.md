# EdgePlane Home Layout + SQLite WAL Fix — Design

**Date:** 2026-06-10
**Status:** Approved (design), pending implementation plan
**Scope:** `crates/edgeplane` (CLI) + `crates/edgeplaned/*` (daemon). No tower/Postgres changes.

## Problem

Two coupled problems in the local on-disk footprint under `~/.edgeplane/`.

**1. Unbounded SQLite WAL.** `registry.db` is 224 KB but its WAL is 4.1 MB and growing. All three SQLite open sites set `journal_mode=WAL` and nothing else relevant — no `journal_size_limit`, no `busy_timeout`, no explicit checkpoint. `registry.db` has two concurrent writers (the daemon and the standalone CLI), the daemon holds a persistent connection, and the default *passive* auto-checkpoint cannot fully drain the WAL past that persistent connection. With no `journal_size_limit`, the file never truncates back down. It high-water-marks near the ~4 MB autocheckpoint threshold and sticks there.

**2. Process-based, commingled layout.** Files are split by *who writes them* (`~/.edgeplane/` root = CLI, `~/.edgeplane/edgeplaned/` = daemon) rather than *what they are*. Durable hand-edited config (`cron.toml`, `config.yaml`), churny machine-local state (`registry.db`, `receipts.db`, `state.json`), sensitive auth (`session.json`), and ephemeral runtime (sockets, lock) are interleaved across both levels. There is no clean "back up this, ignore that" boundary, and two stale scratch files (`hn.html`, `hn-news.html`, 175 KB each) sit in the daemon dir.

## Goals

- Bound the SQLite WAL files; reclaim disk on idle.
- Re-lay the home directory by **function**: `config/` (durable), `state/` (machine-local), `run/` (ephemeral), `work/` (workspaces).
- Dissolve the `edgeplaned/` process namespace entirely (one node manager; namespacing by it is redundant).
- Zero breakage for the live 5-service fleet and documented hand-edit paths during the transition.

## Non-goals

- No XDG Base Directory split (config/state/runtime stay under a single `~/.edgeplane/` home, matching how Claude Code `~/.claude` and Codex `~/.codex` lay out). Rejected in brainstorming.
- No change to `EP_HOME` semantics — it still names the single home root.
- No schema or data-format changes to either SQLite DB.

---

## Part A — SQLite WAL checkpoint fix

### A.1 Standardize the pragma block

A single documented pragma set, applied at every connection open:

```sql
PRAGMA journal_mode       = WAL;
PRAGMA synchronous        = NORMAL;
PRAGMA busy_timeout       = 5000;        -- 5s: kills SQLITE_BUSY races between the two registry writers
PRAGMA wal_autocheckpoint = 1000;        -- explicit (was the implicit default)
PRAGMA journal_size_limit = 8388608;     -- 8 MB: WAL truncates back to this ceiling after a checkpoint
PRAGMA foreign_keys       = ON;          -- registry.db only (already set there)
```

Applied at the three open sites:

| Site | File | Notes |
|------|------|-------|
| Daemon registry writer | `crates/edgeplaned/crates/edgeplaned-bin/src/local_registry.rs:206` (`open`) and the second `open` near line 432 | currently sets only `journal_mode`, `foreign_keys` |
| CLI standalone registry r/w | `crates/edgeplane/src/local_db.rs:46` (`open`) | currently sets only `journal_mode` |
| Receipts store | `crates/edgeplaned/crates/edgeplaned-receipts/src/store.rs:26` (`open`) | currently sets `journal_mode`, `synchronous` |

**Home for the constant.** Prefer a single tiny helper (e.g. `fn tune_sqlite(conn: &Connection) -> rusqlite::Result<()>`) so the pragma set lives in one place. `edgeplaned-receipts` is its own leaf crate; the helper goes in the lowest crate all three can reach, or is duplicated as one documented `const PRAGMAS: &str` with a test asserting the three copies are byte-identical. Decide the exact home in the implementation plan after confirming the crate dependency graph (`edgeplaned-receipts` must not gain a heavy dep just to share five pragmas).

### A.2 Active checkpoint on the daemon

`journal_size_limit` caps the file but does not by itself reclaim it while a persistent connection is open. The daemon must actively drain:

- **Periodic:** run `PRAGMA wal_checkpoint(TRUNCATE)` on the daemon's existing ~60s reconcile tick.
- **Shutdown:** run `PRAGMA wal_checkpoint(TRUNCATE)` on graceful shutdown (SIGTERM handler).

`TRUNCATE` checkpoints force-drain and shrink the file even with the persistent connection held, which `journal_size_limit` + passive auto-checkpoint alone cannot guarantee. Net effect: `registry.db-wal` stays ≤ 8 MB and returns toward zero on idle.

### A.3 Concurrent-writer note

The two-writer arrangement on `registry.db` (daemon + standalone CLI) is by design — the CLI writes directly in standalone mode and the daemon reconciles. `busy_timeout=5000` makes that contention robust against transient `SQLITE_BUSY`. No locking redesign in scope.

---

## Part B — Home directory layout

### B.1 Target layout

```
~/.edgeplane/                         (= $EP_HOME, unchanged)
├── config/    cron.toml, config.yaml, <cli config>, contexts.yaml, servers
├── state/     registry.db*  receipts.db*  state.json(+.bak)  session.json(0600)
│              agent_id  instances/  sessions/  profiles/  infisical_profiles.json
│              (+ skills, if skills_home_dir resolves here today — verify in impl)
├── run/       edgeplaned.sock  mgmt.sock  secrets.sock  edgeplaned.lock
├── work/      agent workspaces
└── edgeplaned/   ← transition tombstone ONLY: holds cron.toml -> ../config/cron.toml
                    symlink for one release, then deleted
```

Sorting axis is **function**, single axis. `edgeplaned/` is not a peer bucket; the only reason it survives one release is the back-compat symlink.

### B.2 File → bucket mapping (full inventory)

| File / dir | Bucket | Current location | Owner |
|------------|--------|------------------|-------|
| `cron.toml` | `config/` | `edgeplaned/` | daemon (hand-edited) |
| `config.yaml` | `config/` | `edgeplaned/` | daemon |
| CLI config (`config_file_path()`) | `config/` | root | CLI |
| `contexts.yaml` | `config/` | root | CLI (which controlplane) |
| `servers` | `config/` | root | CLI (discovery-populated) |
| `registry.db` (+`-wal`/`-shm`) | `state/` | `edgeplaned/` | daemon + CLI |
| `receipts.db` (+`-wal`/`-shm`) | `state/` | root | daemon + CLI |
| `state.json` (+`.bak`) | `state/` | `edgeplaned/` | daemon |
| `session.json` (auth, 0600) | `state/` | root | CLI |
| `agent_id` (`agent_id_file()`) | `state/` | root | CLI |
| `infisical_profiles.json` | `state/` | root | CLI |
| `instances/` | `state/` | root | CLI agent harness |
| `sessions/` | `state/` | root | CLI |
| `profiles/` | `state/` | root | CLI |
| `edgeplaned.sock`, `mgmt.sock`, `secrets.sock` | `run/` | `edgeplaned/` | daemon |
| `edgeplaned.lock` | `run/` | `edgeplaned/` | daemon |
| `work/` | `work/` | `edgeplaned/work/` | daemon |

(`skills_home_dir()`: confirm its current resolved path during implementation; map to `state/skills` if it lives under the home, otherwise leave untouched.)

### B.3 Path helper centralization

Path construction is currently duplicated and must agree on the new layout:

- Daemon: `crates/edgeplaned/crates/edgeplaned-core/src/paths.rs` (`mcd_dir`, `mcd_config_path`, `mcd_work_dir`, `registry_db_path`, `state_file_path`, `lock_file_path`, `*_socket_path`, `receipts_db_path`, `session_file_path`, `sync_cache_dir`).
- CLI: `crates/edgeplane/src/config.rs` (`config_file_path`, `ep_home_dir`, `servers_file_path`, `skills_home_dir`, `agent_id_file`), `context.rs` (`contexts.yaml`), `local_db.rs` (registry path).

The `mcd_*` helpers hardcode the `edgeplaned/` join and are **rewritten** (not repointed) to drop the namespace:

```rust
pub fn config_dir() -> PathBuf { ep_home_dir().join("config") }
pub fn state_dir()  -> PathBuf { ep_home_dir().join("state") }
pub fn run_dir()    -> PathBuf { ep_home_dir().join("run") }
pub fn work_dir()   -> PathBuf { ep_home_dir().join("work") }
```

Make `edgeplaned-core::paths` the **single source** of these four bucket helpers. The CLI already references `edgeplaned_core::paths` (e.g. `agent_ops.rs`), so the dependency is viable; make it explicit if not already direct. All concrete file helpers derive from the four bucket fns. Keep thin CLI wrappers if needed for ergonomics, but they must delegate, not recompute. Add a test asserting daemon and CLI agree on each concrete path.

**Sweep inline path joins, not just named helpers.** Several sites build paths inline rather than calling a helper and must be routed through the bucket fns too — at minimum `crates/edgeplane/src/cmd/receipts.rs:49` (`ep_home_dir().join("receipts.db")`), `crates/edgeplane/src/maintenance.rs:116` (`instances`), `crates/edgeplane/src/agent_harness.rs:372` (`instances/<id>`), and `crates/edgeplane/src/local_db.rs:19` (`edgeplaned/registry.db`). Grep for `ep_home_dir().join(` / `mcd_dir().join(` across both binaries during implementation and convert each to the appropriate bucket helper.

### B.4 Migration (`migrate_once` v2 + compat shim)

Extend the existing one-shot migration (`crates/edgeplaned/crates/edgeplaned-core/src/migrate.rs`, currently `.migrated-v1`) with a v2 pass writing `.migrated-v2`. Reuse `move_file` / `move_db` (the latter moves `-wal`/`-shm` siblings). Soft-fail per the existing contract: warn, never block boot.

The v2 pass:
1. `mkdir -p` `config/`, `state/`, `run/`, `work/`.
2. Move every file from its current location (both `edgeplaned/` and root) into its bucket per B.2.
3. Create back-compat symlink `~/.edgeplane/edgeplaned/cron.toml -> ../config/cron.toml`.
4. Leave the (now near-empty) `edgeplaned/` dir in place for the transition; do **not** delete it this release.

**Compat shim (one release):**
- Bucket-aware helpers read `config/<f>` (etc.) first, fall back to the legacy location if the new path is absent. This covers the window between binary upgrade and the migration pass, and any external reader.
- `MCD_CRON_FILE` env override already exists; the symlink + fallback-read mean the documented `~/.edgeplane/edgeplaned/cron.toml` path and `edgeplane agent cron reload` keep working unchanged.

**Removal (next release):** delete the fallback-read branch, the symlink creation, and the `edgeplaned/` dir (a `.migrated-v3` pass `rm`s the empty tombstone).

### B.5 Junk cleanup

Delete `~/.edgeplane/edgeplaned/hn.html` and `hn-news.html` in the v2 migration pass. No in-tree code writes them (confirmed by grep), so they are orphaned legacy artifacts — no writer to fix, just remove.

### B.6 Docs updated in the same PR

- `~/code/aria/.claude/rule-library/scheduling.md` — cron.toml path.
- `~/code/aria/profiles/*/CLAUDE.md` and root `CLAUDE.md` — `~/.edgeplane/edgeplaned/cron.toml` references.
- aria `MEMORY.md` cron/edgeplaned-layout lines.
- Any `~/.ep/` references in edgeplane source doc-comments touched by the move (cosmetic, opportunistic).

---

## Testing

- **Pragma:** unit test opening each DB asserts `journal_size_limit` and `busy_timeout` are set; a test writes > 8 MB of churn and asserts the `-wal` file returns ≤ 8 MB after a `wal_checkpoint(TRUNCATE)`.
- **Migration:** build a fixture home populated with the *current* layout (files in both `edgeplaned/` and root, live `-wal`/`-shm` siblings), run `migrate_once`, assert every file landed in the right bucket, the cron symlink resolves, and `.migrated-v2` is written. Re-run asserts idempotency (no double-move).
- **Path agreement:** test asserting daemon and CLI resolve identical absolute paths for every shared file.
- **Fallback-read:** test that a helper finds a file at the legacy location when the new bucket path is absent.
- `cargo nextest` across the touched crates; `clippy -D warnings`.

## Rollout

1. Merge; new daemon binary migrates on next `systemctl --user restart edgeplaned`.
2. Symlink + dual-read keep the running fleet and muscle-memory paths working through the transition.
3. Verify post-restart: `ls ~/.edgeplane/{config,state,run,work}`, WAL bounded, `edgeplane agent cron list` works, fleet services healthy.
4. Next release: drop the shim and delete the `edgeplaned/` tombstone.

## Risks

- **Moving live data** is the real risk. Mitigated by: soft-fail migration, `move_db` sibling handling, idempotent sentinel, fixture test against a populated home, and the daemon being stopped during its own restart (no in-flight writes mid-move). The CLI could theoretically write mid-migration; the `.migrated-v2` sentinel + create-dir-first ordering make a partial run resumable.
- **Missed path reference** breaking a service. Mitigated by the path-agreement test and the dual-read fallback covering anything not yet repointed.
- **Symlink on exotic FS.** Soft-fail: if symlink creation fails, the `MCD_CRON_FILE` fallback-read still resolves the moved file; warn and continue.
