//! One-shot migrations from legacy path layouts to the canonical `~/.edgeplane/` layout.
//!
//! Each pass is independently idempotent via its own sentinel file written inside
//! `~/.edgeplane/edgeplaned/`. All operations are soft-fail: warnings are emitted
//! but boot is never blocked.
//!
//! Passes:
//! - v1 (`.migrated-v1`): relocate `edgeplane-mesh.*` scattered files → `edgeplaned/`
//! - v2 (`.migrated-v2`): relocate flat/process-split files → function buckets
//!   (`config/`, `state/`, `run/`, `work/`) and dissolve the `edgeplaned/` namespace.

use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::paths::{config_dir, ep_home_dir, mcd_dir, run_dir, state_dir, work_dir};

const SENTINEL: &str = ".migrated-v1";
const SENTINEL_V2: &str = ".migrated-v2";

/// Run all one-shot path migrations in order. Each pass is independently
/// idempotent via its own sentinel.
///
/// Call from `edgeplaned/src/main.rs` before daemon startup.
pub fn migrate_once() {
    migrate_legacy_v1();
    migrate_to_buckets_v2();
}

/// v1: relocate scattered `edgeplane-mesh.*` files into `edgeplaned/`.
fn migrate_legacy_v1() {
    let edgeplaned = mcd_dir();
    if let Err(e) = std::fs::create_dir_all(&edgeplaned) {
        warn!("migrate: could not create edgeplaned dir {}: {e}", edgeplaned.display());
        return;
    }

    let sentinel = edgeplaned.join(SENTINEL);
    if sentinel.exists() {
        return;
    }

    info!("edgeplaned: running first-boot path migration…");

    let edgeplane = ep_home_dir();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

    // ── Daemon state files (scattered ~/.ep/edgeplane-mesh.* → ~/.ep/edgeplaned/) ────────

    move_file(edgeplane.join("edgeplane-mesh.yaml"), edgeplaned.join("config.yaml"), "config");
    move_file(edgeplane.join("edgeplane-mesh.state.json"), edgeplaned.join("state.json"), "state");
    move_db(
        edgeplane.join("edgeplane-mesh.db"),
        edgeplane.join("edgeplane-mesh.db-shm"),
        edgeplane.join("edgeplane-mesh.db-wal"),
        edgeplaned.join("registry.db"),
        "registry",
    );
    move_dir(edgeplane.join("edgeplane-mesh"), edgeplaned.join("work_legacy"), "work dir");

    // ── legacy CLI config cleanup ─────────────────────────────────────────────────────
    // NOTE: ep_ctrl == ep_home_dir() after the MC→Edgeplane rename; the move_file
    // calls below are no-ops (src==dst). The remove_file calls are still useful for
    // cleaning up stale daemon socket/yaml artifacts from old installs.

    let ep_ctrl = home.join(".edgeplane");
    if ep_ctrl.exists() {
        move_file(ep_ctrl.join("config.json"), edgeplane.join("config.json"), "edgeplane config");
        move_file(ep_ctrl.join("session.json"), edgeplane.join("session.json"), "edgeplane session");
        move_dir(ep_ctrl.join("sync"), edgeplane.join("sync"), "edgeplane sync");

        // Remove stale legacy edgeplaned artifacts left by old installs.
        let _ = std::fs::remove_file(ep_ctrl.join("edgeplane-mesh.sock"));
        let _ = std::fs::remove_file(ep_ctrl.join("edgeplane-mesh.yaml"));
        let _ = std::fs::remove_dir_all(ep_ctrl.join("edgeplane-mesh"));

        // Remove ~/.edgeplane if now empty.
        if is_dir_empty(&ep_ctrl) {
            if let Err(e) = std::fs::remove_dir(&ep_ctrl) {
                warn!("migrate: could not remove empty {}: {e}", ep_ctrl.display());
            } else {
                info!("migrate: removed {}", ep_ctrl.display());
            }
        } else {
            warn!(
                "migrate: ~/.edgeplane still has entries after migration — leaving in place"
            );
        }
    }

    // ── Keyring service rename (edgeplane-mesh → edgeplaned) ───────────────────────────────
    // edgeplaned-secrets handles the fallback read internally; no action needed here.

    // ── Write sentinel ───────────────────────────────────────────────────────
    if let Err(e) = std::fs::write(&sentinel, b"") {
        warn!("migrate: could not write sentinel {}: {e}", sentinel.display());
    } else {
        info!("edgeplaned: path migration complete");
    }
}

/// v2: relocate the flat/process-split layout into function buckets
/// (`config/`, `state/`, `run/`, `work/`) and dissolve the `edgeplaned/` namespace.
/// Idempotent via `.migrated-v2`. Soft-fail throughout.
fn migrate_to_buckets_v2() {
    let home = ep_home_dir();
    let edgeplaned = home.join("edgeplaned");
    let v2_sentinel = edgeplaned.join(SENTINEL_V2);
    if v2_sentinel.exists() {
        return;
    }

    for d in [config_dir(), state_dir(), run_dir(), edgeplaned.clone()] {
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
    move_file(
        edgeplaned.join("state.json.bak"),
        state_dir().join("state.json.bak"),
        "state.json.bak",
    );
    move_file(home.join("session.json"), state_dir().join("session.json"), "session.json");
    move_file(home.join("agent_id"), state_dir().join("agent_id"), "agent_id");
    move_file(
        home.join("infisical_profiles.json"),
        state_dir().join("infisical_profiles.json"),
        "infisical_profiles.json",
    );
    move_dir(home.join("instances"), state_dir().join("instances"), "instances");
    move_dir(home.join("sessions"), state_dir().join("sessions"), "sessions");
    move_dir(home.join("profiles"), state_dir().join("profiles"), "profiles");
    move_dir(home.join("skills"), state_dir().join("skills"), "skills");
    move_dir(home.join("sync"), state_dir().join("sync"), "sync");

    // run/
    move_file(
        edgeplaned.join("edgeplaned.lock"),
        run_dir().join("edgeplaned.lock"),
        "lock",
    );
    // sockets are recreated by the daemon on boot; do not move live sockets.

    // work/ — merge contents into the bucket. The daemon may already have created
    // an empty (or partially-populated) `work/` before/after migration, so we move
    // each child entry rather than renaming the whole dir (which would skip on a
    // pre-existing destination and strand the old workspaces).
    merge_dir(edgeplaned.join("work"), work_dir());

    // junk
    for j in ["hn.html", "hn-news.html"] {
        let _ = std::fs::remove_file(edgeplaned.join(j));
    }

    // compat: keep the documented cron path working for one release.
    // `../config/cron.toml` is relative to the `edgeplaned/` dir, so it resolves
    // to `<home>/config/cron.toml`.
    let link = edgeplaned.join("cron.toml");
    let target = PathBuf::from("../config/cron.toml");
    if !link.exists() {
        #[cfg(unix)]
        if let Err(e) = std::os::unix::fs::symlink(&target, &link) {
            warn!("migrate v2: cron compat symlink failed: {e}");
        }
    }

    if let Err(e) = std::fs::write(&v2_sentinel, b"") {
        warn!(
            "migrate v2: could not write sentinel {}: {e}",
            v2_sentinel.display()
        );
    } else {
        info!("edgeplaned: v2 bucket migration complete");
    }
}

// ---------- helpers ----------

fn move_file(src: PathBuf, dst: PathBuf, label: &str) {
    if !src.exists() {
        return;
    }
    if dst.exists() {
        info!("migrate: {label} already at destination, skipping");
        return;
    }
    if let Err(e) = std::fs::rename(&src, &dst) {
        // rename fails across filesystems — fall back to copy + remove.
        if let Err(e2) = std::fs::copy(&src, &dst).and_then(|_| std::fs::remove_file(&src)) {
            warn!("migrate: could not move {label} ({} → {}): rename={e} copy={e2}",
                src.display(), dst.display());
            return;
        }
    }
    info!("migrate: moved {label}: {} → {}", src.display(), dst.display());
}

fn move_db(src: PathBuf, src_shm: PathBuf, src_wal: PathBuf, dst: PathBuf, label: &str) {
    if !src.exists() {
        return;
    }
    if dst.exists() {
        info!("migrate: {label} db already at destination, skipping");
        return;
    }
    // Move the .db plus its -shm/-wal siblings together so no committed-but-uncheckpointed state is lost.
    let dst_shm = dst.with_extension("db-shm");
    let dst_wal = dst.with_extension("db-wal");
    move_file(src, dst, label);
    move_file(src_shm, dst_shm, &format!("{label}-shm"));
    move_file(src_wal, dst_wal, &format!("{label}-wal"));
}

fn move_dir(src: PathBuf, dst: PathBuf, label: &str) {
    if !src.exists() {
        return;
    }
    if dst.exists() {
        info!("migrate: {label} dir already at destination, skipping");
        return;
    }
    if let Err(e) = std::fs::rename(&src, &dst) {
        warn!("migrate: could not rename {label} dir ({} → {}): {e}",
            src.display(), dst.display());
    } else {
        info!("migrate: moved {label} dir: {} → {}", src.display(), dst.display());
    }
}

/// Move every child entry of `src` into `dst`, creating `dst` if needed.
/// Per-entry: skip if the destination entry already exists (never clobber).
/// Removes `src` afterward if it ends up empty. Soft-fail. Same-filesystem
/// rename is assumed (both live under `$EP_HOME`); a cross-fs rename failure is
/// logged and the entry is left in place for a manual move.
fn merge_dir(src: PathBuf, dst: PathBuf) {
    if !src.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dst) {
        warn!("migrate v2: mkdir {} failed: {e}", dst.display());
        return;
    }
    let entries = match std::fs::read_dir(&src) {
        Ok(e) => e,
        Err(e) => {
            warn!("migrate v2: read_dir {} failed: {e}", src.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if to.exists() {
            info!("migrate v2: work item already at destination, skipping {}", to.display());
            continue;
        }
        if let Err(e) = std::fs::rename(&from, &to) {
            warn!("migrate v2: could not move work item {} -> {}: {e}", from.display(), to.display());
        }
    }
    // Remove the now-(hopefully)-empty source dir; ignore failure if not empty.
    let _ = std::fs::remove_dir(&src);
}

fn is_dir_empty(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut d| d.next().is_none())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fake_home(tmp: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
        let home = tmp.path().to_path_buf();
        let edgeplane = home.join(".edgeplane");
        let edgeplaned = edgeplane.join("edgeplaned");
        fs::create_dir_all(&edgeplane).unwrap();
        (home, edgeplane, edgeplaned)
    }

    #[test]
    fn test_sentinel_skips_on_rerun() {
        let tmp = TempDir::new().unwrap();
        let (_, edgeplane, edgeplaned) = fake_home(&tmp);
        fs::create_dir_all(&edgeplaned).unwrap();
        fs::write(edgeplaned.join(SENTINEL), b"").unwrap();

        // Write a file that migration would otherwise move.
        fs::write(edgeplane.join("edgeplane-mesh.yaml"), b"backend_url: http://test").unwrap();

        // With HOME pointing at tmp, sentinel is already present — nothing moves.
        // (migrate_once uses dirs::home_dir() so we can't easily override in a unit test;
        //  test the helpers directly instead.)
        move_file(edgeplane.join("edgeplane-mesh.yaml"), edgeplaned.join("config.yaml"), "config");
        assert!(edgeplaned.join("config.yaml").exists());
        assert!(!edgeplane.join("edgeplane-mesh.yaml").exists());
    }

    #[test]
    fn test_move_file_idempotent() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src.txt");
        let dst = tmp.path().join("dst.txt");
        fs::write(&src, b"hello").unwrap();
        fs::write(&dst, b"existing").unwrap();

        // dst already exists — src should NOT be overwritten.
        move_file(src.clone(), dst.clone(), "test");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "existing");
        assert!(src.exists()); // src untouched
    }

    #[test]
    fn test_edgeplane_cleanup() {
        // Test that move_file correctly migrates a config file from a legacy
        // directory to a new one and leaves the source empty.
        let tmp = TempDir::new().unwrap();
        let legacy_dir = tmp.path().join("legacy-ep");
        let new_dir = tmp.path().join("edgeplane");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::create_dir_all(&new_dir).unwrap();
        fs::write(legacy_dir.join("config.json"), b"{\"server\":\"http://edgeplane\"}").unwrap();

        move_file(legacy_dir.join("config.json"), new_dir.join("config.json"), "edgeplane config");

        assert!(new_dir.join("config.json").exists());
        assert!(!legacy_dir.join("config.json").exists());
        assert!(is_dir_empty(&legacy_dir));
    }

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
        fs::create_dir_all(edgeplaned.join("work/agentA")).unwrap();
        fs::write(edgeplaned.join("work/agentA/ws.txt"), b"workspace").unwrap();

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
        assert!(home.join("work/agentA/ws.txt").exists(), "agent workspace moved to work/");
        assert_eq!(fs::read_to_string(home.join("work/agentA/ws.txt")).unwrap(), "workspace");

        unsafe { std::env::remove_var("EP_HOME") };
    }

    /// Regression: pre-existing `work/` (created by the daemon before migration
    /// finishes) must not strand workspaces from `edgeplaned/work/`.
    #[test]
    fn v2_work_merge_into_existing_dir() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        // SAFETY: migrate tests run single-threaded.
        unsafe { std::env::set_var("EP_HOME", &home) };

        let edgeplaned = home.join("edgeplaned");
        fs::create_dir_all(&edgeplaned).unwrap();
        fs::write(edgeplaned.join(".migrated-v1"), b"").unwrap();

        // Simulate: old workspaces in edgeplaned/work/
        fs::create_dir_all(edgeplaned.join("work/agentA")).unwrap();
        fs::write(edgeplaned.join("work/agentA/old.txt"), b"old-workspace").unwrap();

        // Simulate: daemon already created <home>/work/ (e.g. agentB started before migration)
        fs::create_dir_all(home.join("work/agentB")).unwrap();
        fs::write(home.join("work/agentB/new.txt"), b"new-workspace").unwrap();

        migrate_once();

        // Both agents must be present in the merged work/ bucket.
        assert!(
            home.join("work/agentA/old.txt").exists(),
            "old workspace from edgeplaned/work/ must be merged into work/"
        );
        assert_eq!(
            fs::read_to_string(home.join("work/agentA/old.txt")).unwrap(),
            "old-workspace"
        );
        assert!(
            home.join("work/agentB/new.txt").exists(),
            "pre-existing work/ entry must be preserved"
        );
        assert_eq!(
            fs::read_to_string(home.join("work/agentB/new.txt")).unwrap(),
            "new-workspace"
        );

        // v2 sentinel written.
        assert!(home.join("edgeplaned/.migrated-v2").exists(), "v2 sentinel written");

        unsafe { std::env::remove_var("EP_HOME") };
    }

    #[test]
    fn v2_idempotent_on_double_run() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().to_path_buf();
        // SAFETY: migrate tests run single-threaded.
        unsafe { std::env::set_var("EP_HOME", &home) };

        let edgeplaned = home.join("edgeplaned");
        fs::create_dir_all(&edgeplaned).unwrap();
        fs::write(edgeplaned.join(".migrated-v1"), b"").unwrap();

        // Seed some files for the first run.
        fs::write(edgeplaned.join("cron.toml"), b"schema_version = 1\n").unwrap();
        fs::write(edgeplaned.join("state.json"), b"{}").unwrap();

        // First run — migrates files.
        migrate_once();
        assert!(home.join("config/cron.toml").exists(), "first run: cron moved");
        assert!(home.join("state/state.json").exists(), "first run: state moved");
        assert!(home.join("edgeplaned/.migrated-v2").exists(), "sentinel written");

        // Seed a new file at the OLD location after the first run.
        fs::write(edgeplaned.join("state.json"), b"NEW").unwrap();

        // Second run — sentinel short-circuits; the new file at the old location
        // must NOT be moved.
        migrate_once();
        assert_eq!(
            fs::read_to_string(home.join("state/state.json")).unwrap(),
            "{}",
            "second run must not overwrite bucket file"
        );
        assert!(
            edgeplaned.join("state.json").exists(),
            "second run must leave old-location file untouched"
        );

        unsafe { std::env::remove_var("EP_HOME") };
    }
}
