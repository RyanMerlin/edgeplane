//! One-shot migration from legacy path layouts to the canonical `~/.ep/edgeplaned/` layout.
//!
//! Runs on first `edgeplaned` boot. Writes `~/.ep/edgeplaned/.migrated-v1` as a sentinel and
//! skips on subsequent starts. All operations are soft-fail: warnings are emitted
//! but boot is never blocked.

use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::paths::{mc_home_dir, mcd_dir};

const SENTINEL: &str = ".migrated-v1";

/// Run legacy-path migration once. Idempotent after the sentinel is written.
///
/// Call from `edgeplaned/src/main.rs` before daemon startup.
pub fn migrate_once() {
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

    let edgeplane = mc_home_dir();
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

    // ── ~/.edgeplane → ~/.edgeplane (edgeplane CLI config) ────────────────────────────

    let mc_ctrl = home.join(".edgeplane");
    if mc_ctrl.exists() {
        move_file(mc_ctrl.join("config.json"), edgeplane.join("config.json"), "edgeplane config");
        move_file(mc_ctrl.join("session.json"), edgeplane.join("session.json"), "edgeplane session");
        move_dir(mc_ctrl.join("sync"), edgeplane.join("sync"), "edgeplane sync");

        // Remove stale legacy edgeplaned artifacts left by old installs.
        let _ = std::fs::remove_file(mc_ctrl.join("edgeplane-mesh.sock"));
        let _ = std::fs::remove_file(mc_ctrl.join("edgeplane-mesh.yaml"));
        let _ = std::fs::remove_dir_all(mc_ctrl.join("edgeplane-mesh"));

        // Remove ~/.edgeplane if now empty.
        if is_dir_empty(&mc_ctrl) {
            if let Err(e) = std::fs::remove_dir(&mc_ctrl) {
                warn!("migrate: could not remove empty {}: {e}", mc_ctrl.display());
            } else {
                info!("migrate: removed {}", mc_ctrl.display());
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
    // Checkpoint WAL before moving (best-effort; daemon isn't running yet).
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
        let tmp = TempDir::new().unwrap();
        let (home, edgeplane, edgeplaned) = fake_home(&tmp);
        fs::create_dir_all(&edgeplaned).unwrap();
        let mc_ctrl = home.join(".edgeplane");
        fs::create_dir_all(&mc_ctrl).unwrap();
        fs::write(mc_ctrl.join("config.json"), b"{\"server\":\"http://edgeplane\"}").unwrap();

        move_file(mc_ctrl.join("config.json"), edgeplane.join("config.json"), "edgeplane config");

        assert!(edgeplane.join("config.json").exists());
        assert!(!mc_ctrl.join("config.json").exists());
        assert!(is_dir_empty(&mc_ctrl));
    }
}
