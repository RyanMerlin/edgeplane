# EdgePlane Home Layout + SQLite WAL Fix — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound the SQLite WAL files and re-lay `~/.edgeplane/` by function (`config/ state/ run/ work/`), dissolving the `edgeplaned/` namespace, with zero breakage for the live fleet.

**Architecture:** Introduce one tiny leaf crate `edgeplaned-paths` (deps: `dirs`, `rusqlite`) that is the single source of truth for both the on-disk directory layout AND the canonical tuned SQLite open. Both binaries depend on it; `edgeplaned-core::paths` and `edgeplane::config` delegate to it. A `migrate_once` v2 pass relocates existing files into the new buckets and leaves a one-release `cron.toml` compat symlink.

**Tech Stack:** Rust, rusqlite 0.31 (bundled), tokio, cargo-nextest. Spec: `docs/superpowers/specs/2026-06-10-edgeplane-home-layout-and-wal-design.md`.

---

## File Structure

**New:**
- `crates/edgeplaned-paths/Cargo.toml` — leaf crate manifest.
- `crates/edgeplaned-paths/src/lib.rs` — layout helpers + sqlite tuning. Single responsibility: "where local files live and how we open them."

**Modified (delegation / repoint):**
- `crates/edgeplaned/crates/edgeplaned-core/src/paths.rs` — re-export bucket helpers from `edgeplaned-paths`; rewrite concrete helpers to buckets.
- `crates/edgeplane/src/config.rs`, `context.rs`, `local_db.rs`, `cmd/receipts.rs`, `maintenance.rs`, `agent_harness.rs` — delegate path construction to `edgeplaned-paths`.
- `crates/edgeplaned/crates/edgeplaned-bin/src/local_registry.rs` — two open sites use `open_tuned`.
- `crates/edgeplaned/crates/edgeplaned-receipts/src/store.rs` — open site uses `open_tuned`.
- `crates/edgeplaned/crates/edgeplaned-bin/src/cron_config.rs` — `default_path()` → `config_dir()`.
- `crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs` — periodic + shutdown WAL checkpoint.
- `crates/edgeplaned/crates/edgeplaned-core/src/migrate.rs` — v2 migration pass.
- Workspace `Cargo.toml` — register the new crate.

**Docs (separate commit):** `~/code/aria/.claude/rule-library/scheduling.md`, aria `CLAUDE.md` files, aria `MEMORY.md`.

---

## Task 1: Create the `edgeplaned-paths` leaf crate

**Files:**
- Create: `crates/edgeplaned-paths/Cargo.toml`
- Create: `crates/edgeplaned-paths/src/lib.rs`
- Modify: root `Cargo.toml` (workspace members)

- [ ] **Step 1: Add the crate to the workspace members list**

Find the `[workspace] members = [...]` array in the root `/home/merlin/code/edgeplane/Cargo.toml` and add:

```toml
    "crates/edgeplaned-paths",
```

- [ ] **Step 2: Write the crate manifest**

Create `crates/edgeplaned-paths/Cargo.toml`:

```toml
[package]
name = "edgeplaned-paths"
version = "0.1.0"
edition = "2021"

[dependencies]
dirs = { workspace = true }
rusqlite = { workspace = true }
```

If `rusqlite` is not a workspace dependency (it is declared per-crate as `rusqlite = { version = "0.31", features = ["bundled"] }`), use that literal form here instead of `{ workspace = true }`:

```toml
rusqlite = { version = "0.31", features = ["bundled"] }
```

- [ ] **Step 3: Write the failing test for bucket layout**

Create `crates/edgeplaned-paths/src/lib.rs` with tests first:

```rust
//! Single source of truth for the EdgePlane local on-disk layout and the
//! canonical tuned SQLite open. Depended on by both `edgeplane` (CLI) and the
//! `edgeplaned-*` daemon crates so they never disagree on where files live.

use std::path::{Path, PathBuf};

/// Root home: `$EP_HOME` if set and non-empty, else `~/.edgeplane`.
pub fn ep_home_dir() -> PathBuf {
    if let Ok(val) = std::env::var("EP_HOME") {
        if !val.is_empty() {
            return expand_home(&val);
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".edgeplane")
}

pub fn config_dir() -> PathBuf { ep_home_dir().join("config") }
pub fn state_dir() -> PathBuf { ep_home_dir().join("state") }
pub fn run_dir() -> PathBuf { ep_home_dir().join("run") }
pub fn work_dir() -> PathBuf { ep_home_dir().join("work") }

fn expand_home(val: &str) -> PathBuf {
    if let Some(stripped) = val.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_children_of_home() {
        // SAFETY: tests in this crate run single-threaded (set --test-threads=1).
        unsafe { std::env::set_var("EP_HOME", "/tmp/ep-test-home") };
        assert_eq!(config_dir(), PathBuf::from("/tmp/ep-test-home/config"));
        assert_eq!(state_dir(), PathBuf::from("/tmp/ep-test-home/state"));
        assert_eq!(run_dir(), PathBuf::from("/tmp/ep-test-home/run"));
        assert_eq!(work_dir(), PathBuf::from("/tmp/ep-test-home/work"));
        unsafe { std::env::remove_var("EP_HOME") };
    }
}
```

- [ ] **Step 4: Run the test, expect PASS**

Run: `cargo nextest run -p edgeplaned-paths --test-threads=1`
Expected: PASS (1 test).

- [ ] **Step 5: Add the concrete file-path helpers**

Append to `src/lib.rs` (above the `tests` module):

```rust
// ── config/ ──────────────────────────────────────────────────────────────
pub fn cron_config_path() -> PathBuf { config_dir().join("cron.toml") }
pub fn daemon_config_path() -> PathBuf { config_dir().join("config.yaml") }
pub fn cli_config_path() -> PathBuf { config_dir().join("config.json") }
pub fn contexts_path() -> PathBuf { config_dir().join("contexts.yaml") }
pub fn servers_path() -> PathBuf { config_dir().join("servers") }

// ── state/ ───────────────────────────────────────────────────────────────
pub fn registry_db_path() -> PathBuf { state_dir().join("registry.db") }
pub fn receipts_db_path() -> PathBuf { state_dir().join("receipts.db") }
pub fn state_file_path() -> PathBuf { state_dir().join("state.json") }
pub fn session_file_path() -> PathBuf { state_dir().join("session.json") }
pub fn agent_id_path() -> PathBuf { state_dir().join("agent_id") }
pub fn infisical_profiles_path() -> PathBuf { state_dir().join("infisical_profiles.json") }
pub fn instances_dir() -> PathBuf { state_dir().join("instances") }
pub fn sessions_dir() -> PathBuf { state_dir().join("sessions") }
pub fn profiles_dir() -> PathBuf { state_dir().join("profiles") }
pub fn skills_dir() -> PathBuf { state_dir().join("skills") }
pub fn sync_cache_dir() -> PathBuf { state_dir().join("sync") }

// ── run/ ─────────────────────────────────────────────────────────────────
pub fn attach_socket_path() -> PathBuf { run_dir().join("edgeplaned.sock") }
pub fn mgmt_socket_path() -> PathBuf { run_dir().join("mgmt.sock") }
pub fn secrets_socket_path() -> PathBuf { run_dir().join("secrets.sock") }
pub fn lock_file_path() -> PathBuf { run_dir().join("edgeplaned.lock") }
```

- [ ] **Step 6: Add the SQLite tuning helpers with a failing test**

Append to `src/lib.rs` (above `tests`):

```rust
use rusqlite::Connection;

/// Canonical pragma set for every WAL-mode SQLite DB in the home.
/// `journal_size_limit` caps the WAL file so it truncates back down after a
/// checkpoint; `busy_timeout` survives the two-writer races on registry.db.
pub const TUNE_PRAGMAS: &str = "\
PRAGMA journal_mode=WAL;\
PRAGMA synchronous=NORMAL;\
PRAGMA busy_timeout=5000;\
PRAGMA wal_autocheckpoint=1000;\
PRAGMA journal_size_limit=8388608;\
PRAGMA foreign_keys=ON;";

/// Open a connection at `path` with the canonical pragma set applied.
/// Creates the parent directory if needed. Callers add their own schema after.
pub fn open_tuned(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(TUNE_PRAGMAS)?;
    Ok(conn)
}

/// Force-drain and truncate the WAL for the DB at `path`. Best-effort: a
/// concurrent long-lived reader may limit how much is reclaimed.
pub fn checkpoint_truncate(path: &Path) -> rusqlite::Result<()> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE);", [], |_row| Ok(()))?;
    Ok(())
}
```

Add this test inside the `tests` module:

```rust
    #[test]
    fn open_tuned_sets_wal_and_size_limit() {
        let dir = std::env::temp_dir().join("ep-paths-tune-test");
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("t.db");
        let _ = std::fs::remove_file(&db);
        let conn = open_tuned(&db).unwrap();
        let mode: String = conn.query_row("PRAGMA journal_mode;", [], |r| r.get(0)).unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        let limit: i64 = conn.query_row("PRAGMA journal_size_limit;", [], |r| r.get(0)).unwrap();
        assert_eq!(limit, 8_388_608);
        let busy: i64 = conn.query_row("PRAGMA busy_timeout;", [], |r| r.get(0)).unwrap();
        assert_eq!(busy, 5000);
    }
```

- [ ] **Step 7: Run the tests, expect PASS**

Run: `cargo nextest run -p edgeplaned-paths --test-threads=1`
Expected: PASS (2 tests).

- [ ] **Step 8: Commit**

```bash
git add crates/edgeplaned-paths Cargo.toml
git commit -m "feat(paths): add edgeplaned-paths SSOT crate (layout + tuned sqlite open)"
```

---

## Task 2: WAL fix — route the four open sites through `open_tuned`

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/local_registry.rs` (`open` ~206, `replace_source` ~431)
- Modify: `crates/edgeplane/src/local_db.rs` (`open` ~46)
- Modify: `crates/edgeplaned/crates/edgeplaned-receipts/src/store.rs` (`open` ~26)
- Modify: the `Cargo.toml` of `edgeplaned-bin`, `edgeplane`, `edgeplaned-receipts` (add `edgeplaned-paths` dep)

- [ ] **Step 1: Add the dependency to the three consuming crates**

In each of these `[dependencies]` sections add `edgeplaned-paths = { path = "..." }` with the correct relative path:

- `crates/edgeplaned/crates/edgeplaned-bin/Cargo.toml`: `edgeplaned-paths = { path = "../../../edgeplaned-paths" }`
- `crates/edgeplane/Cargo.toml`: `edgeplaned-paths = { path = "../edgeplaned-paths" }`
- `crates/edgeplaned/crates/edgeplaned-receipts/Cargo.toml`: `edgeplaned-paths = { path = "../../../edgeplaned-paths" }`

Verify each relative path resolves: `ls <crate-dir>/<relative-path>/Cargo.toml` must show the new crate's manifest.

- [ ] **Step 2: Replace the registry `open` site**

In `crates/edgeplaned/crates/edgeplaned-bin/src/local_registry.rs`, the `open` fn (~line 206) currently:

```rust
        let conn = Connection::open(path)
            .with_context(|| format!("opening registry {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", 1)?;
```

Replace those four lines with:

```rust
        let conn = edgeplaned_paths::open_tuned(path)
            .with_context(|| format!("opening registry {}", path.display()))?;
```

(The directory-creation block above it becomes redundant since `open_tuned` does it, but leave it — harmless and explicit.)

- [ ] **Step 3: Replace the registry `replace_source` site**

Same file, `replace_source` (~line 432):

```rust
        let mut conn = Connection::open(path)
            .with_context(|| format!("opening registry for replace_source: {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
```

Replace with:

```rust
        let mut conn = edgeplaned_paths::open_tuned(path)
            .with_context(|| format!("opening registry for replace_source: {}", path.display()))?;
```

- [ ] **Step 4: Replace the CLI `local_db.rs` open site**

In `crates/edgeplane/src/local_db.rs`, `open` (~line 46):

```rust
    let conn = Connection::open(&path)
        .with_context(|| format!("opening local registry {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
```

Replace with:

```rust
    let conn = edgeplaned_paths::open_tuned(&path)
        .with_context(|| format!("opening local registry {}", path.display()))?;
```

- [ ] **Step 5: Replace the receipts store open site**

In `crates/edgeplaned/crates/edgeplaned-receipts/src/store.rs`, `open` (~line 26). Change:

```rust
        let conn = Connection::open(path)?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS receipts (
```

to (drop the two PRAGMA lines; `open_tuned` sets them):

```rust
        let conn = edgeplaned_paths::open_tuned(path)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS receipts (
```

- [ ] **Step 6: Build + run existing tests for the three crates**

Run: `cargo nextest run -p edgeplaned-bin -p edgeplane -p edgeplaned-receipts`
Expected: PASS (the existing `local_registry`/`receipts` tests still pass; behaviour unchanged except pragmas).

If `Connection` is now an unused import in any file, remove it (`cargo clippy -p <crate> -D warnings` will flag it).

- [ ] **Step 7: Run clippy on the touched crates**

Run: `cargo clippy -p edgeplaned-bin -p edgeplane -p edgeplaned-receipts -- -D warnings`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/edgeplane crates/edgeplaned
git commit -m "fix(sqlite): tune WAL (journal_size_limit, busy_timeout) at all open sites"
```

---

## Task 3: Daemon periodic + shutdown WAL checkpoint

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs` (background task ~547; shutdown ~595)

- [ ] **Step 1: Add the periodic checkpoint task**

In `daemon.rs`, just after the `gc_task` spawn block (~line 557, after the `tokio::spawn(async move { crate::unit_health::gc_task(...).await });` block), add:

```rust
        // Keep the SQLite WAL files bounded: TRUNCATE-checkpoint registry + receipts
        // on a 60s cadence. journal_size_limit caps the file; this actively reclaims it.
        let ckpt_registry = edgeplaned_paths::registry_db_path();
        let ckpt_receipts = edgeplaned_paths::receipts_db_path();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            tick.tick().await; // consume the immediate first tick
            loop {
                tick.tick().await;
                for p in [ckpt_registry.clone(), ckpt_receipts.clone()] {
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Err(e) = edgeplaned_paths::checkpoint_truncate(&p) {
                            tracing::debug!("wal checkpoint {}: {e}", p.display());
                        }
                    })
                    .await;
                }
            }
        });
```

- [ ] **Step 2: Add the shutdown checkpoint**

In the same file, after the ctrl-c/select shutdown block and before the socket cleanup (~line 595, just before `let _ = std::fs::remove_file(attach_gateway::socket_path());`), add:

```rust
    // Drain the WAL on graceful shutdown so the files don't linger at high-water mark.
    let _ = edgeplaned_paths::checkpoint_truncate(&edgeplaned_paths::registry_db_path());
    let _ = edgeplaned_paths::checkpoint_truncate(&edgeplaned_paths::receipts_db_path());
```

- [ ] **Step 3: Build the daemon**

Run: `cargo build -p edgeplaned-bin`
Expected: compiles. (`edgeplaned-paths` dep was added in Task 2 Step 1.)

- [ ] **Step 4: Manual smoke check of the checkpoint helper**

Run (uses the dev binary against a temp db):

```bash
cargo nextest run -p edgeplaned-paths --test-threads=1
```

Expected: PASS — the `open_tuned`/checkpoint behaviour is already unit-covered; no daemon runtime needed here.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs
git commit -m "feat(daemon): periodic + shutdown wal_checkpoint(TRUNCATE) for registry/receipts"
```

---

## Task 4: Repoint daemon path helpers to the new buckets

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-core/src/paths.rs`
- Modify: `crates/edgeplaned/crates/edgeplaned-core/Cargo.toml` (add `edgeplaned-paths` dep)

- [ ] **Step 1: Add the dependency**

In `crates/edgeplaned/crates/edgeplaned-core/Cargo.toml` `[dependencies]` add:

```toml
edgeplaned-paths = { path = "../../../edgeplaned-paths" }
```

- [ ] **Step 2: Rewrite `paths.rs` as a delegating shim**

Replace the body of `crates/edgeplaned/crates/edgeplaned-core/src/paths.rs` with delegations. Keep every existing public fn name (daemon call sites depend on them) but point them at buckets:

```rust
use std::path::PathBuf;

pub use edgeplaned_paths::{config_dir, ep_home_dir, run_dir, state_dir, work_dir};

// Back-compat name kept for daemon call sites. Was `~/.edgeplane/edgeplaned`;
// now the function buckets, so this returns the home root (no namespace dir).
pub fn mcd_dir() -> PathBuf { ep_home_dir() }
pub fn mcd_work_dir() -> PathBuf { work_dir() }
pub fn mcd_config_path() -> PathBuf { edgeplaned_paths::daemon_config_path() }

pub fn session_file_path() -> PathBuf { edgeplaned_paths::session_file_path() }
pub fn receipts_db_path() -> PathBuf { edgeplaned_paths::receipts_db_path() }
pub fn attach_socket_path() -> PathBuf { edgeplaned_paths::attach_socket_path() }
pub fn mgmt_socket_path() -> PathBuf { edgeplaned_paths::mgmt_socket_path() }
pub fn secrets_socket_path() -> PathBuf { edgeplaned_paths::secrets_socket_path() }
pub fn registry_db_path() -> PathBuf { edgeplaned_paths::registry_db_path() }
pub fn state_file_path() -> PathBuf { edgeplaned_paths::state_file_path() }
pub fn lock_file_path() -> PathBuf { edgeplaned_paths::lock_file_path() }
pub fn sync_cache_dir() -> PathBuf { edgeplaned_paths::sync_cache_dir() }
```

Note: `mcd_dir()` is referenced by `cron_config::default_path()` — Task 5 repoints that separately, so `mcd_dir()` returning the home root here is a transitional value, not used for cron after Task 5.

- [ ] **Step 3: Build edgeplaned-core and its dependents**

Run: `cargo build -p edgeplaned-core -p edgeplaned-bin`
Expected: compiles. If any daemon call site used `mcd_dir().join("cron.toml")` directly (other than `cron_config`), grep and convert:

Run: `rg -n "mcd_dir\(\)\.join" crates/edgeplaned`
Expected after review: only `cron_config.rs` (handled in Task 5) and any work/socket joins already covered by named helpers.

- [ ] **Step 4: Run daemon-core tests**

Run: `cargo nextest run -p edgeplaned-core -p edgeplaned-bin`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-core
git commit -m "refactor(paths): delegate daemon paths to edgeplaned-paths buckets"
```

---

## Task 5: Repoint CLI path helpers + cron default to buckets

**Files:**
- Modify: `crates/edgeplane/src/config.rs` (`config_file_path`, `servers_file_path`, `skills_home_dir`, `agent_id_file`, `ensure_mc_dirs`)
- Modify: `crates/edgeplane/src/context.rs` (`contexts.yaml`)
- Modify: `crates/edgeplane/src/local_db.rs` (`db_path`, `is_federated` state path)
- Modify: `crates/edgeplane/src/cmd/receipts.rs:49` (inline `receipts.db`)
- Modify: `crates/edgeplane/src/maintenance.rs:116` (inline `instances`)
- Modify: `crates/edgeplane/src/agent_harness.rs:372` (inline `instances`)
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/cron_config.rs` (`default_path`)
- Modify: auth/session path (`session.json`) — find via grep

- [ ] **Step 1: Repoint the named CLI helpers in `config.rs`**

Change each to delegate (keep the fn names; `ep_home_dir` stays as-is since `edgeplaned_paths::ep_home_dir` is identical):

```rust
pub fn config_file_path() -> PathBuf { edgeplaned_paths::cli_config_path() }
pub fn servers_file_path() -> PathBuf { edgeplaned_paths::servers_path() }
pub fn skills_home_dir() -> PathBuf { edgeplaned_paths::skills_dir() }
pub fn agent_id_file() -> PathBuf { edgeplaned_paths::agent_id_path() }
```

And update `ensure_mc_dirs()` to create the buckets it needs:

```rust
pub fn ensure_mc_dirs() -> std::io::Result<()> {
    fs::create_dir_all(edgeplaned_paths::config_dir())?;
    fs::create_dir_all(edgeplaned_paths::state_dir())?;
    fs::create_dir_all(skills_home_dir())?;
    Ok(())
}
```

- [ ] **Step 2: Repoint `context.rs` and the session path**

In `crates/edgeplane/src/context.rs` (~line 34) change `ep_home_dir().join("contexts.yaml")` to `edgeplaned_paths::contexts_path()`.

Grep for the session.json constructor and repoint it:

Run: `rg -n 'join\("session.json"\)' crates/edgeplane`
For each hit, replace `ep_home_dir().join("session.json")` with `edgeplaned_paths::session_file_path()`.

- [ ] **Step 3: Repoint `local_db.rs` inline joins**

In `crates/edgeplane/src/local_db.rs`:

```rust
pub fn db_path() -> std::path::PathBuf {
    edgeplaned_paths::registry_db_path()
}
```

and in `is_federated()` change `ep_home_dir().join("edgeplaned").join("state.json")` to `edgeplaned_paths::state_file_path()`.

- [ ] **Step 4: Repoint the remaining CLI inline joins**

- `crates/edgeplane/src/cmd/receipts.rs:49`: replace `crate::config::ep_home_dir().join("receipts.db")` with `edgeplaned_paths::receipts_db_path()`.
- `crates/edgeplane/src/maintenance.rs:116`: replace `root.join("instances")` with `edgeplaned_paths::instances_dir()` (confirm `root` is `ep_home_dir()`; if it is a passed-in arg, instead point the caller at `state_dir()`).
- `crates/edgeplane/src/agent_harness.rs:372`: replace `base_mc_home.join("instances").join(&runtime_session_id)` with `edgeplaned_paths::instances_dir().join(&runtime_session_id)`.

Also grep for `infisical_profiles.json` and `sessions`/`profiles` dir constructors and repoint to `infisical_profiles_path()` / `sessions_dir()` / `profiles_dir()`:

Run: `rg -n 'infisical_profiles.json|join\("sessions"\)|join\("profiles"\)' crates/edgeplane`

- [ ] **Step 5: Repoint the cron default path**

In `crates/edgeplaned/crates/edgeplaned-bin/src/cron_config.rs`, `default_path()`:

```rust
pub fn default_path() -> PathBuf {
    edgeplaned_paths::cron_config_path()
}
```

(`edgeplaned-bin` already has the `edgeplaned-paths` dep from Task 2.) The `MCD_CRON_FILE` env override and `config_override` precedence in `resolve_path` are unchanged.

- [ ] **Step 6: Build the whole workspace**

Run: `cargo build --workspace`
Expected: compiles. Fix any remaining `edgeplaned_paths` import (`use edgeplaned_paths;` is not needed — fully-qualified calls work once the dep is declared).

- [ ] **Step 7: Run the CLI + daemon test suites**

Run: `cargo nextest run -p edgeplane -p edgeplaned-bin -p edgeplaned-core`
Expected: PASS. The `local_registry` tests set `EP_HOME` to a temp dir; they should still pass because the registry path now resolves to `<temp>/state/registry.db` and `open_tuned` creates the parent.

- [ ] **Step 8: Clippy + commit**

Run: `cargo clippy --workspace -- -D warnings`

```bash
git add crates/edgeplane crates/edgeplaned
git commit -m "refactor(paths): route CLI + cron paths through edgeplaned-paths buckets"
```

---

## Task 6: `migrate_once` v2 — relocate into buckets + compat symlink + junk delete

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-core/src/migrate.rs`

- [ ] **Step 1: Write the failing fixture test**

Add to the `tests` module in `migrate.rs` (it already imports `super::*`, `std::fs`, `TempDir`, and has a `fake_home` helper):

```rust
    #[test]
    fn v2_relocates_files_into_buckets() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        // SAFETY: migrate tests run single-threaded.
        unsafe { std::env::set_var("EP_HOME", &home) };

        let edgeplaned = home.join("edgeplaned");
        fs::create_dir_all(&edgeplaned).unwrap();
        // v1 sentinel present so only the v2 pass runs.
        fs::write(edgeplaned.join(".migrated-v1"), b"").unwrap();

        // Seed current-layout files in both levels.
        fs::write(edgeplaned.join("cron.toml"), b"schema_version = 1\n").unwrap();
        fs::write(edgeplaned.join("config.yaml"), b"backend_url: x\n").unwrap();
        fs::write(edgeplaned.join("state.json"), b"{}").unwrap();
        fs::write(edgeplaned.join("registry.db"), b"db").unwrap();
        fs::write(edgeplaned.join("registry.db-wal"), b"wal").unwrap();
        fs::write(edgeplaned.join("hn.html"), b"junk").unwrap();
        fs::write(home.join("receipts.db"), b"r").unwrap();
        fs::write(home.join("contexts.yaml"), b"c").unwrap();
        fs::write(home.join("session.json"), b"s").unwrap();

        migrate_once();

        assert!(home.join("config/cron.toml").exists(), "cron.toml -> config/");
        assert!(home.join("config/config.yaml").exists(), "config.yaml -> config/");
        assert!(home.join("config/contexts.yaml").exists(), "contexts.yaml -> config/");
        assert!(home.join("state/state.json").exists(), "state.json -> state/");
        assert!(home.join("state/registry.db").exists(), "registry.db -> state/");
        assert!(home.join("state/registry.db-wal").exists(), "wal sibling moved");
        assert!(home.join("state/receipts.db").exists(), "receipts.db -> state/");
        assert!(home.join("state/session.json").exists(), "session.json -> state/");
        assert!(!edgeplaned.join("hn.html").exists(), "hn.html junk deleted");
        // cron compat symlink resolves to the moved file.
        let link = edgeplaned.join("cron.toml");
        assert!(link.exists(), "cron.toml compat symlink present");
        assert_eq!(fs::read_to_string(&link).unwrap(), "schema_version = 1\n");
        assert!(home.join("edgeplaned/.migrated-v2").exists(), "v2 sentinel written");

        unsafe { std::env::remove_var("EP_HOME") };
    }
```

- [ ] **Step 2: Run the test, expect FAIL**

Run: `cargo nextest run -p edgeplaned-core v2_relocates --test-threads=1`
Expected: FAIL (no v2 pass yet; files not moved).

- [ ] **Step 3: Add the v2 sentinel constant and pass**

At the top of `migrate.rs` near `const SENTINEL: &str = ".migrated-v1";` add:

```rust
const SENTINEL_V2: &str = ".migrated-v2";
```

**Restructure `migrate_once` so the v1 early-return does not skip v2.** The current `migrate_once()` does `if sentinel.exists() { return; }`, which on every already-migrated machine would skip a v2 call placed at its end. Fix by extracting the existing body into a v1-private fn and running both passes, each with its own sentinel guard:

1. Rename the current `pub fn migrate_once()` to `fn migrate_legacy_v1()` (keep its entire body, including its own `if sentinel.exists() { return; }`).
2. Inside `migrate_legacy_v1()`, change its first line `let edgeplaned = mcd_dir();` to the literal `let edgeplaned = ep_home_dir().join("edgeplaned");`. This pins v1 to its original target so the `mcd_dir()` rewrite in Task 4 (which now returns the home root) cannot alter v1 semantics.
3. Add a new dispatcher:

```rust
/// Run all one-shot path migrations in order. Each pass is independently
/// idempotent via its own sentinel.
pub fn migrate_once() {
    migrate_legacy_v1();
    migrate_to_buckets_v2();
}
```

Then add the v2 function (after the dispatcher, before `// ---------- helpers ----------`):

```rust
/// v2: relocate the flat/process-split layout into function buckets
/// (config/ state/ run/ work/) and dissolve the `edgeplaned/` namespace.
/// Idempotent via `.migrated-v2`. Soft-fail throughout.
fn migrate_to_buckets_v2() {
    use edgeplaned_paths::{config_dir, run_dir, state_dir, work_dir};

    let home = ep_home_dir();
    let edgeplaned = home.join("edgeplaned");
    let v2_sentinel = edgeplaned.join(SENTINEL_V2);
    if v2_sentinel.exists() {
        return;
    }

    for d in [config_dir(), state_dir(), run_dir(), work_dir(), edgeplaned.clone()] {
        if let Err(e) = std::fs::create_dir_all(&d) {
            warn!("migrate v2: mkdir {} failed: {e}", d.display());
            return;
        }
    }

    // config/
    move_file(edgeplaned.join("cron.toml"), config_dir().join("cron.toml"), "cron.toml");
    move_file(edgeplaned.join("config.yaml"), config_dir().join("config.yaml"), "config.yaml");
    move_file(home.join("config.json"), config_dir().join("config.json"), "cli config.json");
    move_file(home.join("contexts.yaml"), config_dir().join("contexts.yaml"), "contexts.yaml");
    move_file(home.join("servers"), config_dir().join("servers"), "servers");

    // state/
    move_db(
        edgeplaned.join("registry.db"),
        edgeplaned.join("registry.db-shm"),
        edgeplaned.join("registry.db-wal"),
        state_dir().join("registry.db"),
        "registry",
    );
    move_db(
        home.join("receipts.db"),
        home.join("receipts.db-shm"),
        home.join("receipts.db-wal"),
        state_dir().join("receipts.db"),
        "receipts",
    );
    move_file(edgeplaned.join("state.json"), state_dir().join("state.json"), "state.json");
    move_file(edgeplaned.join("state.json.bak"), state_dir().join("state.json.bak"), "state.json.bak");
    move_file(home.join("session.json"), state_dir().join("session.json"), "session.json");
    move_file(home.join("agent_id"), state_dir().join("agent_id"), "agent_id");
    move_file(home.join("infisical_profiles.json"), state_dir().join("infisical_profiles.json"), "infisical_profiles.json");
    move_dir(home.join("instances"), state_dir().join("instances"), "instances");
    move_dir(home.join("sessions"), state_dir().join("sessions"), "sessions");
    move_dir(home.join("profiles"), state_dir().join("profiles"), "profiles");
    move_dir(home.join("skills"), state_dir().join("skills"), "skills");
    move_dir(home.join("sync"), state_dir().join("sync"), "sync");

    // run/
    move_file(edgeplaned.join("edgeplaned.lock"), run_dir().join("edgeplaned.lock"), "lock");
    // sockets are recreated by the daemon on boot; do not move live sockets.

    // work/
    move_dir(edgeplaned.join("work"), work_dir(), "work");

    // junk
    for j in ["hn.html", "hn-news.html"] {
        let _ = std::fs::remove_file(edgeplaned.join(j));
    }

    // compat: keep the documented cron path working for one release.
    let link = edgeplaned.join("cron.toml");
    let target = std::path::PathBuf::from("../config/cron.toml");
    if !link.exists() {
        #[cfg(unix)]
        if let Err(e) = std::os::unix::fs::symlink(&target, &link) {
            warn!("migrate v2: cron compat symlink failed: {e}");
        }
    }

    if let Err(e) = std::fs::write(&v2_sentinel, b"") {
        warn!("migrate v2: could not write sentinel {}: {e}", v2_sentinel.display());
    } else {
        info!("edgeplaned: v2 bucket migration complete");
    }
}
```

Add the dep if not present: `edgeplaned-core/Cargo.toml` already gained `edgeplaned-paths` in Task 4 Step 1.

- [ ] **Step 4: Run the test, expect PASS**

Run: `cargo nextest run -p edgeplaned-core v2_relocates --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Run idempotency — re-run migrate, sentinel skips**

Add a second assertion test (re-run `migrate_once()` after the first and assert files are not double-moved and no panic). Then:

Run: `cargo nextest run -p edgeplaned-core --test-threads=1`
Expected: PASS (all migrate tests, including the existing v1 ones).

- [ ] **Step 6: Clippy + commit**

Run: `cargo clippy -p edgeplaned-core -- -D warnings`

```bash
git add crates/edgeplaned/crates/edgeplaned-core/src/migrate.rs
git commit -m "feat(migrate): v2 relocate to config/state/run/work buckets + cron compat symlink"
```

---

## Task 7: Update docs + memory references (aria repo)

**Files (in `/home/merlin/code/aria`):**
- Modify: `.claude/rule-library/scheduling.md`
- Modify: `CLAUDE.md`, `profiles/*/CLAUDE.md` (cron.toml path references)
- Modify: `profiles/engineer/CLAUDE.md` (the "Scheduling" section references `~/.edgeplane/edgeplaned/cron.toml`)
- Modify: aria auto-memory `MEMORY.md` line for edgeplaned layout

- [ ] **Step 1: Find every stale path reference**

Run (from `/home/merlin/code/aria`):

```bash
rg -n 'edgeplane/edgeplaned/cron.toml|\.edgeplane/edgeplaned' --glob '!**/target/**'
```

- [ ] **Step 2: Update the canonical cron path in prose**

For each hit, change `~/.edgeplane/edgeplaned/cron.toml` to `~/.edgeplane/config/cron.toml`, and add a one-line note where the scheduling pattern is documented: "(legacy `~/.edgeplane/edgeplaned/cron.toml` still works for one release via a compat symlink)."

- [ ] **Step 3: Commit the aria-repo doc changes separately**

```bash
cd /home/merlin/code/aria
git add .claude/rule-library/scheduling.md CLAUDE.md profiles
git commit -m "docs(scheduling): cron.toml moved to ~/.edgeplane/config/ (compat symlink one release)"
```

(The `MEMORY.md` update is an auto-memory edit — apply it via the Write tool to the memory file, not git, per the memory system.)

---

## Verification (whole-feature, before opening the PR)

- [ ] `cargo nextest run --workspace` — all green.
- [ ] `cargo clippy --workspace -- -D warnings` — clean.
- [ ] Dry-run the migration against a copy of the real home:
  ```bash
  cp -a ~/.edgeplane /tmp/ep-home-copy
  EP_HOME=/tmp/ep-home-copy cargo run -p edgeplaned-bin -- run --help >/dev/null 2>&1 || true
  # then run the daemon's migrate path in a one-shot harness, or inspect after a guarded start
  ls -R /tmp/ep-home-copy | head -40
  ```
  Confirm `config/ state/ run/ work/` populated, `state/registry.db-wal` ≤ 8 MB after a checkpoint, `edgeplaned/cron.toml` is a symlink resolving to `config/cron.toml`, `hn*.html` gone.
- [ ] Restart the real daemon (`systemctl --user restart edgeplaned`), then:
  - `ls ~/.edgeplane` shows the four buckets + the `edgeplaned/` tombstone.
  - `edgeplane agent cron list` works (reads via the symlink/new path).
  - `du -h ~/.edgeplane/state/registry.db-wal` stays bounded over a few minutes.
  - Fleet services healthy: `systemctl --user is-active aria.service aria-research.service aria-engineer.service`.

## Out of scope (next release)
- Delete the fallback-read branch, the cron compat symlink, and the empty `edgeplaned/` tombstone (a `.migrated-v3` `rm`).
- XDG split. Not happening; single home is the chosen model.

## Self-review notes
- Spec coverage: WAL pragma (Task 2) + active checkpoint (Task 3) = spec Part A. Buckets + dissolve `edgeplaned/` (Tasks 4–5) + migration/symlink/junk (Task 6) = Part B. Path-helper centralization realized as the `edgeplaned-paths` crate (Task 1). Docs (Task 7).
- Type consistency: `open_tuned`, `checkpoint_truncate`, `TUNE_PRAGMAS`, and the bucket fns (`config_dir`/`state_dir`/`run_dir`/`work_dir`) and file fns are defined once in Task 1 and referenced by exact name thereafter.
- Open verification points carried from the spec, to confirm during execution: `maintenance.rs` `root` is actually `ep_home_dir()` (Task 5 Step 4); exact session.json constructor sites (Task 5 Step 2); whether `edgeplaned-bin` needs `tokio::time` already in scope (it does — daemon.rs already uses tokio).
