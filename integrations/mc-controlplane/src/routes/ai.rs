use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{sse::Event, sse::Sse, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{auth::Principal, state::AppState};

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ai/runtime-capabilities", get(runtime_capabilities))
        .route("/ai/sessions", post(create_session).get(list_sessions))
        .route("/ai/sessions/{id}", get(get_session))
        .route("/ai/sessions/{id}/turns", post(create_turn))
        .route("/ai/sessions/{id}/actions/{action_id}/approve", post(approve_action))
        .route("/ai/sessions/{id}/actions/{action_id}/reject", post(reject_action))
        .route("/ai/sessions/{id}/stream", get(stream_events))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"detail": "Authentication required"}))).into_response()
}

fn not_found(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn unprocessable(msg: &str) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn require_auth(principal: &Principal) -> Option<Response> {
    if principal.auth_type == "anonymous" {
        Some(unauthorized())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Session serialization helpers
// ---------------------------------------------------------------------------

fn parse_json_field(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
}

fn row_to_turn(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let id: i64 = row.get::<i32, _>("id") as i64;
    let role: String = row.get("role");
    let content_json: String = row.get("content_json");
    let created_at: chrono::NaiveDateTime = row.get("created_at");
    serde_json::json!({
        "id": id,
        "role": role,
        "content": parse_json_field(&content_json),
        "created_at": created_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
    })
}

fn row_to_event(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let id: i64 = row.get::<i32, _>("id") as i64;
    let turn_id: Option<i32> = row.try_get("turn_id").ok().flatten();
    let event_type: String = row.get("event_type");
    let payload_json: String = row.get("payload_json");
    let created_at: chrono::NaiveDateTime = row.get("created_at");
    serde_json::json!({
        "id": id,
        "turn_id": turn_id,
        "event_type": event_type,
        "payload": parse_json_field(&payload_json),
        "created_at": created_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
    })
}

fn row_to_pending_action(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let id: String = row.get("id");
    let tool: String = row.get("tool");
    let args_json: String = row.get("args_json");
    let reason: String = row.get("reason");
    let status: String = row.get("status");
    let requested_by: String = row.get("requested_by");
    let approved_by: String = row.get("approved_by");
    let rejected_by: String = row.get("rejected_by");
    let rejection_note: String = row.get("rejection_note");
    let created_at: chrono::NaiveDateTime = row.get("created_at");
    let updated_at: chrono::NaiveDateTime = row.get("updated_at");
    serde_json::json!({
        "id": id,
        "tool": tool,
        "args": parse_json_field(&args_json),
        "reason": reason,
        "status": status,
        "requested_by": requested_by,
        "approved_by": approved_by,
        "rejected_by": rejected_by,
        "rejection_note": rejection_note,
        "created_at": created_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
        "updated_at": updated_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
    })
}

/// Build a full session JSON object fetching associated turns/events/pending.
async fn serialize_session(
    db: &sqlx::PgPool,
    session_row: &sqlx::postgres::PgRow,
) -> serde_json::Value {
    let id: String = session_row.get("id");
    let owner_subject: String = session_row.get("owner_subject");
    let title: String = session_row.get("title");
    let status: String = session_row.get("status");
    let runtime_kind: String = session_row.get("runtime_kind");
    let runtime_session_id: Option<String> = session_row.try_get("runtime_session_id").ok().flatten();
    let workspace_path: Option<String> = session_row.try_get("workspace_path").ok().flatten();
    let policy_json: String = session_row.get("policy_json");
    let capability_snapshot_json: String = session_row.get("capability_snapshot_json");
    let created_at: chrono::NaiveDateTime = session_row.get("created_at");
    let updated_at: chrono::NaiveDateTime = session_row.get("updated_at");

    let turns = sqlx::query(
        "SELECT id, role, content_json, created_at FROM aiturn WHERE session_id=$1 ORDER BY id ASC"
    )
    .bind(&id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
    .iter()
    .map(row_to_turn)
    .collect::<Vec<_>>();

    let events = sqlx::query(
        "SELECT id, turn_id, event_type, payload_json, created_at FROM aievent WHERE session_id=$1 ORDER BY id ASC"
    )
    .bind(&id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
    .iter()
    .map(row_to_event)
    .collect::<Vec<_>>();

    let pending_actions = sqlx::query(
        "SELECT id, tool, args_json, reason, status, requested_by, approved_by, rejected_by, rejection_note, created_at, updated_at \
         FROM aipendingaction WHERE session_id=$1 ORDER BY created_at ASC"
    )
    .bind(&id)
    .fetch_all(db)
    .await
    .unwrap_or_default()
    .iter()
    .map(row_to_pending_action)
    .collect::<Vec<_>>();

    serde_json::json!({
        "id": id,
        "owner_subject": owner_subject,
        "title": title,
        "status": status,
        "runtime_kind": runtime_kind,
        "runtime_session_id": runtime_session_id,
        "workspace_path": workspace_path,
        "capability_snapshot": parse_json_field(&capability_snapshot_json),
        "policy": parse_json_field(&policy_json),
        "turns": turns,
        "events": events,
        "pending_actions": pending_actions,
        "created_at": created_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
        "updated_at": updated_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
    })
}

// ---------------------------------------------------------------------------
// GET /ai/runtime-capabilities
// ---------------------------------------------------------------------------

async fn runtime_capabilities(principal: Principal) -> Response {
    if let Some(r) = require_auth(&principal) { return r; }
    Json(serde_json::json!([
        {
            "runtime_kind": "claude_code",
            "display_name": "Claude Code",
            "icon_slug": "claude",
            "supports_streaming": true,
            "supports_file_workspace": true,
            "supports_tool_interception": true,
            "supports_skill_packs": true,
            "supports_session_resume": true,
            "max_context_tokens": 200000
        },
        {
            "runtime_kind": "opencode",
            "display_name": "OpenCode",
            "icon_slug": "opencode",
            "supports_streaming": true,
            "supports_file_workspace": true,
            "supports_tool_interception": true,
            "supports_skill_packs": false,
            "supports_session_resume": false,
            "max_context_tokens": 128000
        }
    ])).into_response()
}

// ---------------------------------------------------------------------------
// POST /ai/sessions
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateSessionBody {
    title: Option<String>,
    runtime_kind: Option<String>,
    policy: Option<serde_json::Value>,
}

async fn create_session(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<CreateSessionBody>,
) -> Response {
    if let Some(r) = require_auth(&principal) { return r; }

    let session_id = format!(
        "ais_{}",
        &uuid::Uuid::new_v4().to_string().replace("-", "")[..16]
    );
    let title = body.title.unwrap_or_default();
    let runtime_kind = body.runtime_kind.unwrap_or_else(|| "opencode".to_string());
    let policy_json = body.policy
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".to_string());
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        "INSERT INTO aisession \
         (id, owner_subject, title, status, runtime_kind, runtime_session_id, workspace_path, \
          policy_json, capability_snapshot_json, created_at, updated_at) \
         VALUES ($1,$2,$3,'active',$4,NULL,NULL,$5,'{}',$6,$6) \
         RETURNING id, owner_subject, title, status, runtime_kind, runtime_session_id, \
                   workspace_path, policy_json, capability_snapshot_json, created_at, updated_at"
    )
    .bind(&session_id)
    .bind(&principal.subject)
    .bind(&title)
    .bind(&runtime_kind)
    .bind(&policy_json)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => {
            let session = serialize_session(&state.db, &row).await;
            (StatusCode::OK, Json(session)).into_response()
        }
        Err(e) => {
            tracing::error!("create_session: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// GET /ai/sessions
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ListSessionsQuery {
    limit: Option<i64>,
}

async fn list_sessions(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListSessionsQuery>,
) -> Response {
    if let Some(r) = require_auth(&principal) { return r; }

    let limit = q.limit.unwrap_or(20).min(100);
    let rows = sqlx::query(
        "SELECT id, owner_subject, title, status, runtime_kind, runtime_session_id, \
                workspace_path, created_at, updated_at \
         FROM aisession WHERE owner_subject=$1 ORDER BY updated_at DESC LIMIT $2"
    )
    .bind(&principal.subject)
    .bind(limit)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => {
            let sessions: Vec<serde_json::Value> = rows.iter().map(|row| {
                let id: String = row.get("id");
                let owner_subject: String = row.get("owner_subject");
                let title: String = row.get("title");
                let status: String = row.get("status");
                let runtime_kind: String = row.get("runtime_kind");
                let runtime_session_id: Option<String> = row.try_get("runtime_session_id").ok().flatten();
                let workspace_path: Option<String> = row.try_get("workspace_path").ok().flatten();
                let created_at: chrono::NaiveDateTime = row.get("created_at");
                let updated_at: chrono::NaiveDateTime = row.get("updated_at");
                serde_json::json!({
                    "id": id,
                    "owner_subject": owner_subject,
                    "title": title,
                    "status": status,
                    "runtime_kind": runtime_kind,
                    "runtime_session_id": runtime_session_id,
                    "workspace_path": workspace_path,
                    "turns": [],
                    "events": [],
                    "pending_actions": [],
                    "created_at": created_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
                    "updated_at": updated_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
                })
            }).collect();
            Json(sessions).into_response()
        }
        Err(e) => {
            tracing::error!("list_sessions: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// GET /ai/sessions/{id}
// ---------------------------------------------------------------------------

async fn get_session(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<String>,
) -> Response {
    if let Some(r) = require_auth(&principal) { return r; }

    let row = sqlx::query(
        "SELECT id, owner_subject, title, status, runtime_kind, runtime_session_id, \
                workspace_path, policy_json, capability_snapshot_json, created_at, updated_at \
         FROM aisession WHERE id=$1 AND owner_subject=$2"
    )
    .bind(&id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await;

    match row {
        Ok(Some(row)) => {
            let session = serialize_session(&state.db, &row).await;
            Json(session).into_response()
        }
        Ok(None) => not_found("AI session not found"),
        Err(e) => {
            tracing::error!("get_session: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// POST /ai/sessions/{id}/turns
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateTurnBody {
    message: Option<String>,
}

async fn create_turn(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<String>,
    Json(body): Json<CreateTurnBody>,
) -> Response {
    if let Some(r) = require_auth(&principal) { return r; }

    let message = body.message.unwrap_or_default();
    if message.trim().is_empty() {
        return unprocessable("message must not be empty");
    }

    // Verify session exists and is owned by caller
    let session_exists: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM aisession WHERE id=$1 AND owner_subject=$2"
    )
    .bind(&id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if session_exists.is_none() {
        return not_found("AI session not found");
    }

    let content_json = serde_json::json!({"text": message}).to_string();
    let now = Utc::now().naive_utc();

    // Insert turn
    let turn_row = sqlx::query(
        "INSERT INTO aiturn (session_id, role, content_json, created_at) \
         VALUES ($1,'user',$2,$3) RETURNING id"
    )
    .bind(&id)
    .bind(&content_json)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    let turn_id: i32 = match turn_row {
        Ok(row) => row.get("id"),
        Err(e) => {
            tracing::error!("create_turn insert turn: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Insert event
    let payload_json = serde_json::json!({"text": message}).to_string();
    let event_res = sqlx::query(
        "INSERT INTO aievent (session_id, turn_id, event_type, payload_json, created_at) \
         VALUES ($1,$2,'user_message',$3,$4)"
    )
    .bind(&id)
    .bind(turn_id)
    .bind(&payload_json)
    .bind(now)
    .execute(&state.db)
    .await;

    if let Err(e) = event_res {
        tracing::error!("create_turn insert event: {e}");
    }

    // Update session updated_at
    let _ = sqlx::query("UPDATE aisession SET updated_at=$2 WHERE id=$1")
        .bind(&id)
        .bind(now)
        .execute(&state.db)
        .await;

    // Return full session
    let session_row = sqlx::query(
        "SELECT id, owner_subject, title, status, runtime_kind, runtime_session_id, \
                workspace_path, policy_json, capability_snapshot_json, created_at, updated_at \
         FROM aisession WHERE id=$1"
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await;

    match session_row {
        Ok(row) => {
            let session = serialize_session(&state.db, &row).await;
            (StatusCode::OK, Json(session)).into_response()
        }
        Err(e) => {
            tracing::error!("create_turn fetch session: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// POST /ai/sessions/{id}/actions/{action_id}/approve
// ---------------------------------------------------------------------------

async fn approve_action(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((id, action_id)): Path<(String, String)>,
) -> Response {
    if let Some(r) = require_auth(&principal) { return r; }

    // Verify session ownership
    let session_exists: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM aisession WHERE id=$1 AND owner_subject=$2"
    )
    .bind(&id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if session_exists.is_none() {
        return not_found("AI session not found");
    }

    // Check action exists and is pending
    let action_row = sqlx::query(
        "SELECT id, status FROM aipendingaction WHERE id=$1 AND session_id=$2"
    )
    .bind(&action_id)
    .bind(&id)
    .fetch_optional(&state.db)
    .await;

    match action_row {
        Ok(None) => return not_found("Pending action not found"),
        Err(e) => {
            tracing::error!("approve_action fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(row)) => {
            let status: String = row.get("status");
            if status != "pending" {
                return unprocessable("Action is not in pending status");
            }
        }
    }

    let now = Utc::now().naive_utc();

    // Update action status
    let upd = sqlx::query(
        "UPDATE aipendingaction SET status='executed', approved_by=$2, updated_at=$3 WHERE id=$1"
    )
    .bind(&action_id)
    .bind(&principal.subject)
    .bind(now)
    .execute(&state.db)
    .await;

    if let Err(e) = upd {
        tracing::error!("approve_action update: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Insert approval event
    let payload = serde_json::json!({"action_id": action_id, "status": "approved"}).to_string();
    let _ = sqlx::query(
        "INSERT INTO aievent (session_id, turn_id, event_type, payload_json, created_at) \
         VALUES ($1,NULL,'approval_outcome',$2,$3)"
    )
    .bind(&id)
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await;

    let session_row = sqlx::query(
        "SELECT id, owner_subject, title, status, runtime_kind, runtime_session_id, \
                workspace_path, policy_json, capability_snapshot_json, created_at, updated_at \
         FROM aisession WHERE id=$1"
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await;

    match session_row {
        Ok(row) => {
            let session = serialize_session(&state.db, &row).await;
            Json(session).into_response()
        }
        Err(e) => {
            tracing::error!("approve_action fetch session: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// POST /ai/sessions/{id}/actions/{action_id}/reject
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RejectBody {
    note: Option<String>,
}

async fn reject_action(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((id, action_id)): Path<(String, String)>,
    Json(body): Json<RejectBody>,
) -> Response {
    if let Some(r) = require_auth(&principal) { return r; }

    // Verify session ownership
    let session_exists: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM aisession WHERE id=$1 AND owner_subject=$2"
    )
    .bind(&id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if session_exists.is_none() {
        return not_found("AI session not found");
    }

    // Check action exists and is pending
    let action_row = sqlx::query(
        "SELECT id, status FROM aipendingaction WHERE id=$1 AND session_id=$2"
    )
    .bind(&action_id)
    .bind(&id)
    .fetch_optional(&state.db)
    .await;

    match action_row {
        Ok(None) => return not_found("Pending action not found"),
        Err(e) => {
            tracing::error!("reject_action fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(row)) => {
            let status: String = row.get("status");
            if status != "pending" {
                return unprocessable("Action is not in pending status");
            }
        }
    }

    let note = body.note.unwrap_or_default();
    let now = Utc::now().naive_utc();

    // Update action status
    let upd = sqlx::query(
        "UPDATE aipendingaction SET status='rejected', rejected_by=$2, rejection_note=$3, updated_at=$4 WHERE id=$1"
    )
    .bind(&action_id)
    .bind(&principal.subject)
    .bind(&note)
    .bind(now)
    .execute(&state.db)
    .await;

    if let Err(e) = upd {
        tracing::error!("reject_action update: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Insert rejection event
    let payload = serde_json::json!({"action_id": action_id, "status": "rejected", "note": note}).to_string();
    let _ = sqlx::query(
        "INSERT INTO aievent (session_id, turn_id, event_type, payload_json, created_at) \
         VALUES ($1,NULL,'approval_outcome',$2,$3)"
    )
    .bind(&id)
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await;

    let session_row = sqlx::query(
        "SELECT id, owner_subject, title, status, runtime_kind, runtime_session_id, \
                workspace_path, policy_json, capability_snapshot_json, created_at, updated_at \
         FROM aisession WHERE id=$1"
    )
    .bind(&id)
    .fetch_one(&state.db)
    .await;

    match session_row {
        Ok(row) => {
            let session = serialize_session(&state.db, &row).await;
            Json(session).into_response()
        }
        Err(e) => {
            tracing::error!("reject_action fetch session: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// GET /ai/sessions/{id}/stream  — SSE
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StreamQuery {
    after_id: Option<i64>,
}

async fn stream_events(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<String>,
    Query(q): Query<StreamQuery>,
) -> Response {
    if let Some(r) = require_auth(&principal) { return r; }

    // Verify session ownership
    let session_exists: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM aisession WHERE id=$1 AND owner_subject=$2"
    )
    .bind(&id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if session_exists.is_none() {
        return not_found("AI session not found");
    }

    let after_id = q.after_id.unwrap_or(0);
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(32);
    let db_clone = state.db.clone();
    let sid = id.clone();

    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
        let mut last_id: i64 = after_id;

        loop {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;

            let rows = sqlx::query(
                "SELECT id, turn_id, event_type, payload_json, created_at \
                 FROM aievent WHERE session_id=$1 AND id > $2 ORDER BY id ASC LIMIT 100"
            )
            .bind(&sid)
            .bind(last_id)
            .fetch_all(&db_clone)
            .await;

            if let Ok(rows) = rows {
                for row in &rows {
                    let id: i32 = row.get("id");
                    let id64 = id as i64;
                    if id64 > last_id {
                        last_id = id64;
                    }
                    let data = row_to_event(row);
                    let evt = Event::default()
                        .id(id64.to_string())
                        .event("ai_event")
                        .data(data.to_string());
                    if tx.send(Ok(evt)).await.is_err() {
                        return;
                    }
                }
            }
        }

        // Send a done event so clients know the stream ended
        let done_evt = Event::default().event("done").data("{}");
        let _ = tx.send(Ok(done_evt)).await;
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream).into_response()
}
