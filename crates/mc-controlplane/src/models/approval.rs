use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ApprovalRequest {
    pub id: i32,
    pub mission_id: String,
    pub action: String,
    pub channel: String,
    pub reason: String,
    pub target_entity_type: String,
    pub target_entity_id: String,
    pub request_context_json: String,
    pub status: String,
    pub requested_by: String,
    pub approved_by: String,
    pub rejected_by: String,
    pub decision_note: String,
    pub approval_nonce: String,
    pub approval_expires_at: Option<NaiveDateTime>,
    pub approved_at: Option<NaiveDateTime>,
    pub rejected_at: Option<NaiveDateTime>,
    pub executed_at: Option<NaiveDateTime>,
    pub executed_action: String,
    pub executed_request_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

// ── request/response shapes ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ApprovalRequestCreate {
    pub mission_id: String,
    pub action: String,
    pub channel: Option<String>,
    pub reason: Option<String>,
    pub target_entity_type: Option<String>,
    pub target_entity_id: Option<String>,
    pub request_context: Option<serde_json::Value>,
    pub expires_in_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalRespond {
    /// "approve" or "reject"
    pub decision: String,
    pub note: Option<String>,
    pub expires_in_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalListQuery {
    pub mission_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
}
