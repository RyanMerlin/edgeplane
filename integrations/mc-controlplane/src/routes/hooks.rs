use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use chrono::Utc;
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Claude Code hooks
        .route("/hooks/claude/session-start", post(claude_session_start))
        .route("/hooks/claude/session-end", post(claude_session_end))
        .route("/hooks/claude/tool-audit", post(claude_tool_audit))
        // Codex hooks (same logic, different capability label)
        .route("/hooks/codex/session-start", post(codex_session_start))
        .route("/hooks/codex/session-end", post(codex_session_end))
        .route("/hooks/codex/tool-audit", post(codex_tool_audit))
}

// ── Claude ────────────────────────────────────────────────────────────────────

async fn claude_session_start(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    session_start(state, principal.subject, payload, "claude-code").await
}

async fn claude_session_end(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    session_end(state, principal.subject, payload).await
}

async fn claude_tool_audit(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    tool_audit(state, principal.subject, payload).await
}

// ── Codex ─────────────────────────────────────────────────────────────────────

async fn codex_session_start(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    session_start(state, principal.subject, payload, "codex-sdk").await
}

async fn codex_session_end(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    session_end(state, principal.subject, payload).await
}

async fn codex_tool_audit(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    tool_audit(state, principal.subject, payload).await
}

// ── Shared logic ──────────────────────────────────────────────────────────────

async fn session_start(
    state: Arc<AppState>,
    subject: String,
    payload: serde_json::Value,
    capability: &str,
) -> impl IntoResponse {
    let session_id = payload.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let source = payload
        .get("source")
        .or_else(|| payload.get("hook_event_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let now = Utc::now().naive_utc();

    // Find or create the agent row for this subject
    let agent_id: i32 = match find_or_create_agent(&state, &subject, capability).await {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("session_start find_or_create_agent: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    };

    // Create an AgentSession if we have a session_id
    if !session_id.is_empty() {
        let existing: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM agentsession WHERE claude_session_id=$1 AND ended_at IS NULL",
        )
        .bind(&session_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();

        if existing.is_none() {
            let _ = sqlx::query(
                "INSERT INTO agentsession (agent_id, context, claude_session_id, started_at) \
                 VALUES ($1,$2,$3,$4)",
            )
            .bind(agent_id)
            .bind(&source)
            .bind(&session_id)
            .bind(now)
            .execute(&state.db)
            .await;

            let _ = sqlx::query("UPDATE agent SET status='online', updated_at=$2 WHERE id=$1")
                .bind(agent_id)
                .bind(now)
                .execute(&state.db)
                .await;
        }
    }

    let body = format!(
        "[MC Session — {session_id}]\nAgent: {subject}\nSource: {source}\nRegistered: {ts}\nAgent ID: {agent_id}\nCapabilities: {capability}",
        session_id = if session_id.is_empty() { "unknown".to_string() } else { session_id },
        ts = now.format("%Y-%m-%dT%H:%M:%SZ"),
    );

    ([(header::CONTENT_TYPE, "text/plain")], body).into_response()
}

async fn session_end(
    state: Arc<AppState>,
    subject: String,
    payload: serde_json::Value,
) -> impl IntoResponse {
    let session_id = payload.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let end_reason = payload
        .get("source")
        .or_else(|| payload.get("hook_event_name"))
        .and_then(|v| v.as_str())
        .unwrap_or("session_end")
        .to_string();
    let now = Utc::now().naive_utc();

    if let Ok(agent_id) = find_or_create_agent(&state, &subject, "").await {
        if !session_id.is_empty() {
            let result = sqlx::query(
                "UPDATE agentsession SET ended_at=$3, end_reason=$4 \
                 WHERE claude_session_id=$1 AND agent_id=$2 AND ended_at IS NULL",
            )
            .bind(&session_id)
            .bind(agent_id)
            .bind(now)
            .bind(&end_reason)
            .execute(&state.db)
            .await;

            if result.map(|r| r.rows_affected()).unwrap_or(0) > 0 {
                let _ = sqlx::query("UPDATE agent SET status='offline', updated_at=$2 WHERE id=$1")
                    .bind(agent_id)
                    .bind(now)
                    .execute(&state.db)
                    .await;
            }
        }
    }

    Json(serde_json::json!({"ok": true}))
}

async fn tool_audit(
    state: Arc<AppState>,
    subject: String,
    payload: serde_json::Value,
) -> impl IntoResponse {
    let session_id = payload.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let tool_name = payload.get("tool_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let tool_input = payload.get("tool_input").cloned().unwrap_or(serde_json::json!({}));

    let entry = serde_json::json!({
        "ts": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "tool": tool_name,
        "input_summary": serde_json::to_string(&tool_input)
            .unwrap_or_default()
            .chars()
            .take(512)
            .collect::<String>(),
    });
    let entry_line = format!("{}\n", serde_json::to_string(&entry).unwrap_or_default());

    if let Ok(agent_id) = find_or_create_agent(&state, &subject, "").await {
        if !session_id.is_empty() {
            let _ = sqlx::query(
                "UPDATE agentsession SET audit_log = COALESCE(audit_log,'') || $3 \
                 WHERE claude_session_id=$1 AND agent_id=$2 AND ended_at IS NULL",
            )
            .bind(&session_id)
            .bind(agent_id)
            .bind(&entry_line)
            .execute(&state.db)
            .await;
        }
    }

    Json(serde_json::json!({"ok": true}))
}

/// Return the id of the agent with this subject name, creating it if absent.
async fn find_or_create_agent(state: &Arc<AppState>, subject: &str, capability: &str) -> Result<i32, sqlx::Error> {
    if let Some(row) =
        sqlx::query("SELECT id FROM agent WHERE name=$1")
            .bind(subject)
            .fetch_optional(&state.db)
            .await?
    {
        return Ok(row.get("id"));
    }
    let now = Utc::now().naive_utc();
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO agent (name, capabilities, status, metadata, created_at, updated_at) \
         VALUES ($1,$2,'offline','{}', $3,$3) \
         ON CONFLICT (name) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .bind(subject)
    .bind(if capability.is_empty() { "unknown" } else { capability })
    .bind(now)
    .fetch_one(&state.db)
    .await?;
    Ok(id)
}
