use axum::{
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use std::sync::Arc;

use crate::state::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/webhooks/tailscale", post(tailscale_webhook))
}

// ---------------------------------------------------------------------------
// Tailscale HMAC-SHA256 signature verification
//
// Header: Tailscale-Webhook-Signature: t=<unix_ts>,v1=<hex>
// Signed payload: "<timestamp>.<raw_body>"
// Replay window: 5 minutes
// ---------------------------------------------------------------------------

fn verify_tailscale(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), (StatusCode, serde_json::Value)> {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;

    let secret = std::env::var("TS_WEBHOOK_SECRET").unwrap_or_default();
    if secret.is_empty() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({
                "code": "ts_webhook_unconfigured",
                "reason": "TS_WEBHOOK_SECRET not set"
            }),
        ));
    }

    let sig_header = headers
        .get("tailscale-webhook-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if sig_header.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "code": "ts_sig_missing",
                "reason": "Tailscale-Webhook-Signature header absent"
            }),
        ));
    }

    // Parse t=<ts>,v1=<sig>
    let mut timestamp = "";
    let mut provided_sig = "";
    for part in sig_header.split(',') {
        if let Some(v) = part.trim().strip_prefix("t=") {
            timestamp = v;
        } else if let Some(v) = part.trim().strip_prefix("v1=") {
            provided_sig = v;
        }
    }

    if timestamp.is_empty() || provided_sig.is_empty() {
        return Err((
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "code": "ts_sig_malformed",
                "reason": "cannot parse t= or v1= from signature header"
            }),
        ));
    }

    // Replay protection: reject if timestamp is outside a 5-minute window
    let ts: i64 = timestamp.parse().unwrap_or(0);
    let now = chrono::Utc::now().timestamp();
    if (now - ts).abs() > 300 {
        return Err((
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "code": "ts_sig_expired",
                "reason": "webhook timestamp outside 5-minute replay window"
            }),
        ));
    }

    // HMAC-SHA256("<timestamp>.<raw_body>")
    let body_str = std::str::from_utf8(body).unwrap_or("");
    let signed_payload = format!("{timestamp}.{body_str}");

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"code": "ts_hmac_init_error"}),
        )
    })?;
    mac.update(signed_payload.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    if !constant_time_eq(&expected, provided_sig) {
        return Err((
            StatusCode::UNAUTHORIZED,
            serde_json::json!({
                "code": "ts_sig_invalid",
                "reason": "HMAC mismatch"
            }),
        ));
    }

    Ok(())
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
// Tailscale event types we care about
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum TailscaleEvent {
    NodeCreated,
    NodeDeleted,
    NodeKeyExpiringInOneDay,
    NodeKeyExpired,
    PolicyUpdate,
    SubnetIPForwardingNotEnabled,
    Unknown(String),
}

impl TailscaleEvent {
    fn from_str(s: &str) -> Self {
        match s {
            "nodeCreated" => Self::NodeCreated,
            "nodeDeleted" => Self::NodeDeleted,
            "nodeKeyExpiringInOneDay" => Self::NodeKeyExpiringInOneDay,
            "nodeKeyExpired" => Self::NodeKeyExpired,
            "policyUpdate" => Self::PolicyUpdate,
            "subnetIPForwardingNotEnabled" => Self::SubnetIPForwardingNotEnabled,
            other => Self::Unknown(other.to_string()),
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Self::NodeCreated => "nodeCreated",
            Self::NodeDeleted => "nodeDeleted",
            Self::NodeKeyExpiringInOneDay => "nodeKeyExpiringInOneDay",
            Self::NodeKeyExpired => "nodeKeyExpired",
            Self::PolicyUpdate => "policyUpdate",
            Self::SubnetIPForwardingNotEnabled => "subnetIPForwardingNotEnabled",
            Self::Unknown(s) => s.as_str(),
        }
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

async fn tailscale_webhook(
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if let Err((status, detail)) = verify_tailscale(&headers, &body) {
        tracing::warn!(
            code = detail["code"].as_str().unwrap_or(""),
            reason = detail["reason"].as_str().unwrap_or(""),
            "tailscale webhook signature rejected"
        );
        return (status, Json(detail)).into_response();
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(err = %e, "tailscale webhook: invalid JSON body");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"code": "invalid_json"})),
            )
                .into_response();
        }
    };

    // Tailscale sends an array of events in a single POST
    let events = match payload.as_array() {
        Some(arr) => arr.clone(),
        None => vec![payload.clone()],
    };

    for event in &events {
        let event_type = event
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let tailnet = event
            .get("tailnet")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let timestamp = event
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let ts_event = TailscaleEvent::from_str(event_type);

        match &ts_event {
            TailscaleEvent::NodeCreated | TailscaleEvent::NodeDeleted => {
                let node_id = event
                    .get("data")
                    .and_then(|d| d.get("nodeID"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let device_name = event
                    .get("data")
                    .and_then(|d| d.get("deviceName"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                tracing::info!(
                    event_type = ts_event.as_str(),
                    tailnet,
                    node_id,
                    device_name,
                    ts = timestamp,
                    "tailscale node event"
                );
            }

            TailscaleEvent::NodeKeyExpiringInOneDay | TailscaleEvent::NodeKeyExpired => {
                let node_id = event
                    .get("data")
                    .and_then(|d| d.get("nodeID"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let device_name = event
                    .get("data")
                    .and_then(|d| d.get("deviceName"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                tracing::warn!(
                    event_type = ts_event.as_str(),
                    tailnet,
                    node_id,
                    device_name,
                    ts = timestamp,
                    "tailscale node key expiry"
                );
            }

            TailscaleEvent::PolicyUpdate => {
                tracing::info!(
                    event_type = ts_event.as_str(),
                    tailnet,
                    ts = timestamp,
                    "tailscale policy updated"
                );
            }

            TailscaleEvent::SubnetIPForwardingNotEnabled => {
                let node_id = event
                    .get("data")
                    .and_then(|d| d.get("nodeID"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                tracing::warn!(
                    event_type = ts_event.as_str(),
                    tailnet,
                    node_id,
                    ts = timestamp,
                    "tailscale subnet IP forwarding not enabled"
                );
            }

            TailscaleEvent::Unknown(t) => {
                tracing::debug!(event_type = %t, tailnet, ts = timestamp, "tailscale unhandled event type");
            }
        }
    }

    Json(serde_json::json!({"ok": true, "received": events.len()})).into_response()
}
