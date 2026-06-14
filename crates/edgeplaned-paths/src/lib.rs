//! Single source of truth for the EdgePlane local on-disk layout and the
//! canonical tuned SQLite open. Depended on by both `edgeplane` (CLI) and the
//! `edgeplaned-*` daemon crates so they never disagree on where files live.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

/// Root home: `$EP_HOME` if set and non-empty, else `~/.edgeplane`.
pub fn ep_home_dir() -> PathBuf {
    if let Ok(val) = std::env::var("EP_HOME")
        && !val.is_empty()
    {
        return expand_home(&val);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".edgeplane")
}

pub fn config_dir() -> PathBuf {
    ep_home_dir().join("config")
}
pub fn state_dir() -> PathBuf {
    ep_home_dir().join("state")
}
pub fn run_dir() -> PathBuf {
    ep_home_dir().join("run")
}
pub fn work_dir() -> PathBuf {
    ep_home_dir().join("work")
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
