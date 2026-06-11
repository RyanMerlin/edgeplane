use std::path::PathBuf;

pub use edgeplaned_paths::{config_dir, ep_home_dir, run_dir, state_dir, work_dir};

/// Legacy `~/.edgeplane/edgeplaned/` directory. Retained for the one-shot path
/// migrations and the transition-period compat symlink (and, until the cron
/// default is repointed, the live cron config path). New code must use the
/// function buckets (config/state/run/work).
pub fn mcd_dir() -> PathBuf {
    ep_home_dir().join("edgeplaned")
}
pub fn mcd_work_dir() -> PathBuf {
    work_dir()
}
pub fn mcd_config_path() -> PathBuf {
    edgeplaned_paths::daemon_config_path()
}

pub fn session_file_path() -> PathBuf {
    edgeplaned_paths::session_file_path()
}
pub fn receipts_db_path() -> PathBuf {
    edgeplaned_paths::receipts_db_path()
}
pub fn attach_socket_path() -> PathBuf {
    edgeplaned_paths::attach_socket_path()
}
pub fn mgmt_socket_path() -> PathBuf {
    edgeplaned_paths::mgmt_socket_path()
}
pub fn secrets_socket_path() -> PathBuf {
    edgeplaned_paths::secrets_socket_path()
}
pub fn registry_db_path() -> PathBuf {
    edgeplaned_paths::registry_db_path()
}
pub fn state_file_path() -> PathBuf {
    edgeplaned_paths::state_file_path()
}
pub fn lock_file_path() -> PathBuf {
    edgeplaned_paths::lock_file_path()
}
pub fn sync_cache_dir() -> PathBuf {
    edgeplaned_paths::sync_cache_dir()
}
