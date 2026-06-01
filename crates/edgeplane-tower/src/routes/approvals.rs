use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{NaiveDateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;

use crate::{
    auth::Principal,
    models::approval::{ApprovalListQuery, ApprovalRequestCreate, ApprovalRespond},
    state::AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/approvals/requests", post(create_approval_request))
        .route("/approvals", get(list_approvals))
        .route("/approvals/{approval_id}/respond", post(respond_approval))
        // Direct approve/reject mirrors for Python API compatibility
        .route("/approvals/{approval_id}/approve", post(approve_approval))
        .route("/approvals/{approval_id}/reject", post(reject_approval))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn conflict(msg: &str) -> axum::response::Response {
    (StatusCode::CONFLICT, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn clamp_expires(seconds: Option<i64>) -> i64 {
    seconds.unwrap_or(900).clamp(60, 86400)
}

/// Generates an HMAC-SHA256 signed approval token compatible with the Python backend.
/// Format: base64url(payload_json).base64url(hmac_sig)
/// Secret from EP_APPROVAL_TOKEN_SECRET env var.
fn generate_approval_token(payload: &serde_json::Value) -> Result<String, String> {
    let secret = std::env::var("EP_APPROVAL_TOKEN_SECRET")
        .map_err(|_| "EP_APPROVAL_TOKEN_SECRET not set".to_string())?;
    if secret.trim().is_empty() {
        return Err("EP_APPROVAL_TOKEN_SECRET is empty".to_string());
    }
    let payload_json = serde_json::to_string(payload)
        .map_err(|e| e.to_string())?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.trim().as_bytes())
        .map_err(|e| e.to_string())?;
    mac.update(payload_b64.as_bytes());
    let sig = mac.finalize().into_bytes();
    let sig_b64 = URL_SAFE_NO_PAD.encode(sig);

    Ok(format!("{payload_b64}.{sig_b64}"))
}

fn parse_context(json_str: &str) -> serde_json::Value {
    serde_json::from_str(json_str).unwrap_or(serde_json::json!({}))
}

fn row_to_json(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    use sqlx::Row;
    let context_str: String = row.try_get("request_context_json").unwrap_or_default();
    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "domain_id": row.get::<String, _>("domain_id"),
        "action": row.get::<String, _>("action"),
        "channel": row.get::<String, _>("channel"),
        "reason": row.get::<String, _>("reason"),
        "target_entity_type": row.get::<String, _>("target_entity_type"),
        "target_entity_id": row.get::<String, _>("target_entity_id"),
        "request_context": parse_context(&context_str),
        "status": row.get::<String, _>("status"),
        "requested_by": row.get::<String, _>("requested_by"),
        "approved_by": row.get::<String, _>("approved_by"),
        "rejected_by": row.get::<String, _>("rejected_by"),
        "decision_note": row.get::<String, _>("decision_note"),
        "approval_expires_at": row.try_get::<Option<NaiveDateTime>, _>("approval_expires_at").ok().flatten(),
        "approved_at": row.try_get::<Option<NaiveDateTime>, _>("approved_at").ok().flatten(),
        "rejected_at": row.try_get::<Option<NaiveDateTime>, _>("rejected_at").ok().flatten(),
        "executed_at": row.try_get::<Option<NaiveDateTime>, _>("executed_at").ok().flatten(),
        "executed_action": row.get::<String, _>("executed_action"),
        "executed_request_id": row.get::<String, _>("executed_request_id"),
        "created_at": row.get::<NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<NaiveDateTime, _>("updated_at"),
    })
}

// ── handlers ──────────────────────────────────────────────────────────────────

async fn create_approval_request(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<ApprovalRequestCreate>,
) -> impl IntoResponse {
    let expires_secs = clamp_expires(payload.expires_in_seconds);
    let now = Utc::now().naive_utc();
    let expires_at = now + chrono::Duration::seconds(expires_secs);
    let context_json = serde_json::to_string(
        payload.request_context.as_ref().unwrap_or(&serde_json::json!({})),
    )
    .unwrap_or_default();

    let row = sqlx::query(
        "INSERT INTO approvalrequest \
         (domain_id, action, channel, reason, target_entity_type, target_entity_id, \
          request_context_json, status, requested_by, approved_by, rejected_by, \
          decision_note, approval_nonce, approval_expires_at, created_at, updated_at, \
          executed_action, executed_request_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,'pending',$8,'','','','',$9,$10,$10,'','') \
         RETURNING *",
    )
    .bind(&payload.domain_id)
    .bind(payload.action.trim())
    .bind(payload.channel.as_deref().unwrap_or("api").trim().to_string())
    .bind(payload.reason.as_deref().unwrap_or(""))
    .bind(payload.target_entity_type.as_deref().unwrap_or(""))
    .bind(payload.target_entity_id.as_deref().unwrap_or(""))
    .bind(&context_json)
    .bind(&principal.subject)
    .bind(expires_at)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(r) => (StatusCode::CREATED, Json(row_to_json(&r))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"detail": e.to_string()})),
        )
            .into_response(),
    }
}

async fn list_approvals(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ApprovalListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(100).max(1);
    let rows = match (&q.domain_id, &q.status) {
        (Some(mid), Some(status)) => sqlx::query(
            "SELECT * FROM approvalrequest WHERE domain_id=$1 AND status=$2 \
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(mid)
        .bind(status)
        .bind(limit)
        .fetch_all(&state.db)
        .await,
        (Some(mid), None) => sqlx::query(
            "SELECT * FROM approvalrequest WHERE domain_id=$1 \
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(mid)
        .bind(limit)
        .fetch_all(&state.db)
        .await,
        (None, Some(status)) => sqlx::query(
            "SELECT * FROM approvalrequest WHERE status=$1 \
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(status)
        .bind(limit)
        .fetch_all(&state.db)
        .await,
        (None, None) => sqlx::query(
            "SELECT * FROM approvalrequest ORDER BY created_at DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&state.db)
        .await,
    };

    match rows {
        Ok(rs) => Json(rs.iter().map(row_to_json).collect::<Vec<_>>()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"detail": e.to_string()})),
        )
            .into_response(),
    }
}

async fn respond_approval(
    State(state): State<Arc<AppState>>,
    Path(approval_id): Path<i32>,
    principal: Principal,
    Json(payload): Json<ApprovalRespond>,
) -> impl IntoResponse {
    match payload.decision.as_str() {
        "approve" | "approved" => {
            do_approve(&state, approval_id, &principal.subject, payload.note.as_deref(), payload.expires_in_seconds).await
        }
        "reject" | "rejected" => {
            do_reject(&state, approval_id, &principal.subject, payload.note.as_deref()).await
        }
        other => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"detail": format!("unknown decision '{other}'; use 'approve' or 'reject'")})),
        )
            .into_response(),
    }
}

async fn approve_approval(
    State(state): State<Arc<AppState>>,
    Path(approval_id): Path<i32>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let note = payload.get("note").and_then(|v| v.as_str());
    let expires = payload.get("expires_in_seconds").and_then(|v| v.as_i64());
    do_approve(&state, approval_id, &principal.subject, note, expires).await
}

async fn reject_approval(
    State(state): State<Arc<AppState>>,
    Path(approval_id): Path<i32>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let note = payload.get("note").and_then(|v| v.as_str());
    do_reject(&state, approval_id, &principal.subject, note).await
}

// ── approval/rejection logic ──────────────────────────────────────────────────

async fn do_approve(
    state: &AppState,
    approval_id: i32,
    actor: &str,
    note: Option<&str>,
    expires_in_seconds: Option<i64>,
) -> axum::response::Response {
    let existing = sqlx::query("SELECT * FROM approvalrequest WHERE id=$1")
        .bind(approval_id)
        .fetch_optional(&state.db)
        .await;

    let row = match existing {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("approval request not found"),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"detail": e.to_string()}))).into_response(),
    };

    use sqlx::Row;
    let status: String = row.get("status");
    if status != "pending" {
        return conflict("only pending requests can be approved");
    }

    let expires_secs = clamp_expires(expires_in_seconds);
    let now = Utc::now().naive_utc();
    let expires_at = now + chrono::Duration::seconds(expires_secs);
    let nonce = uuid::Uuid::new_v4().to_string();

    let updated = sqlx::query(
        "UPDATE approvalrequest SET status='approved', approved_by=$2, approved_at=$3, \
         decision_note=$4, approval_nonce=$5, approval_expires_at=$6, updated_at=$3 \
         WHERE id=$1 RETURNING *",
    )
    .bind(approval_id)
    .bind(actor)
    .bind(now)
    .bind(note.unwrap_or(""))
    .bind(&nonce)
    .bind(expires_at)
    .fetch_one(&state.db)
    .await;

    let updated_row = match updated {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"detail": e.to_string()}))).into_response(),
    };

    let domain_id: String = updated_row.get("domain_id");
    let action: String = updated_row.get("action");
    let token_payload = serde_json::json!({
        "approval_request_id": approval_id,
        "domain_id": domain_id,
        "action": action,
        "request_id": format!("approval-{approval_id}"),
        "approved_by": actor,
        "approved_at": now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        "nonce": nonce,
        "exp": expires_at.and_utc().timestamp(),
    });

    let token = match generate_approval_token(&token_payload) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("approval token generation failed: {e}");
            String::new()
        }
    };

    Json(serde_json::json!({
        "approval": row_to_json(&updated_row),
        "approval_token": token,
    }))
    .into_response()
}

async fn do_reject(
    state: &AppState,
    approval_id: i32,
    actor: &str,
    note: Option<&str>,
) -> axum::response::Response {
    let existing = sqlx::query("SELECT * FROM approvalrequest WHERE id=$1")
        .bind(approval_id)
        .fetch_optional(&state.db)
        .await;

    let row = match existing {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("approval request not found"),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"detail": e.to_string()}))).into_response(),
    };

    use sqlx::Row;
    let status: String = row.get("status");
    if status != "pending" {
        return conflict("only pending requests can be rejected");
    }

    let now = Utc::now().naive_utc();
    let updated = sqlx::query(
        "UPDATE approvalrequest SET status='rejected', rejected_by=$2, rejected_at=$3, \
         decision_note=$4, updated_at=$3 WHERE id=$1 RETURNING *",
    )
    .bind(approval_id)
    .bind(actor)
    .bind(now)
    .bind(note.unwrap_or(""))
    .fetch_one(&state.db)
    .await;

    match updated {
        Ok(r) => Json(row_to_json(&r)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"detail": e.to_string()}))).into_response(),
    }
}
