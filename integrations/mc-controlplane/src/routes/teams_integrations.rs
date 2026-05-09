use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use std::sync::Arc;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/integrations/teams/events", post(teams_events))
}

// --- crypto helpers (duplicated per-file to avoid cross-module deps) ---

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(data))
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).unwrap();
    mac.update(msg);
    hex::encode(mac.finalize().into_bytes())
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// --- signature verification ---

enum VerifyResult {
    Ok,
    Err(StatusCode, serde_json::Value),
}

fn verify_teams(headers: &axum::http::HeaderMap, body: &[u8]) -> VerifyResult {
    let signing_secret = std::env::var("TEAMS_SIGNING_SECRET").unwrap_or_default();
    let verification_token = std::env::var("TEAMS_VERIFICATION_TOKEN").unwrap_or_default();

    if !signing_secret.is_empty() {
        let timestamp = headers
            .get("x-mc-timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let signature = headers
            .get("x-mc-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let body_hash = sha256_hex(body);
        let message = format!("{}.{}", timestamp, body_hash);
        let expected = hmac_sha256(signing_secret.as_bytes(), message.as_bytes());
        if !constant_time_eq(&expected, signature) {
            return VerifyResult::Err(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({"code": "teams_signature_invalid", "reason": "hmac_invalid"}),
            );
        }
        return VerifyResult::Ok;
    }

    if !verification_token.is_empty() {
        // Try header first
        if let Some(header_token) = headers
            .get("x-missioncontrol-teams-token")
            .and_then(|v| v.to_str().ok())
        {
            if constant_time_eq(header_token, &verification_token) {
                return VerifyResult::Ok;
            }
            return VerifyResult::Err(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({"code": "teams_signature_invalid", "reason": "token_missing_or_invalid"}),
            );
        }
        // Try payload token
        if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(body) {
            if let Some(token) = payload.get("token").and_then(|v| v.as_str()) {
                if constant_time_eq(token, &verification_token) {
                    return VerifyResult::Ok;
                }
            }
        }
        return VerifyResult::Err(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({"code": "teams_signature_invalid", "reason": "token_missing_or_invalid"}),
        );
    }

    VerifyResult::Err(
        StatusCode::UNAUTHORIZED,
        serde_json::json!({"code": "teams_signature_invalid", "reason": "teams_verification_token_missing"}),
    )
}

// --- handler ---

async fn teams_events(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    match verify_teams(&headers, &body) {
        VerifyResult::Err(status, detail) => {
            return (status, Json(detail)).into_response();
        }
        VerifyResult::Ok => {}
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"detail": "Invalid Teams payload"})),
            )
                .into_response()
        }
    };

    let event_type = payload
        .get("type")
        .or_else(|| payload.get("eventType"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let channel_id = payload
        .get("channel_id")
        .or_else(|| payload.get("conversationId"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let mission_id = payload
        .get("mission_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if !mission_id.is_empty() && !channel_id.is_empty() {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM slackchannelbinding \
             WHERE provider='teams' AND mission_id=$1 AND channel_id=$2)",
        )
        .bind(&mission_id)
        .bind(&channel_id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);

        if !exists {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"detail": "Teams channel is not bound to mission"})),
            )
                .into_response();
        }
    }

    let et = if event_type.is_empty() { "event".to_string() } else { event_type.clone() };
    let dedupe_key = format!(
        "teams:{}:{}:{}:{}",
        et,
        mission_id,
        channel_id,
        sha256_hex(&body)
    );

    let result = sqlx::query(
        "INSERT INTO chatinboundreceipt (provider, event_key, event_type, created_at) \
         VALUES ('teams', $1, 'event', now()) \
         ON CONFLICT (event_key) DO NOTHING",
    )
    .bind(&dedupe_key)
    .execute(&state.db)
    .await;

    match result {
        Ok(r) if r.rows_affected() == 0 => {
            return Json(serde_json::json!({"ok": true, "duplicate": true})).into_response();
        }
        Err(e) => {
            tracing::error!("teams_events insert receipt: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(_) => {}
    }

    tracing::info!(
        event_type = %event_type,
        mission_id = %mission_id,
        channel_id = %channel_id,
        "teams event received"
    );

    Json(serde_json::json!({
        "ok": true,
        "provider": "teams",
        "text": "Event received."
    }))
    .into_response()
}
