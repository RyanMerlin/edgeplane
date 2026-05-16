use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub capability: String,
    pub args_json: String,
    pub result_json: String,
    pub exit_code: i32,
    pub execution_time_ms: u64,
    pub mission_id: Option<String>,
    pub agent_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct ReceiptFilter {
    pub mission_id: Option<String>,
    pub agent_id: Option<String>,
    pub capability: Option<String>,
    pub limit: usize,
    pub since: Option<DateTime<Utc>>,
}
