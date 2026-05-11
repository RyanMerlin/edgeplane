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
    auth::Principal,
    models::agent::{
        Agent, AgentCreate, AgentIdent, AgentMessage, AgentSession, AgentUpdate, AssignmentCreate,
        AssignmentUpdate, MessageSend, SessionCreate, TaskAssignment,
    },
    state::AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/{agent_id}", get(get_agent).patch(update_agent).delete(delete_agent))
        .route("/agents/{agent_id}/restart", post(restart_agent))
        .route("/agents/{agent_id}/clear-context", post(clear_agent_context))
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
        public_id: row.get("public_id"),
        name: row.get("name"),
        capabilities: row.get("capabilities"),
        status: row.get("status"),
        metadata: row.get("metadata"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Generate a fresh public_id for a new agent row. Shape is
/// `{name}-{8-hex-chars}` — readable in CLI/TUI output, and unique enough
/// across delete-and-recreate cycles that a re-registered `aria-work`
/// doesn't collide with the previous one's identifier. Used only on the
/// INSERT side of `create_agent`; the UPDATE path preserves the existing
/// value because public_id is immutable after creation.
fn generate_public_id(name: &str) -> String {
    let raw = uuid::Uuid::new_v4().to_string();
    // First 8 hex chars of the UUID, dashes stripped — matches the migration
    // backfill shape (md5-substr 1,8) so tooling that scans for the format
    // doesn't have to special-case provenance.
    let suffix: String = raw.chars().filter(|c| *c != '-').take(8).collect();
    format!("{name}-{suffix}")
}

/// Upsert an agent row by name and return its `public_id`. Used by every
/// meshagent enrollment path to link a topology row to a persistent agent
/// identity: see `docs/plans/2026-05-11-agent-public-id-mc-mesh-fix.md`.
///
/// Semantics mirror `create_agent`: re-upsertting refreshes `capabilities`
/// and `updated_at`, un-archives the row, and preserves `public_id` (so the
/// wire identifier mc-mesh stores stays stable across re-enrollments).
/// Rejects reserved names (anonymous, system:*) and surfaces the row's
/// status as `offline` on first creation — runtimes flip it to `online`
/// when they actually start.
pub async fn upsert_agent_by_name(
    db: &sqlx::PgPool,
    name: &str,
    capabilities: &str,
) -> anyhow::Result<String> {
    if is_reserved_agent_name(name) {
        anyhow::bail!("reserved agent name");
    }
    let now = chrono::Utc::now().naive_utc();
    let public_id = generate_public_id(name);
    let row = sqlx::query(
        "INSERT INTO agent \
            (name, capabilities, status, metadata, created_at, updated_at, last_seen_at, public_id) \
         VALUES ($1,$2,'offline','{}',$3,$3,$3,$4) \
         ON CONFLICT (name) DO UPDATE SET \
            capabilities = EXCLUDED.capabilities, \
            updated_at   = EXCLUDED.updated_at, \
            last_seen_at = EXCLUDED.last_seen_at, \
            archived_at  = NULL \
         RETURNING public_id",
    )
    .bind(name)
    .bind(capabilities)
    .bind(now)
    .bind(&public_id)
    .fetch_one(db)
    .await?;
    Ok(row.get::<String, _>("public_id"))
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

/// Resolve an [`AgentIdent`] to the internal numeric row id, or return a
/// response (404 / 500) the caller can short-circuit on. Keeps the
/// resolve-or-render pattern out of every handler body.
async fn resolve_agent_or_404(
    ident: &AgentIdent,
    db: &sqlx::PgPool,
) -> Result<i32, axum::response::Response> {
    match ident.resolve_id(db).await {
        Ok(Some(id)) => Ok(id),
        Ok(None) => Err(not_found("Agent not found")),
        Err(e) => {
            tracing::error!("agent ident resolve {}: {e}", ident.as_display());
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// Reserved agent names that callers may not register. `anonymous` was the
/// historical sink for unauthenticated hook callers (see routes/hooks.rs);
/// the `system:` prefix is held for future controlplane-internal agents.
pub fn is_reserved_agent_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.eq_ignore_ascii_case("anonymous")
        || trimmed.starts_with("system:")
        || trimmed.starts_with("system/")
}

#[derive(Deserialize)]
struct ListQuery {
    status: Option<String>,
    limit: Option<i64>,
    agent_id: Option<i32>,
    task_id: Option<i32>,
    /// When true, list_agents returns archived rows alongside live ones. Off
    /// by default so the steady-state TUI/CLI views stay focused. Phase 1 of
    /// the agent-identity spec.
    #[serde(default)]
    include_archived: bool,
}

// ── Agents ────────────────────────────────────────────────────────────────────

async fn list_agents(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    // Archived agents are filtered out by default — callers must opt in. This
    // keeps every existing TUI/CLI view focused on live identities without
    // requiring a code change at each call site.
    let archived_clause = if q.include_archived { "" } else { " AND archived_at IS NULL" };
    let rows = if let Some(s) = &q.status {
        let sql = format!(
            "SELECT * FROM agent WHERE status=$1{archived_clause} ORDER BY updated_at DESC LIMIT $2"
        );
        sqlx::query(&sql).bind(s).bind(limit).fetch_all(&state.db).await
    } else {
        let sql = format!(
            "SELECT * FROM agent WHERE 1=1{archived_clause} ORDER BY updated_at DESC LIMIT $1"
        );
        sqlx::query(&sql).bind(limit).fetch_all(&state.db).await
    };
    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_agent).collect::<Vec<_>>()).into_response(),
        Err(e) => { tracing::error!("list_agents: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn create_agent(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Json(payload): Json<AgentCreate>,
) -> impl IntoResponse {
    if is_reserved_agent_name(&payload.name) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"detail": "Reserved agent name"})),
        )
            .into_response();
    }

    let now = Utc::now().naive_utc();
    let new_public_id = generate_public_id(&payload.name);
    // Upsert by name. Re-registering an existing agent refreshes capabilities,
    // status, metadata, last_seen_at, and un-archives the row — but preserves
    // the original `created_at` and `public_id` so cumulative-lifetime queries
    // and wire identifiers stay stable. `public_id` is bound on every call but
    // only consumed by the INSERT branch; the DO UPDATE deliberately omits it.
    // See docs/plans/mc-agents-identity-spec.md Phase 1 and
    // docs/plans/2026-05-11-agent-public-id-mc-mesh-fix.md.
    let result = sqlx::query(
        "INSERT INTO agent \
            (name, capabilities, status, metadata, created_at, updated_at, last_seen_at, public_id) \
         VALUES ($1,$2,$3,$4,$5,$5,$5,$6) \
         ON CONFLICT (name) DO UPDATE SET \
            capabilities = EXCLUDED.capabilities, \
            status       = EXCLUDED.status, \
            metadata     = EXCLUDED.metadata, \
            updated_at   = EXCLUDED.updated_at, \
            last_seen_at = EXCLUDED.last_seen_at, \
            archived_at  = NULL \
         RETURNING *"
    )
    .bind(&payload.name).bind(&payload.capabilities).bind(&payload.status)
    .bind(&payload.metadata).bind(now).bind(&new_public_id)
    .fetch_one(&state.db).await;

    match result {
        Ok(row) => (StatusCode::OK, Json(row_to_agent(&row))).into_response(),
        Err(e) => { tracing::error!("create_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn get_agent(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match sqlx::query("SELECT * FROM agent WHERE id=$1").bind(agent_id).fetch_optional(&state.db).await {
        Ok(Some(row)) => Json(row_to_agent(&row)).into_response(),
        Ok(None) => not_found("Agent not found"),
        Err(e) => { tracing::error!("get_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn delete_agent(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match sqlx::query("DELETE FROM agent WHERE id=$1")
        .bind(agent_id).execute(&state.db).await {
        Ok(r) if r.rows_affected() == 0 => not_found("Agent not found"),
        Ok(_) => (StatusCode::NO_CONTENT, ()).into_response(),
        Err(e) => { tracing::error!("delete_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

/// Mark all open sessions ended with reason="restart_requested" and set the
/// agent offline. The controlplane only signals — runtimes are responsible for
/// observing the state change and actually restarting the agent process.
async fn restart_agent(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let now = Utc::now().naive_utc();
    let _ = sqlx::query(
        "UPDATE agentsession SET ended_at=$2, end_reason='restart_requested' \
         WHERE agent_id=$1 AND ended_at IS NULL"
    ).bind(agent_id).bind(now).execute(&state.db).await;

    match sqlx::query("UPDATE agent SET status='offline', updated_at=$2 WHERE id=$1 RETURNING *")
        .bind(agent_id).bind(now).fetch_one(&state.db).await {
        Ok(row) => (StatusCode::OK, Json(row_to_agent(&row))).into_response(),
        Err(e) => { tracing::error!("restart_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

/// Stamp `metadata.last_context_clear_at` so listening runtimes can observe
/// the request and reset their own conversation state. The controlplane is
/// otherwise opaque to context content.
async fn clear_agent_context(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let existing = sqlx::query("SELECT * FROM agent WHERE id=$1")
        .bind(agent_id).fetch_optional(&state.db).await;
    let agent = match existing {
        Ok(Some(r)) => row_to_agent(&r),
        Ok(None) => return not_found("Agent not found"),
        Err(e) => { tracing::error!("clear_agent_context fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let now = Utc::now().naive_utc();
    let now_iso = Utc::now().to_rfc3339();
    // metadata is text in the schema; parse-merge-serialize so we don't clobber
    // unrelated keys other systems may have written.
    let mut meta_obj: serde_json::Map<String, serde_json::Value> =
        if agent.metadata.is_empty() {
            serde_json::Map::new()
        } else {
            serde_json::from_str::<serde_json::Value>(&agent.metadata)
                .ok()
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default()
        };
    meta_obj.insert("last_context_clear_at".into(), serde_json::Value::String(now_iso));
    let new_metadata = serde_json::Value::Object(meta_obj).to_string();

    match sqlx::query("UPDATE agent SET metadata=$2, updated_at=$3 WHERE id=$1 RETURNING *")
        .bind(agent_id).bind(&new_metadata).bind(now).fetch_one(&state.db).await {
        Ok(row) => (StatusCode::OK, Json(row_to_agent(&row))).into_response(),
        Err(e) => { tracing::error!("clear_agent_context: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn update_agent(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
    Json(payload): Json<AgentUpdate>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
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
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
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
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
    Json(payload): Json<SessionCreate>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

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
    _principal: Principal,
    Path((ident, session_id)): Path<(AgentIdent, i32)>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

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
    _principal: Principal,
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
    _principal: Principal,
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
    _principal: Principal,
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
    _principal: Principal,
    Path(from_ident): Path<AgentIdent>,
    Json(payload): Json<MessageSend>,
) -> impl IntoResponse {
    let from_id = match resolve_agent_or_404(&from_ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let to_id = match payload.to_agent_id.resolve_id(&state.db).await {
        Ok(Some(id)) => id,
        Ok(None) => return not_found("Recipient agent not found"),
        Err(e) => {
            tracing::error!("send_message resolve to_agent_id: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, AgentMessage>(
        "INSERT INTO agentmessage (from_agent_id, to_agent_id, content, message_type, task_id, read, created_at) \
         VALUES ($1,$2,$3,$4,$5,false,$6) RETURNING *"
    )
    .bind(from_id).bind(to_id).bind(&payload.content)
    .bind(&payload.message_type).bind(payload.task_id).bind(now)
    .fetch_one(&state.db).await {
        Ok(m) => (StatusCode::OK, Json(m)).into_response(),
        Err(e) => { tracing::error!("send_message: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

#[cfg(test)]
mod reserved_name_tests {
    use super::is_reserved_agent_name;

    #[test]
    fn anonymous_is_reserved_any_case() {
        assert!(is_reserved_agent_name("anonymous"));
        assert!(is_reserved_agent_name("ANONYMOUS"));
        assert!(is_reserved_agent_name("Anonymous"));
        assert!(is_reserved_agent_name("  anonymous  "));
    }

    #[test]
    fn system_prefixes_are_reserved() {
        assert!(is_reserved_agent_name("system:reaper"));
        assert!(is_reserved_agent_name("system/heartbeat"));
    }

    #[test]
    fn ordinary_names_are_not_reserved() {
        assert!(!is_reserved_agent_name("aria-operator"));
        assert!(!is_reserved_agent_name("aria-mc-engineer"));
        assert!(!is_reserved_agent_name("anonymouslab"));
        assert!(!is_reserved_agent_name("system-agent"));
    }
}

async fn list_messages(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
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
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
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
