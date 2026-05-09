use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: i32,
    pub name: String,
    pub capabilities: String,
    pub status: String,
    pub metadata: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AgentSession {
    pub id: i32,
    pub agent_id: i32,
    pub context: String,
    pub started_at: NaiveDateTime,
    pub ended_at: Option<NaiveDateTime>,
    pub claude_session_id: Option<String>,
    pub end_reason: Option<String>,
    pub audit_log: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaskAssignment {
    pub id: i32,
    pub task_id: i32,
    pub agent_id: i32,
    pub status: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AgentMessage {
    pub id: i32,
    pub from_agent_id: i32,
    pub to_agent_id: i32,
    pub content: String,
    pub message_type: String,
    pub task_id: Option<i32>,
    pub read: bool,
    pub created_at: NaiveDateTime,
}

// ── Request shapes ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AgentCreate {
    pub name: String,
    #[serde(default)]
    pub capabilities: String,
    #[serde(default = "default_offline")]
    pub status: String,
    #[serde(default)]
    pub metadata: String,
}

#[derive(Debug, Deserialize)]
pub struct AgentUpdate {
    pub name: Option<String>,
    pub capabilities: Option<String>,
    pub status: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionCreate {
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Deserialize)]
pub struct AssignmentCreate {
    pub task_id: i32,
    pub agent_id: i32,
    #[serde(default = "default_available")]
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct AssignmentUpdate {
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageSend {
    pub to_agent_id: i32,
    pub content: String,
    #[serde(default = "default_info")]
    pub message_type: String,
    pub task_id: Option<i32>,
}

fn default_offline() -> String { "offline".into() }
fn default_available() -> String { "available".into() }
fn default_info() -> String { "info".into() }
