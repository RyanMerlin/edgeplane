use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub struct SyncStatus {
    pub last_pulled_at: Option<DateTime<Utc>>,
    pub last_pushed_at: Option<DateTime<Utc>>,
    pub node_branch_dirty: bool,
    pub fleet_branch_ahead: u32,
}

pub struct SyncResult {
    pub pulled_at: DateTime<Utc>,
    pub commits_fetched: u32,
}

pub struct PushResult {
    pub pushed_at: DateTime<Utc>,
    pub branch: String,
    pub files_committed: u32,
}

#[derive(Serialize, Deserialize, Default)]
pub struct SyncState {
    pub last_pulled_at: Option<String>,
    pub last_pushed_at: Option<String>,
    pub commits_fetched: u32,
}
