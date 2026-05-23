use std::{env, path::PathBuf};

/// Returns `~/.edgeplane` by default, or `$EP_HOME` if set — matching the edgeplane CLI.
pub fn mc_home_dir() -> PathBuf {
    if let Ok(val) = env::var("EP_HOME") {
        if !val.is_empty() {
            return expand_home(&val);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".edgeplane")
}

pub fn mcd_dir() -> PathBuf {
    mc_home_dir().join("edgeplaned")
}

pub fn mcd_work_dir() -> PathBuf {
    mcd_dir().join("work")
}

pub fn mcd_config_path() -> PathBuf {
    mcd_dir().join("config.yaml")
}

pub fn session_file_path() -> PathBuf {
    mc_home_dir().join("session.json")
}

pub fn receipts_db_path() -> PathBuf {
    mc_home_dir().join("receipts.db")
}

pub fn attach_socket_path() -> PathBuf {
    mcd_dir().join("edgeplaned.sock")
}

pub fn mgmt_socket_path() -> PathBuf {
    mcd_dir().join("mgmt.sock")
}

pub fn secrets_socket_path() -> PathBuf {
    mcd_dir().join("secrets.sock")
}

pub fn registry_db_path() -> PathBuf {
    mcd_dir().join("registry.db")
}

pub fn state_file_path() -> PathBuf {
    mcd_dir().join("state.json")
}

pub fn lock_file_path() -> PathBuf {
    mcd_dir().join("edgeplaned.lock")
}

pub fn sync_cache_dir() -> PathBuf {
    mc_home_dir().join("sync")
}

fn expand_home(val: &str) -> PathBuf {
    if let Some(stripped) = val.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(val)
}
