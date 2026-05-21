use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AgentRun {
    pub id: String,
    pub owner_subject: String,
    pub mesh_agent_id: Option<String>,
    pub mesh_task_id: Option<String>,
    pub runtime_kind: String,
    pub runtime_session_id: Option<String>,
    pub status: String,
    pub started_at: Option<NaiveDateTime>,
    pub ended_at: Option<NaiveDateTime>,
    pub resume_token: String,
    pub last_checkpoint_at: Option<NaiveDateTime>,
    pub total_cost_cents: i32,
    pub parent_run_id: Option<String>,
    pub metadata_json: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RunCheckpoint {
    pub id: String,
    pub run_id: String,
    pub seq: i32,
    pub kind: String,
    pub payload_json: String,
    pub created_at: NaiveDateTime,
}

#[derive(Deserialize)]
pub struct StartRunRequest {
    pub runtime_kind: String,
    // Accept both the historic short names and the column-aligned names.
    // The fields bind to agentrun.mesh_agent_id / mesh_task_id server-side;
    // callers following column naming should not silently drop to NULL.
    #[serde(alias = "mesh_agent_id")]
    pub agent_id: Option<String>,
    #[serde(alias = "mesh_task_id")]
    pub task_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub runtime_session_id: Option<String>,
}

#[derive(Deserialize)]
pub struct CheckpointRequest {
    pub kind: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Deserialize)]
pub struct ResumeRequest {
    pub resume_token: String,
}

#[derive(Deserialize)]
pub struct ListRunsQuery {
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
}
