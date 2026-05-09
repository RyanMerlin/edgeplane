use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{
    models::agent::{
        Agent, AgentCreate, AgentMessage, AgentSession, AgentUpdate, AssignmentCreate,
        AssignmentUpdate, MessageSend, SessionCreate, TaskAssignment,
    },
    state::AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/{agent_id}", get(get_agent).patch(update_agent))
        .route("/agents/{agent_id}/sessions", get(list_sessions).post(start_session))
        .route("/agents/{agent_id}/sessions/{session_id}/end", post(end_session))
        .route("/agents/{agent_id}/message", post(send_message))
        .route("/agents/{agent_id}/messages", get(list_messages))
        .route("/agents/{agent_id}/inbox", get(get_inbox))
        .route("/agents/assignments", get(list_assignments).post(create_assignment))
        .route("/agents/assignments/{assignment_id}", axum::routing::patch(update_assignment))
}

fn row_to_agent(row: &sqlx::postgres::PgRow) -> Agent {
    Agent {
        id: row.get("id"),
        name: row.get("name"),
        capabilities: row.get("capabilities"),
        status: row.get("status"),
        metadata: row.get("metadata"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

#[derive(Deserialize)]
struct ListQuery {
    status: Option<String>,
    limit: Option<i64>,
    agent_id: Option<i32>,
    task_id: Option<i32>,
}

// ── Agents ────────────────────────────────────────────────────────────────────

async fn list_agents(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    let rows = if let Some(s) = &q.status {
        sqlx::query("SELECT * FROM agent WHERE status=$1 ORDER BY updated_at DESC LIMIT $2")
            .bind(s).bind(limit).fetch_all(&state.db).await
    } else {
        sqlx::query("SELECT * FROM agent ORDER BY updated_at DESC LIMIT $1")
            .bind(limit).fetch_all(&state.db).await
    };
    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_agent).collect::<Vec<_>>()).into_response(),
        Err(e) => { tracing::error!("list_agents: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn create_agent(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AgentCreate>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();
    let result = sqlx::query(
        "INSERT INTO agent (name, capabilities, status, metadata, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$5) RETURNING *"
    )
    .bind(&payload.name).bind(&payload.capabilities).bind(&payload.status)
    .bind(&payload.metadata).bind(now)
    .fetch_one(&state.db).await;

    match result {
        Ok(row) => (StatusCode::OK, Json(row_to_agent(&row))).into_response(),
        Err(e) if e.to_string().contains("unique") || e.to_string().contains("duplicate") => {
            (StatusCode::CONFLICT, Json(serde_json::json!({"detail": "Agent name already exists"}))).into_response()
        }
        Err(e) => { tracing::error!("create_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i32>,
) -> impl IntoResponse {
    match sqlx::query("SELECT * FROM agent WHERE id=$1").bind(agent_id).fetch_optional(&state.db).await {
        Ok(Some(row)) => Json(row_to_agent(&row)).into_response(),
        Ok(None) => not_found("Agent not found"),
        Err(e) => { tracing::error!("get_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn update_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i32>,
    Json(payload): Json<AgentUpdate>,
) -> impl IntoResponse {
    let existing = sqlx::query("SELECT * FROM agent WHERE id=$1")
        .bind(agent_id).fetch_optional(&state.db).await;
    let agent = match existing {
        Ok(Some(r)) => row_to_agent(&r),
        Ok(None) => return not_found("Agent not found"),
        Err(e) => { tracing::error!("update_agent fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let name         = payload.name.unwrap_or(agent.name);
    let capabilities = payload.capabilities.unwrap_or(agent.capabilities);
    let status       = payload.status.unwrap_or(agent.status);
    let metadata     = payload.metadata.unwrap_or(agent.metadata);
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "UPDATE agent SET name=$2, capabilities=$3, status=$4, metadata=$5, updated_at=$6 WHERE id=$1 RETURNING *"
    )
    .bind(agent_id).bind(&name).bind(&capabilities).bind(&status).bind(&metadata).bind(now)
    .fetch_one(&state.db).await {
        Ok(row) => Json(row_to_agent(&row)).into_response(),
        Err(e) => { tracing::error!("update_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

// ── Sessions ──────────────────────────────────────────────────────────────────

async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i32>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(200);
    match sqlx::query_as::<_, AgentSession>(
        "SELECT id, agent_id, context, started_at, ended_at, claude_session_id, end_reason, audit_log \
         FROM agentsession WHERE agent_id=$1 ORDER BY started_at DESC LIMIT $2"
    )
    .bind(agent_id).bind(limit).fetch_all(&state.db).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => { tracing::error!("list_sessions: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn start_session(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i32>,
    Json(payload): Json<SessionCreate>,
) -> impl IntoResponse {
    let agent_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM agent WHERE id=$1")
        .bind(agent_id).fetch_optional(&state.db).await.unwrap_or(None);
    if agent_exists.is_none() { return not_found("Agent not found"); }

    let now = Utc::now().naive_utc();
    let _ = sqlx::query("UPDATE agent SET status='online', updated_at=$2 WHERE id=$1")
        .bind(agent_id).bind(now).execute(&state.db).await;

    match sqlx::query_as::<_, AgentSession>(
        "INSERT INTO agentsession (agent_id, context, started_at) VALUES ($1,$2,$3) \
         RETURNING id, agent_id, context, started_at, ended_at, claude_session_id, end_reason, audit_log"
    )
    .bind(agent_id).bind(&payload.context).bind(now)
    .fetch_one(&state.db).await {
        Ok(s) => (StatusCode::OK, Json(s)).into_response(),
        Err(e) => { tracing::error!("start_session: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn end_session(
    State(state): State<Arc<AppState>>,
    Path((agent_id, session_id)): Path<(i32, i32)>,
) -> impl IntoResponse {
    let agent_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM agent WHERE id=$1")
        .bind(agent_id).fetch_optional(&state.db).await.unwrap_or(None);
    if agent_exists.is_none() { return not_found("Agent not found"); }

    let now = Utc::now().naive_utc();
    let _ = sqlx::query("UPDATE agent SET status='offline', updated_at=$2 WHERE id=$1")
        .bind(agent_id).bind(now).execute(&state.db).await;

    match sqlx::query_as::<_, AgentSession>(
        "UPDATE agentsession SET ended_at=$3 WHERE id=$1 AND agent_id=$2 \
         RETURNING id, agent_id, context, started_at, ended_at, claude_session_id, end_reason, audit_log"
    )
    .bind(session_id).bind(agent_id).bind(now)
    .fetch_optional(&state.db).await {
        Ok(Some(s)) => Json(s).into_response(),
        Ok(None) => not_found("Session not found"),
        Err(e) => { tracing::error!("end_session: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

// ── Assignments ───────────────────────────────────────────────────────────────

async fn list_assignments(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    let mut sql = "SELECT * FROM taskassignment WHERE 1=1".to_string();
    let mut params: Vec<String> = vec![];
    if q.agent_id.is_some() { params.push(format!("agent_id=${}", params.len() + 2)); }
    if q.task_id.is_some()  { params.push(format!("task_id=${}", params.len() + 2)); }
    if q.status.is_some()   { params.push(format!("status=${}", params.len() + 2)); }
    if !params.is_empty() { sql = format!("{} AND {}", sql, params.join(" AND ")); }
    sql = format!("{} ORDER BY updated_at DESC LIMIT $1", sql);

    let mut q_builder = sqlx::query_as::<_, TaskAssignment>(&sql).bind(limit);
    if let Some(aid) = q.agent_id { q_builder = q_builder.bind(aid); }
    if let Some(tid) = q.task_id  { q_builder = q_builder.bind(tid); }
    if let Some(s)   = q.status   { q_builder = q_builder.bind(s); }
    match q_builder.fetch_all(&state.db).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => { tracing::error!("list_assignments: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn create_assignment(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AssignmentCreate>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, TaskAssignment>(
        "INSERT INTO taskassignment (task_id, agent_id, status, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$4) RETURNING *"
    )
    .bind(payload.task_id).bind(payload.agent_id).bind(&payload.status).bind(now)
    .fetch_one(&state.db).await {
        Ok(a) => (StatusCode::OK, Json(a)).into_response(),
        Err(e) => { tracing::error!("create_assignment: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn update_assignment(
    State(state): State<Arc<AppState>>,
    Path(assignment_id): Path<i32>,
    Json(payload): Json<AssignmentUpdate>,
) -> impl IntoResponse {
    let existing = sqlx::query_as::<_, TaskAssignment>("SELECT * FROM taskassignment WHERE id=$1")
        .bind(assignment_id).fetch_optional(&state.db).await;
    let a = match existing {
        Ok(Some(a)) => a,
        Ok(None) => return not_found("Assignment not found"),
        Err(e) => { tracing::error!("update_assignment: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };
    let status = payload.status.unwrap_or(a.status);
    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, TaskAssignment>(
        "UPDATE taskassignment SET status=$2, updated_at=$3 WHERE id=$1 RETURNING *"
    )
    .bind(assignment_id).bind(&status).bind(now)
    .fetch_one(&state.db).await {
        Ok(a) => Json(a).into_response(),
        Err(e) => { tracing::error!("update_assignment patch: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

// ── Messages ──────────────────────────────────────────────────────────────────

async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i32>,
    Json(payload): Json<MessageSend>,
) -> impl IntoResponse {
    let from_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM agent WHERE id=$1")
        .bind(agent_id).fetch_optional(&state.db).await.unwrap_or(None);
    if from_exists.is_none() { return not_found("Agent not found"); }
    let to_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM agent WHERE id=$1")
        .bind(payload.to_agent_id).fetch_optional(&state.db).await.unwrap_or(None);
    if to_exists.is_none() { return not_found("Recipient agent not found"); }

    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, AgentMessage>(
        "INSERT INTO agentmessage (from_agent_id, to_agent_id, content, message_type, task_id, read, created_at) \
         VALUES ($1,$2,$3,$4,$5,false,$6) RETURNING *"
    )
    .bind(agent_id).bind(payload.to_agent_id).bind(&payload.content)
    .bind(&payload.message_type).bind(payload.task_id).bind(now)
    .fetch_one(&state.db).await {
        Ok(m) => (StatusCode::OK, Json(m)).into_response(),
        Err(e) => { tracing::error!("send_message: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn list_messages(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i32>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(200);
    match sqlx::query_as::<_, AgentMessage>(
        "SELECT * FROM agentmessage WHERE from_agent_id=$1 OR to_agent_id=$1 \
         ORDER BY created_at DESC LIMIT $2"
    )
    .bind(agent_id).bind(limit).fetch_all(&state.db).await {
        Ok(msgs) => Json(msgs).into_response(),
        Err(e) => { tracing::error!("list_messages: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn get_inbox(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<i32>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(200);
    let msgs = sqlx::query_as::<_, AgentMessage>(
        "UPDATE agentmessage SET read=true \
         WHERE id IN (SELECT id FROM agentmessage WHERE to_agent_id=$1 AND read=false \
                      ORDER BY created_at ASC LIMIT $2) \
         RETURNING *"
    )
    .bind(agent_id).bind(limit).fetch_all(&state.db).await;

    match msgs {
        Ok(m) => Json(m).into_response(),
        Err(e) => { tracing::error!("get_inbox: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}
