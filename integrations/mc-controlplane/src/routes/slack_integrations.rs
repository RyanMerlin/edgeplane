use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/integrations/slack/bindings",
            post(create_binding).get(list_bindings),
        )
        .route(
            "/integrations/slack/bindings/{binding_id}",
            delete(delete_binding),
        )
        .route("/integrations/slack/events", post(slack_events))
        .route("/integrations/slack/commands", post(slack_commands))
        .route("/integrations/slack/interactions", post(slack_interactions))
}

// ---------------------------------------------------------------------------
// Crypto helpers
// ---------------------------------------------------------------------------

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(data))
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

// ---------------------------------------------------------------------------
// Slack HMAC signature verification
// ---------------------------------------------------------------------------

enum VerifyResult {
    Ok,
    Err(StatusCode, serde_json::Value),
}

fn verify_slack(headers: &HeaderMap, body: &[u8]) -> VerifyResult {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;

    let secret = std::env::var("SLACK_SIGNING_SECRET").unwrap_or_default();
    if secret.is_empty() {
        return VerifyResult::Err(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "code": "slack_signature_invalid",
                "reason": "SLACK_SIGNING_SECRET not configured"
            }),
        );
    }

    let timestamp = headers
        .get("x-slack-request-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let sig_header = headers
        .get("x-slack-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if timestamp.is_empty() || sig_header.is_empty() {
        return VerifyResult::Err(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "code": "slack_signature_invalid",
                "reason": "missing_timestamp_or_signature"
            }),
        );
    }

    let basestring = format!(
        "v0:{}:{}",
        timestamp,
        std::str::from_utf8(body).unwrap_or("")
    );

    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return VerifyResult::Err(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({
                    "code": "slack_signature_invalid",
                    "reason": "internal_hmac_error"
                }),
            )
        }
    };
    mac.update(basestring.as_bytes());
    let expected = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

    if !constant_time_eq(&expected, sig_header) {
        return VerifyResult::Err(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "code": "slack_signature_invalid",
                "reason": "hmac_mismatch"
            }),
        );
    }

    VerifyResult::Ok
}

// ---------------------------------------------------------------------------
// Idempotency helpers
// ---------------------------------------------------------------------------

fn inbound_event_key(
    event_type: &str,
    body: &[u8],
    mission_id: &str,
    channel_id: &str,
) -> String {
    let raw_hash = sha256_hex(body);
    format!("slack:{event_type}:{mission_id}:{channel_id}:{raw_hash}")
}

/// Returns `true` if the receipt was newly inserted, `false` if duplicate.
async fn record_receipt(
    db: &sqlx::PgPool,
    event_key: &str,
    event_type: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO chatinboundreceipt (provider, event_key, event_type, created_at) \
         VALUES ('slack', $1, $2, now()) \
         ON CONFLICT (event_key) DO NOTHING",
    )
    .bind(event_key)
    .bind(event_type)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

async fn binding_exists(
    db: &sqlx::PgPool,
    provider: &str,
    mission_id: &str,
    channel_id: &str,
) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM slackchannelbinding \
         WHERE provider=$1 AND mission_id=$2 AND channel_id=$3)",
    )
    .bind(provider)
    .bind(mission_id)
    .bind(channel_id)
    .fetch_one(db)
    .await
    .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Row → JSON
// ---------------------------------------------------------------------------

fn row_to_binding(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let metadata_json: String = row.get("channel_metadata_json");
    let metadata: serde_json::Value =
        serde_json::from_str(&metadata_json).unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "provider": row.get::<String, _>("provider"),
        "mission_id": row.get::<String, _>("mission_id"),
        "workspace_external_id": row.get::<String, _>("workspace_external_id"),
        "channel_id": row.get::<String, _>("channel_id"),
        "channel_name": row.get::<String, _>("channel_name"),
        "channel_metadata": metadata,
        "created_by": row.get::<String, _>("created_by"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

// ---------------------------------------------------------------------------
// Request / query types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateBindingBody {
    mission_id: String,
    channel_id: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    workspace_external_id: String,
    #[serde(default)]
    channel_name: String,
    channel_metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ListBindingsQuery {
    mission_id: String,
    #[serde(default)]
    provider: String,
    limit: Option<i64>,
}

// ---------------------------------------------------------------------------
// Binding handlers
// ---------------------------------------------------------------------------

async fn create_binding(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<CreateBindingBody>,
) -> impl IntoResponse {
    let provider = if body.provider.is_empty() {
        "slack".to_string()
    } else {
        body.provider.trim().to_lowercase()
    };

    let metadata_json = body
        .channel_metadata
        .as_ref()
        .and_then(|m| serde_json::to_string(m).ok())
        .unwrap_or_else(|| "{}".to_string());

    let now = Utc::now().naive_utc();

    // Return existing binding if present (upsert-style idempotency)
    match sqlx::query(
        "SELECT * FROM slackchannelbinding \
         WHERE provider=$1 AND mission_id=$2 AND channel_id=$3",
    )
    .bind(&provider)
    .bind(&body.mission_id)
    .bind(&body.channel_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => return Json(row_to_binding(&row)).into_response(),
        Err(e) => {
            tracing::error!("slack create_binding check: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(None) => {}
    }

    match sqlx::query(
        "INSERT INTO slackchannelbinding \
         (provider, mission_id, workspace_external_id, channel_id, channel_name, \
          channel_metadata_json, created_by, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8) RETURNING *",
    )
    .bind(&provider)
    .bind(&body.mission_id)
    .bind(&body.workspace_external_id)
    .bind(&body.channel_id)
    .bind(&body.channel_name)
    .bind(&metadata_json)
    .bind(&principal.subject)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::CREATED, Json(row_to_binding(&row))).into_response(),
        Err(e) => {
            tracing::error!("slack create_binding insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_bindings(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Query(q): Query<ListBindingsQuery>,
) -> impl IntoResponse {
    let provider = if q.provider.is_empty() {
        "slack".to_string()
    } else {
        q.provider.trim().to_lowercase()
    };
    let limit = q.limit.unwrap_or(100).min(500);

    match sqlx::query(
        "SELECT * FROM slackchannelbinding \
         WHERE provider=$1 AND mission_id=$2 \
         ORDER BY updated_at DESC LIMIT $3",
    )
    .bind(&provider)
    .bind(&q.mission_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_binding).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("slack list_bindings: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_binding(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(binding_id): Path<i32>,
) -> impl IntoResponse {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM slackchannelbinding WHERE id=$1)",
    )
    .bind(binding_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);

    if !exists {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"detail": "Slack binding not found"})),
        )
            .into_response();
    }

    match sqlx::query("DELETE FROM slackchannelbinding WHERE id=$1")
        .bind(binding_id)
        .execute(&state.db)
        .await
    {
        Ok(_) => Json(serde_json::json!({"ok": true, "deleted_id": binding_id})).into_response(),
        Err(e) => {
            tracing::error!("slack delete_binding: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Slack events webhook
// ---------------------------------------------------------------------------

async fn slack_events(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    match verify_slack(&headers, &body) {
        VerifyResult::Err(status, detail) => return (status, Json(detail)).into_response(),
        VerifyResult::Ok => {}
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"detail": "Invalid Slack payload"})),
            )
                .into_response()
        }
    };

    // Slack URL verification challenge
    if payload.get("type").and_then(|v| v.as_str()) == Some("url_verification") {
        let challenge = payload
            .get("challenge")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Json(serde_json::json!({"challenge": challenge})).into_response();
    }

    // Extract channel_id from event sub-object or top-level
    let event = payload
        .get("event")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let channel_id = event
        .get("channel")
        .and_then(|v| v.as_str())
        .or_else(|| {
            event
                .get("item")
                .and_then(|v| v.as_object())
                .and_then(|o| o.get("channel"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| payload.get("channel_id").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    let mission_id = payload
        .get("mission_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    // Binding check (only when both are present)
    if !mission_id.is_empty()
        && !channel_id.is_empty()
        && !binding_exists(&state.db, "slack", &mission_id, &channel_id).await
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "Slack channel is not bound to mission"})),
        )
            .into_response();
    }

    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("event")
        .to_string();

    let event_key = inbound_event_key(&event_type, &body, &mission_id, &channel_id);

    match record_receipt(&state.db, &event_key, "event").await {
        Ok(false) => {
            return Json(serde_json::json!({"ok": true, "duplicate": true})).into_response()
        }
        Err(e) => {
            tracing::error!("slack_events insert receipt: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(true) => {}
    }

    tracing::info!(
        event_type = %event_type,
        mission_id = %mission_id,
        channel_id = %channel_id,
        "slack event received"
    );

    Json(serde_json::json!({"ok": true})).into_response()
}

// ---------------------------------------------------------------------------
// Slack slash commands
// ---------------------------------------------------------------------------

/// Parse `application/x-www-form-urlencoded` body into a simple flat map.
/// Returns the first value for each key (Slack sends single-value fields).
fn parse_form(body: &[u8]) -> std::collections::HashMap<String, String> {
    let s = std::str::from_utf8(body).unwrap_or("");
    let mut map = std::collections::HashMap::new();
    for pair in s.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let key = urlencoding_decode(k);
            let val = urlencoding_decode(v);
            map.entry(key).or_insert(val);
        }
    }
    map
}

/// Minimal percent-decode + `+` → space (as used in form encoding).
fn urlencoding_decode(s: &str) -> String {
    let with_spaces = s.replace('+', " ");
    // Use percent_decode from std if available, otherwise manual approach
    let bytes = with_spaces.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

async fn slack_commands(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    match verify_slack(&headers, &body) {
        VerifyResult::Err(status, detail) => return (status, Json(detail)).into_response(),
        VerifyResult::Ok => {}
    }

    let form = parse_form(&body);
    let channel_id = form.get("channel_id").cloned().unwrap_or_default();
    let user_id = form.get("user_id").cloned().unwrap_or_default();
    let command = form.get("command").cloned().unwrap_or_default();
    let text = form.get("text").cloned().unwrap_or_default();
    // mission_id may be passed as a form field or embedded in text as `mission_id=<id>`
    let mission_id = form
        .get("mission_id")
        .cloned()
        .unwrap_or_default()
        .trim()
        .to_string();
    let mission_id = if mission_id.is_empty() {
        extract_kv_mission_id(&text)
    } else {
        mission_id
    };

    // Binding check
    if !mission_id.is_empty()
        && !channel_id.is_empty()
        && !binding_exists(&state.db, "slack", &mission_id, &channel_id).await
    {
        let msg = format!(
            "Channel `{channel_id}` is not bound to mission `{mission_id}`."
        );
        return Json(slack_ephemeral_response(&msg)).into_response();
    }

    let event_key = inbound_event_key("command", &body, &mission_id, &channel_id);
    match record_receipt(&state.db, &event_key, "command").await {
        Ok(false) => {
            return Json(slack_ephemeral_response(
                "Duplicate command received. Ignoring replay.",
            ))
            .into_response()
        }
        Err(e) => {
            tracing::error!("slack_commands insert receipt: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(true) => {}
    }

    tracing::info!(
        command = %command,
        user_id = %user_id,
        channel_id = %channel_id,
        mission_id = %mission_id,
        "slack command received"
    );

    Json(slack_ephemeral_response(
        "Command received. Supported: task.create, approval.request, search",
    ))
    .into_response()
}

/// Extract `mission_id=<value>` from a free-text command string.
fn extract_kv_mission_id(text: &str) -> String {
    for token in text.split_whitespace() {
        if let Some(v) = token.strip_prefix("mission_id=") {
            return v.trim().to_string();
        }
    }
    String::new()
}

fn slack_ephemeral_response(text: &str) -> serde_json::Value {
    serde_json::json!({
        "response_type": "ephemeral",
        "text": text,
    })
}

// ---------------------------------------------------------------------------
// Slack block interactions
// ---------------------------------------------------------------------------

async fn slack_interactions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    match verify_slack(&headers, &body) {
        VerifyResult::Err(status, detail) => return (status, Json(detail)).into_response(),
        VerifyResult::Ok => {}
    }

    // Interactions come as `application/x-www-form-urlencoded` with a `payload` field
    let form = parse_form(&body);
    let raw_payload = form
        .get("payload")
        .cloned()
        .unwrap_or_else(|| "{}".to_string());

    let payload: serde_json::Value = match serde_json::from_str(&raw_payload) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"detail": "Invalid Slack interaction payload"})),
            )
                .into_response()
        }
    };

    let channel_id = payload
        .get("channel")
        .and_then(|v| v.as_object())
        .and_then(|o| o.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mission_id = payload
        .get("mission_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    // Binding check
    if !mission_id.is_empty()
        && !channel_id.is_empty()
        && !binding_exists(&state.db, "slack", &mission_id, &channel_id).await
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "Slack channel is not bound to mission"})),
        )
            .into_response();
    }

    let interaction_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("interaction")
        .to_string();

    let event_key = inbound_event_key(&interaction_type, body.as_ref(), &mission_id, &channel_id);
    match record_receipt(&state.db, &event_key, "interaction").await {
        Ok(false) => {
            return Json(serde_json::json!({"ok": true, "duplicate": true})).into_response()
        }
        Err(e) => {
            tracing::error!("slack_interactions insert receipt: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(true) => {}
    }

    tracing::info!(
        interaction_type = %interaction_type,
        channel_id = %channel_id,
        mission_id = %mission_id,
        "slack interaction received"
    );

    Json(serde_json::json!({"ok": true})).into_response()
}
