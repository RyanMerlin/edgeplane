//! Single source of truth for the EdgePlane local on-disk layout and the
//! canonical tuned SQLite open. Depended on by both `edgeplane` (CLI) and the
//! `edgeplaned-*` daemon crates so they never disagree on where files live.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// `EP_HOME` if set to a non-empty value (with `~/` expansion), else None.
/// When set, it is the single unified root for every bucket and `XDG_*` is
/// ignored (an explicit operator override).
fn ep_home_override() -> Option<PathBuf> {
    match std::env::var("EP_HOME") {
        Ok(val) if !val.is_empty() => Some(expand_home(&val)),
        _ => None,
    }
}

/// The unified default home, `~/.edgeplane` (or `./.edgeplane` if home is unknown).
fn unified_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".edgeplane")
}

/// Root home: `$EP_HOME` if set and non-empty, else `~/.edgeplane`.
///
/// NOTE: the bucket directories (config/state/run/work) MAY diverge from
/// `ep_home_dir().join(bucket)` when an `XDG_*` var is set and `EP_HOME` is not.
/// Always use the bucket accessors below — never `ep_home_dir().join("config")`.
pub fn ep_home_dir() -> PathBuf {
    ep_home_override().unwrap_or_else(unified_home)
}

/// Resolve a bucket dir: `EP_HOME/<bucket>` when EP_HOME is set; else
/// `$<xdg_var>/edgeplane` when that XDG var is set non-empty; else
/// `~/.edgeplane/<bucket>`. Buckets resolve independently.
fn bucket_dir(xdg_var: &str, bucket: &str) -> PathBuf {
    if let Some(root) = ep_home_override() {
        return root.join(bucket);
    }
    if let Ok(val) = std::env::var(xdg_var)
        && !val.is_empty()
    {
        return PathBuf::from(val).join("edgeplane");
    }
    unified_home().join(bucket)
}

pub fn config_dir() -> PathBuf {
    bucket_dir("XDG_CONFIG_HOME", "config")
}
pub fn state_dir() -> PathBuf {
    bucket_dir("XDG_STATE_HOME", "state")
}
pub fn run_dir() -> PathBuf {
    bucket_dir("XDG_RUNTIME_DIR", "run")
}
pub fn work_dir() -> PathBuf {
    bucket_dir("XDG_DATA_HOME", "work")
}

fn expand_home(val: &str) -> PathBuf {
    if let Some(stripped) = val.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(stripped);
    }
    PathBuf::from(val)
}

// ── config/ ──────────────────────────────────────────────────────────────
pub fn cron_config_path() -> PathBuf {
    config_dir().join("cron.toml")
}
pub fn daemon_config_path() -> PathBuf {
    config_dir().join("config.yaml")
}
pub fn cli_config_path() -> PathBuf {
    config_dir().join("config.json")
}
pub fn contexts_path() -> PathBuf {
    config_dir().join("contexts.yaml")
}
pub fn servers_path() -> PathBuf {
    config_dir().join("servers")
}

// ── state/ ───────────────────────────────────────────────────────────────
pub fn registry_db_path() -> PathBuf {
    state_dir().join("registry.db")
}
pub fn receipts_db_path() -> PathBuf {
    state_dir().join("receipts.db")
}
pub fn state_file_path() -> PathBuf {
    state_dir().join("state.json")
}
pub fn session_file_path() -> PathBuf {
    state_dir().join("session.json")
}
pub fn agent_id_path() -> PathBuf {
    state_dir().join("agent_id")
}
pub fn infisical_profiles_path() -> PathBuf {
    state_dir().join("infisical_profiles.json")
}
pub fn instances_dir() -> PathBuf {
    state_dir().join("instances")
}
pub fn sessions_dir() -> PathBuf {
    state_dir().join("sessions")
}
pub fn profiles_dir() -> PathBuf {
    state_dir().join("profiles")
}
pub fn sync_cache_dir() -> PathBuf {
    state_dir().join("sync")
}

/// Per-profile secrets file: `<state>/profiles/<profile>/secrets.json`.
/// The single definition consumed by both the CLI and the tower.
pub fn profile_secrets_path(profile: &str) -> PathBuf {
    profiles_dir().join(profile).join("secrets.json")
}

/// User-provided schema-pack override dir: `<ep-home>/schema-packs`.
/// Top-level home item (not a bucket); honors `EP_HOME`.
pub fn schema_packs_dir() -> PathBuf {
    ep_home_dir().join("schema-packs")
}

// ── run/ ─────────────────────────────────────────────────────────────────
pub fn attach_socket_path() -> PathBuf {
    run_dir().join("edgeplaned.sock")
}
pub fn mgmt_socket_path() -> PathBuf {
    run_dir().join("mgmt.sock")
}
pub fn secrets_socket_path() -> PathBuf {
    run_dir().join("secrets.sock")
}
pub fn lock_file_path() -> PathBuf {
    run_dir().join("edgeplaned.lock")
}

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
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let _ = std::fs::create_dir_all(parent);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_children_of_home() {
        // SAFETY: tests in this crate run single-threaded (--test-threads=1).
        unsafe { std::env::set_var("EP_HOME", "/tmp/ep-test-home") };
        assert_eq!(config_dir(), PathBuf::from("/tmp/ep-test-home/config"));
        assert_eq!(state_dir(), PathBuf::from("/tmp/ep-test-home/state"));
        assert_eq!(run_dir(), PathBuf::from("/tmp/ep-test-home/run"));
        assert_eq!(work_dir(), PathBuf::from("/tmp/ep-test-home/work"));
        unsafe { std::env::remove_var("EP_HOME") };
    }

    #[test]
    fn ep_home_overrides_all_buckets_ignoring_xdg() {
        // SAFETY: crate tests run single-threaded / process-isolated.
        unsafe {
            std::env::set_var("EP_HOME", "/srv/ep");
            std::env::set_var("XDG_CONFIG_HOME", "/x/cfg");
        }
        assert_eq!(config_dir(), PathBuf::from("/srv/ep/config"));
        assert_eq!(state_dir(), PathBuf::from("/srv/ep/state"));
        assert_eq!(run_dir(), PathBuf::from("/srv/ep/run"));
        assert_eq!(work_dir(), PathBuf::from("/srv/ep/work"));
        unsafe {
            std::env::remove_var("EP_HOME");
            std::env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn xdg_moves_only_its_own_bucket() {
        unsafe {
            std::env::remove_var("EP_HOME");
            std::env::set_var("XDG_CONFIG_HOME", "/x/cfg");
            std::env::remove_var("XDG_STATE_HOME");
        }
        assert_eq!(config_dir(), PathBuf::from("/x/cfg/edgeplane"));
        // state has no XDG var set -> falls back under the unified ~/.edgeplane home
        assert!(state_dir().ends_with(".edgeplane/state"));
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
    }

    #[test]
    fn empty_xdg_var_is_treated_as_unset() {
        unsafe {
            std::env::remove_var("EP_HOME");
            std::env::set_var("XDG_STATE_HOME", "");
        }
        assert!(state_dir().ends_with(".edgeplane/state"));
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
    }

    #[test]
    fn profile_secrets_path_under_state_profiles() {
        unsafe {
            std::env::set_var("EP_HOME", "/srv/ep");
            std::env::remove_var("XDG_STATE_HOME");
        }
        assert_eq!(
            profile_secrets_path("work"),
            PathBuf::from("/srv/ep/state/profiles/work/secrets.json")
        );
        unsafe { std::env::remove_var("EP_HOME") };
    }

    #[test]
    fn schema_packs_dir_under_home_root() {
        unsafe { std::env::set_var("EP_HOME", "/srv/ep") };
        assert_eq!(schema_packs_dir(), PathBuf::from("/srv/ep/schema-packs"));
        unsafe { std::env::remove_var("EP_HOME") };
    }

    #[test]
    fn open_tuned_sets_wal_and_size_limit() {
        let dir = std::env::temp_dir().join("ep-paths-tune-test");
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("t.db");
        let _ = std::fs::remove_file(&db);
        let conn = open_tuned(&db).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        let limit: i64 = conn
            .query_row("PRAGMA journal_size_limit;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(limit, 8_388_608);
        let busy: i64 = conn
            .query_row("PRAGMA busy_timeout;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(busy, 5000);
    }
}
