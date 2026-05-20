pub mod error;
pub mod types;
pub mod client;

pub use client::SyncClient;
pub use error::SyncError;
pub use types::{PushResult, SyncResult, SyncState, SyncStatus};

use std::path::PathBuf;

pub fn default_cache_dir() -> PathBuf {
    mc_home_dir().join("sync")
}

fn mc_home_dir() -> PathBuf {
    if let Ok(val) = std::env::var("MC_HOME") {
        if !val.is_empty() {
            return expand_home(&val);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mc")
}

fn expand_home(val: &str) -> PathBuf {
    if let Some(stripped) = val.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(val)
}
