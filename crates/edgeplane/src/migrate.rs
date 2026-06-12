//! One-shot self-heal of legacy CLI on-disk paths.
//!
//! The daemon runs `edgeplaned_core::migrate::migrate_once()` to relocate the
//! flat `~/.edgeplane/` layout into the `config/` + `state/` buckets, but:
//!   1. the `edgeplane` CLI never ran any migration of its own, and
//!   2. the daemon pass is sentinel-gated, so config/session files written at
//!      the legacy root by an older binary *after* the sentinel was set are
//!      stranded — the CLI then reads the (empty) canonical paths and silently
//!      falls back to a hard-coded localhost context.
//!
//! This helper relocates the CLI-critical config/session files on every CLI
//! startup. It is idempotent (only moves when the canonical destination is
//! absent), NOT sentinel-gated (so it heals stray files written at any time),
//! and soft-fail (never blocks the command).

use edgeplaned_paths::{config_dir, ep_home_dir, sessions_dir, state_dir};
use std::path::Path;

/// Relocate legacy root-level CLI config/session files into the canonical
/// `config/` and `state/` buckets. Safe to call unconditionally at startup.
///
/// Only touches the CLI-owned config/session surface; daemon-owned files
/// (registry/receipts DBs, daemon state, instances) are left to the daemon's
/// own `migrate_once` so the CLI never races the daemon over live state.
pub fn heal_legacy_cli_paths() {
    let home = ep_home_dir();

    // config/ bucket
    move_if_dest_absent(&home.join("contexts.yaml"), &config_dir().join("contexts.yaml"));
    move_if_dest_absent(&home.join("config.json"), &config_dir().join("config.json"));

    // state/ bucket
    move_if_dest_absent(&home.join("session.json"), &state_dir().join("session.json"));
    merge_dir_into(&home.join("sessions"), &sessions_dir());
}

/// Move `src` → `dst` only when `dst` does not yet exist. If both exist, the
/// canonical `dst` wins and the stray `src` is left in place (logged) rather
/// than clobbering a possibly-newer canonical file. Cross-filesystem safe
/// (rename → copy+remove fallback). Soft-fail.
fn move_if_dest_absent(src: &Path, dst: &Path) {
    if !src.exists() {
        return;
    }
    if dst.exists() {
        tracing::warn!(
            "edgeplane: legacy file {} present but canonical {} already exists — leaving stray in place",
            src.display(),
            dst.display()
        );
        return;
    }
    if let Some(parent) = dst.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!("edgeplane: could not create {}: {e}", parent.display());
        return;
    }
    match std::fs::rename(src, dst) {
        Ok(()) => tracing::info!("edgeplane: relocated {} → {}", src.display(), dst.display()),
        Err(_) => {
            // rename fails across filesystems — copy then remove the source.
            match std::fs::copy(src, dst).and_then(|_| std::fs::remove_file(src)) {
                Ok(_) => tracing::info!(
                    "edgeplane: relocated (copy) {} → {}",
                    src.display(),
                    dst.display()
                ),
                Err(e) => tracing::warn!(
                    "edgeplane: could not relocate {} → {}: {e}",
                    src.display(),
                    dst.display()
                ),
            }
        }
    }
}

/// Merge children of legacy `src_dir` into `dst_dir`, moving only entries whose
/// destination is absent. Removes `src_dir` if it ends up empty. Soft-fail.
fn merge_dir_into(src_dir: &Path, dst_dir: &Path) {
    if !src_dir.is_dir() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(dst_dir) {
        tracing::warn!("edgeplane: could not create {}: {e}", dst_dir.display());
        return;
    }
    let entries = match std::fs::read_dir(src_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("edgeplane: could not read {}: {e}", src_dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let from = entry.path();
        let Some(name) = from.file_name() else { continue };
        move_if_dest_absent(&from, &dst_dir.join(name));
    }
    // Best-effort: drop the now-empty legacy dir (no-op if non-empty).
    let _ = std::fs::remove_dir(src_dir);
}
