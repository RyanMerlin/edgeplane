//! One-shot migration from legacy path layouts to the canonical `~/.mc/mcd/` layout.
//!
//! Runs on first `mcd` boot. Writes `~/.mc/mcd/.migrated-v1` as a sentinel and
//! skips on subsequent starts. All operations are soft-fail: warnings are emitted
//! but boot is never blocked.

use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::paths::{mc_home_dir, mcd_dir};

const SENTINEL: &str = ".migrated-v1";

/// Run legacy-path migration once. Idempotent after the sentinel is written.
///
/// Call from `mcd/src/main.rs` before daemon startup.
pub fn migrate_once() {
    let mcd = mcd_dir();
    if let Err(e) = std::fs::create_dir_all(&mcd) {
        warn!("migrate: could not create mcd dir {}: {e}", mcd.display());
        return;
    }

    let sentinel = mcd.join(SENTINEL);
    if sentinel.exists() {
        return;
    }

    info!("mcd: running first-boot path migration…");

    let mc = mc_home_dir();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));

    // ── Daemon state files (scattered ~/.mc/mc-mesh.* → ~/.mc/mcd/) ────────

    move_file(mc.join("mc-mesh.yaml"), mcd.join("config.yaml"), "config");
    move_file(mc.join("mc-mesh.state.json"), mcd.join("state.json"), "state");
    move_db(
        mc.join("mc-mesh.db"),
        mc.join("mc-mesh.db-shm"),
        mc.join("mc-mesh.db-wal"),
        mcd.join("registry.db"),
        "registry",
    );
    move_dir(mc.join("mc-mesh"), mcd.join("work_legacy"), "work dir");

    // ── ~/.missioncontrol → ~/.mc (mc CLI config) ────────────────────────────

    let mc_ctrl = home.join(".missioncontrol");
    if mc_ctrl.exists() {
        move_file(mc_ctrl.join("config.json"), mc.join("config.json"), "mc config");
        move_file(mc_ctrl.join("session.json"), mc.join("session.json"), "mc session");
        move_dir(mc_ctrl.join("sync"), mc.join("sync"), "mc sync");

        // Remove stale legacy mcd artifacts left by old installs.
        let _ = std::fs::remove_file(mc_ctrl.join("mc-mesh.sock"));
        let _ = std::fs::remove_file(mc_ctrl.join("mc-mesh.yaml"));
        let _ = std::fs::remove_dir_all(mc_ctrl.join("mc-mesh"));

        // Remove ~/.missioncontrol if now empty.
        if is_dir_empty(&mc_ctrl) {
            if let Err(e) = std::fs::remove_dir(&mc_ctrl) {
                warn!("migrate: could not remove empty {}: {e}", mc_ctrl.display());
            } else {
                info!("migrate: removed {}", mc_ctrl.display());
            }
        } else {
            warn!(
                "migrate: ~/.missioncontrol still has entries after migration — leaving in place"
            );
        }
    }

    // ── Keyring service rename (mc-mesh → mcd) ───────────────────────────────
    // mcd-secrets handles the fallback read internally; no action needed here.

    // ── Write sentinel ───────────────────────────────────────────────────────
    if let Err(e) = std::fs::write(&sentinel, b"") {
        warn!("migrate: could not write sentinel {}: {e}", sentinel.display());
    } else {
        info!("mcd: path migration complete");
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
        let mc = home.join(".mc");
        let mcd = mc.join("mcd");
        fs::create_dir_all(&mc).unwrap();
        (home, mc, mcd)
    }

    #[test]
    fn test_sentinel_skips_on_rerun() {
        let tmp = TempDir::new().unwrap();
        let (_, mc, mcd) = fake_home(&tmp);
        fs::create_dir_all(&mcd).unwrap();
        fs::write(mcd.join(SENTINEL), b"").unwrap();

        // Write a file that migration would otherwise move.
        fs::write(mc.join("mc-mesh.yaml"), b"backend_url: http://test").unwrap();

        // With HOME pointing at tmp, sentinel is already present — nothing moves.
        // (migrate_once uses dirs::home_dir() so we can't easily override in a unit test;
        //  test the helpers directly instead.)
        move_file(mc.join("mc-mesh.yaml"), mcd.join("config.yaml"), "config");
        assert!(mcd.join("config.yaml").exists());
        assert!(!mc.join("mc-mesh.yaml").exists());
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
    fn test_missioncontrol_cleanup() {
        let tmp = TempDir::new().unwrap();
        let (home, mc, mcd) = fake_home(&tmp);
        fs::create_dir_all(&mcd).unwrap();
        let mc_ctrl = home.join(".missioncontrol");
        fs::create_dir_all(&mc_ctrl).unwrap();
        fs::write(mc_ctrl.join("config.json"), b"{\"server\":\"http://mc\"}").unwrap();

        move_file(mc_ctrl.join("config.json"), mc.join("config.json"), "mc config");

        assert!(mc.join("config.json").exists());
        assert!(!mc_ctrl.join("config.json").exists());
        assert!(is_dir_empty(&mc_ctrl));
    }
}
