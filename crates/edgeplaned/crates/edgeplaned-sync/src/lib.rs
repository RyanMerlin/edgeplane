pub mod client;
pub mod error;
pub mod types;

pub use client::SyncClient;
pub use error::SyncError;
pub use types::{PushResult, SyncResult, SyncState, SyncStatus};

use std::path::PathBuf;

pub fn default_cache_dir() -> PathBuf {
    edgeplaned_paths::sync_cache_dir()
}
