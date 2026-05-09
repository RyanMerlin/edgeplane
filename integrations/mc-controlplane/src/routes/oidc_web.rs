//! OIDC authentication routes — device flow (RFC 8628), browser PKCE flow, CLI login.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Duration, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

use crate::{auth::make_token, state::AppState};

// ── Constants ─────────────────────────────────────────────────────────────────

const SESSION_PREFIX: &str = "mcs_";
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_TTL_HOURS: i64 = 8;
const DEFAULT_DEVICE_INTERVAL: u64 = 5;
const DEFAULT_DEVICE_EXPIRES: u64 = 600;

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/oidc/device/authorize", post(device_authorize))
        .route("/auth/oidc/device/verify", get(device_verify))
        .route("/auth/oidc/device/token", post(device_token))
        .route("/auth/oidc/device/success", get(device_success))
        .route("/auth/oidc/cli-initiate", get(cli_initiate))
        .route("/auth/oidc/cli-poll/{cli_nonce}", get(cli_poll))
        .route("/auth/oidc/start", get(oidc_start))
        .route("/auth/oidc/callback", get(oidc_callback))
        .route("/auth/oidc/cli-success", get(cli_success_page))
        .route("/auth/oidc/exchange", post(exchange_grant))
}

// ── OIDC Config helpers ───────────────────────────────────────────────────────

struct OidcConfig {
    /// Issuer URL used for discovery (may be internal / cluster-local).
    issuer_internal: String,
    /// Issuer URL shown to the browser (public-facing).
    issuer_public: String,
    client_id: String,
    client_secret: Option<String>,
    /// Redirect URI sent to the OIDC provider on callback.
    redirect_uri_override: Option<String>,
    scopes: String,
    device_interval: u64,
    device_expires: u64,
    session_ttl_hours: i64,
    session_cookie_secure: bool,
}

impl OidcConfig {
    fn from_env() -> Self {
        let issuer = std::env::var("OIDC_ISSUER_URL").unwrap_or_default();
        let internal = std::env::var("OIDC_INTERNAL_ISSUER_URL").unwrap_or_else(|_| issuer.clone());
        let public = std::env::var("OIDC_PUBLIC_ISSUER_URL").unwrap_or_else(|_| issuer.clone());
        let client_id = std::env::var("OIDC_CLIENT_ID").unwrap_or_default();
        let client_secret = std::env::var("OIDC_CLIENT_SECRET").ok();
        let redirect_uri_override = std::env::var("OIDC_REDIRECT_URI").ok();
        let scopes = std::env::var("OIDC_SCOPES")
            .unwrap_or_else(|_| "openid profile email".to_string());
        let device_interval = std::env::var("OIDC_DEVICE_INTERVAL_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_DEVICE_INTERVAL);
        let device_expires = std::env::var("OIDC_DEVICE_EXPIRES_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_DEVICE_EXPIRES);
        let session_ttl_hours = std::env::var("MC_SESSION_TTL_HOURS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TTL_HOURS);
        let session_cookie_secure = std::env::var("MC_SESSION_COOKIE_SECURE")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(true);

        OidcConfig {
            issuer_internal: internal,
            issuer_public: public,
            client_id,
            client_secret,
            redirect_uri_override,
            scopes,
            device_interval,
            device_expires,
            session_ttl_hours,
            session_cookie_secure,
        }
    }
}

// ── PKCE helpers ──────────────────────────────────────────────────────────────

fn generate_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash.as_slice())
}

fn random_b64url(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(&bytes)
}

/// Derive the 4-4 uppercase user code from a device code.
fn device_user_code(device_code: &str) -> String {
    let digest = format!("{:X}", Sha256::digest(device_code.as_bytes()));
    let raw = &digest[..8];
    format!("{}-{}", &raw[..4], &raw[4..])
}

// ── JWT payload extraction (no signature verification) ───────────────────────

fn extract_jwt_claims(token: &str) -> serde_json::Value {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() < 2 {
        return serde_json::json!({});
    }
    let payload = parts[1];
    // Re-pad to a multiple of 4
    let padded = {
        let pad = (4 - payload.len() % 4) % 4;
        let mut s = payload.to_string();
        s.extend(std::iter::repeat('=').take(pad));
        s
    };
    use base64::engine::general_purpose::URL_SAFE;
    URL_SAFE
        .decode(&padded)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::json!({}))
}

// ── OIDC discovery ────────────────────────────────────────────────────────────

async fn fetch_discovery(issuer: &str) -> Result<serde_json::Value, String> {
    let url = format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("discovery request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("discovery returned HTTP {}", resp.status()));
    }
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("discovery parse error: {e}"))
}

// ── Session token issuance ────────────────────────────────────────────────────

/// Create a new usersession row and return `(token, session_id, expires_at)`.
async fn issue_session_token(
    db: &PgPool,
    subject: &str,
    user_agent: &str,
    ttl_hours: i64,
) -> Result<(String, i32, chrono::NaiveDateTime), sqlx::Error> {
    let token = make_token(SESSION_PREFIX);
    let token_hash = crate::auth::hash_token(&token);
    let token_prefix = token[..token.len().min(12)].to_string();
    let now = Utc::now().naive_utc();
    let expires_at = now + Duration::hours(ttl_hours);

    let session_id = sqlx::query_scalar::<_, i32>(
        "INSERT INTO usersession \
         (subject, token_hash, token_prefix, expires_at, created_at, last_used_at, user_agent, revoked, capability_scope) \
         VALUES ($1,$2,$3,$4,$5,$5,$6,false,'') RETURNING id",
    )
    .bind(subject)
    .bind(&token_hash)
    .bind(&token_prefix)
    .bind(expires_at)
    .bind(now)
    .bind(user_agent)
    .fetch_one(db)
    .await?;

    Ok((token, session_id, expires_at))
}

/// Build a `Set-Cookie` header value for the session cookie.
fn session_cookie(token: &str, expires_at: chrono::NaiveDateTime, secure: bool) -> String {
    let expires = expires_at.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let secure_flag = if secure { "; Secure" } else { "" };
    format!(
        "mc_session_token={token}; HttpOnly{secure_flag}; SameSite=Lax; Path=/; Expires={expires}"
    )
}

// ── Error helper ──────────────────────────────────────────────────────────────

fn json_err(status: StatusCode, detail: &str) -> Response {
    (status, Json(serde_json::json!({"detail": detail}))).into_response()
}

// ═══════════════════════════════════════════════════════════════════════════════
// Device flow — RFC 8628
// ═══════════════════════════════════════════════════════════════════════════════

// POST /auth/oidc/device/authorize
//
// Requests a device_code + user_code from the MC server (not the upstream OIDC
// provider directly — we proxy / wrap the device flow).  We store an
// OidcAuthRequest so that when the browser verifies, we can link back to it.

async fn device_authorize(State(state): State<Arc<AppState>>) -> Response {
    let cfg = OidcConfig::from_env();

    // Discover the OIDC endpoint (we only need it to validate the issuer is
    // reachable; the actual authorization URL is our own /device/verify).
    if let Err(e) = fetch_discovery(&cfg.issuer_internal).await {
        tracing::warn!("device_authorize: discovery failed: {e}");
    }

    let device_code = random_b64url(32);
    let user_code = device_user_code(&device_code);
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();
    let expires_at = now + Duration::seconds(cfg.device_expires as i64);

    // state field stores the device_code so /device/verify can look it up.
    let result = sqlx::query(
        "INSERT INTO oidcauthrequest \
         (id, state, code_verifier, nonce, redirect_path, cli_nonce, created_at, expires_at) \
         VALUES ($1,$2,'','','/ui/',NULL,$3,$4)",
    )
    .bind(&id)
    .bind(&device_code)
    .bind(now)
    .bind(expires_at)
    .execute(&state.db)
    .await;

    if let Err(e) = result {
        tracing::error!("device_authorize: db insert: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // The verification_uri is our own /device/verify page where the browser
    // user enters their user_code.
    let verification_uri = format!("/auth/oidc/device/verify?user_code={user_code}");
    let verification_uri_complete = format!("/auth/oidc/device/verify?user_code={user_code}");

    Json(serde_json::json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": verification_uri,
        "verification_uri_complete": verification_uri_complete,
        "expires_in": cfg.device_expires,
        "interval": cfg.device_interval,
    }))
    .into_response()
}

// ─── GET /auth/oidc/device/verify ────────────────────────────────────────────
//
// Browser lands here after scanning QR / typing the URI.  We find the matching
// OidcAuthRequest by user_code (stored as state = device_code, user_code is
// derived from it), then redirect the browser to the real OIDC authorize URL.

#[derive(serde::Deserialize)]
struct DeviceVerifyQuery {
    user_code: Option<String>,
}

async fn device_verify(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DeviceVerifyQuery>,
) -> Response {
    let user_code = match q.user_code {
        Some(c) => c.trim().to_uppercase(),
        None => {
            return Html(error_page("Missing user_code parameter.")).into_response();
        }
    };

    let cfg = OidcConfig::from_env();

    // Load all non-expired, unused auth requests and find by matching user_code.
    let now = Utc::now().naive_utc();
    let rows = sqlx::query(
        "SELECT id, state, code_verifier FROM oidcauthrequest \
         WHERE used_at IS NULL AND expires_at > $1",
    )
    .bind(now)
    .fetch_all(&state.db)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("device_verify: db: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let matched = rows.into_iter().find(|row| {
        let device_code: String = row.get("state");
        device_user_code(&device_code) == user_code
    });

    let row = match matched {
        Some(r) => r,
        None => {
            return Html(error_page("Invalid or expired user code.")).into_response();
        }
    };

    let request_id: String = row.get("id");
    let device_code: String = row.get("state");

    // Generate PKCE for this browser leg.
    let verifier = generate_verifier();
    let challenge = pkce_challenge(&verifier);
    let nonce = random_b64url(16);

    // Update the row with the verifier + nonce so the callback can use them.
    if let Err(e) = sqlx::query(
        "UPDATE oidcauthrequest SET code_verifier=$1, nonce=$2 WHERE id=$3",
    )
    .bind(&verifier)
    .bind(&nonce)
    .bind(&request_id)
    .execute(&state.db)
    .await
    {
        tracing::error!("device_verify: update: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Fetch the OIDC discovery doc.
    let discovery = match fetch_discovery(&cfg.issuer_internal).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("device_verify: discovery: {e}");
            return Html(error_page("OIDC provider unavailable.")).into_response();
        }
    };

    let authorization_endpoint = match discovery["authorization_endpoint"].as_str() {
        Some(u) => u.to_string(),
        None => {
            return Html(error_page("OIDC provider returned no authorization_endpoint."))
                .into_response();
        }
    };

    // Redirect URI points back to our callback.
    let redirect_uri = cfg
        .redirect_uri_override
        .unwrap_or_else(|| "/auth/oidc/callback".to_string());

    // We encode the device flow context via the state param (prefixed so callback knows).
    let oidc_state = format!("device:{device_code}:{request_id}");

    let authorize_url = format!(
        "{authorization_endpoint}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
        urlencoded(&cfg.client_id),
        urlencoded(&redirect_uri),
        urlencoded(&cfg.scopes),
        urlencoded(&oidc_state),
        urlencoded(&nonce),
        urlencoded(&challenge),
    );

    // Use the public issuer for the browser redirect.
    let public_auth_endpoint = authorization_endpoint.replace(
        cfg.issuer_internal.trim_end_matches('/'),
        cfg.issuer_public.trim_end_matches('/'),
    );
    let public_authorize_url = format!(
        "{public_auth_endpoint}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
        urlencoded(&cfg.client_id),
        urlencoded(&redirect_uri),
        urlencoded(&cfg.scopes),
        urlencoded(&oidc_state),
        urlencoded(&nonce),
        urlencoded(&challenge),
    );
    let _ = authorize_url; // prefer public URL for browser

    Redirect::to(&public_authorize_url).into_response()
}

// ─── POST /auth/oidc/device/token ────────────────────────────────────────────
//
// RFC 8628 token polling.  Client polls with device_code until the browser leg
// completes (i.e. OidcLoginGrant exists for this device_code).

async fn device_token(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    // Accept form-encoded OR JSON.
    body: axum::body::Bytes,
) -> Response {
    let body_bytes: &[u8] = &body;

    let (grant_type, device_code, user_agent_body): (Option<String>, Option<String>, Option<String>) =
        if headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.contains("application/json"))
            .unwrap_or(false)
        {
            let v: serde_json::Value =
                serde_json::from_slice(body_bytes).unwrap_or(serde_json::json!({}));
            (
                v["grant_type"].as_str().map(|s| s.to_string()),
                v["device_code"].as_str().map(|s| s.to_string()),
                v["user_agent"].as_str().map(|s| s.to_string()),
            )
        } else {
            // form-encoded
            let parsed: Vec<(String, String)> = form_urlencoded(body_bytes);
            let get = |key: &str| {
                parsed
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.clone())
            };
            (get("grant_type"), get("device_code"), get("user_agent"))
        };

    let grant_type = grant_type.unwrap_or_default();
    if grant_type != DEVICE_GRANT_TYPE {
        return json_err(
            StatusCode::BAD_REQUEST,
            &format!("unsupported grant_type, expected {DEVICE_GRANT_TYPE}"),
        );
    }

    let device_code = match device_code {
        Some(dc) => dc,
        None => return json_err(StatusCode::BAD_REQUEST, "missing device_code"),
    };

    let ua = user_agent_body
        .or_else(|| {
            headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    let now = Utc::now().naive_utc();

    // Check that the auth request exists and hasn't expired.
    let auth_req = sqlx::query(
        "SELECT id, expires_at FROM oidcauthrequest WHERE state=$1",
    )
    .bind(&device_code)
    .fetch_optional(&state.db)
    .await;

    match auth_req {
        Err(e) => {
            tracing::error!("device_token: auth_req lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(None) => {
            return Json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "device_code not found",
            }))
            .into_response();
        }
        Ok(Some(row)) => {
            let expires_at: chrono::NaiveDateTime = row.get("expires_at");
            if expires_at < now {
                return Json(serde_json::json!({
                    "error": "expired_token",
                    "error_description": "device code has expired",
                }))
                .into_response();
            }
        }
    }

    // Look for a completed OidcLoginGrant for this device_code.
    let grant = sqlx::query(
        "SELECT id, subject, email FROM oidclogingrant \
         WHERE cli_nonce=$1 AND used_at IS NULL AND expires_at > $2",
    )
    .bind(&device_code)
    .bind(now)
    .fetch_optional(&state.db)
    .await;

    let grant = match grant {
        Err(e) => {
            tracing::error!("device_token: grant lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(None) => {
            // Authorization pending.
            return Json(serde_json::json!({
                "error": "authorization_pending",
                "error_description": "the user has not yet authorized this device",
            }))
            .into_response();
        }
        Ok(Some(r)) => r,
    };

    let grant_id: String = grant.get("id");
    let subject: String = grant.get("subject");

    let cfg = OidcConfig::from_env();

    // Issue a session token.
    let (token, _session_id, expires_at) =
        match issue_session_token(&state.db, &subject, &ua, cfg.session_ttl_hours).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("device_token: issue_session: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    // Mark grant as used.
    let _ = sqlx::query("UPDATE oidclogingrant SET used_at=$1 WHERE id=$2")
        .bind(now)
        .bind(&grant_id)
        .execute(&state.db)
        .await;

    // Mark auth request as used.
    let _ = sqlx::query("UPDATE oidcauthrequest SET used_at=$1 WHERE state=$2")
        .bind(now)
        .bind(&device_code)
        .execute(&state.db)
        .await;

    let expires_in = (expires_at - now).num_seconds().max(0);

    Json(serde_json::json!({
        "access_token": token,
        "token_type": "Bearer",
        "expires_in": expires_in,
        "subject": subject,
    }))
    .into_response()
}

// ─── GET /auth/oidc/device/success ───────────────────────────────────────────

async fn device_success() -> impl IntoResponse {
    Html(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Device Authorized — MissionControl</title>
  <style>
    body { font-family: system-ui, sans-serif; background: #0f172a; color: #e2e8f0;
           display: flex; align-items: center; justify-content: center; min-height: 100vh; margin: 0; }
    .card { background: #1e293b; border-radius: 12px; padding: 2.5rem 3rem; text-align: center; max-width: 420px; }
    h1 { color: #22c55e; font-size: 1.8rem; margin: 0 0 1rem; }
    p { color: #94a3b8; line-height: 1.6; }
    .check { font-size: 4rem; margin-bottom: 1rem; }
  </style>
</head>
<body>
  <div class="card">
    <div class="check">&#10003;</div>
    <h1>Device Authorized</h1>
    <p>Your device has been successfully authorized.<br>You can now close this window and return to your terminal.</p>
  </div>
</body>
</html>"#)
}

// ═══════════════════════════════════════════════════════════════════════════════
// CLI PKCE flow
// ═══════════════════════════════════════════════════════════════════════════════

// ─── GET /auth/oidc/cli-initiate ─────────────────────────────────────────────
//
// The MC CLI calls this to start a PKCE login.  Returns the authorize_url the
// CLI should open in a browser, plus a cli_nonce it will poll on.

#[derive(serde::Deserialize)]
struct CliInitiateQuery {
    redirect_path: Option<String>,
}

async fn cli_initiate(
    State(state): State<Arc<AppState>>,
    Query(q): Query<CliInitiateQuery>,
) -> Response {
    let cfg = OidcConfig::from_env();

    let discovery = match fetch_discovery(&cfg.issuer_internal).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("cli_initiate: discovery: {e}");
            return json_err(StatusCode::BAD_GATEWAY, "OIDC provider unreachable");
        }
    };

    let auth_endpoint = match discovery["authorization_endpoint"].as_str() {
        Some(u) => u.to_string(),
        None => return json_err(StatusCode::BAD_GATEWAY, "no authorization_endpoint in discovery"),
    };

    let verifier = generate_verifier();
    let challenge = pkce_challenge(&verifier);
    let nonce = random_b64url(16);
    let cli_nonce = random_b64url(24);
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();
    let expires_at = now + Duration::hours(cfg.session_ttl_hours);
    let redirect_path = q.redirect_path.unwrap_or_else(|| "/auth/oidc/cli-success".to_string());

    let result = sqlx::query(
        "INSERT INTO oidcauthrequest \
         (id, state, code_verifier, nonce, redirect_path, cli_nonce, created_at, expires_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(&id)
    .bind(&id) // state = id for CLI flow
    .bind(&verifier)
    .bind(&nonce)
    .bind(&redirect_path)
    .bind(&cli_nonce)
    .bind(now)
    .bind(expires_at)
    .execute(&state.db)
    .await;

    if let Err(e) = result {
        tracing::error!("cli_initiate: db: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let redirect_uri = cfg
        .redirect_uri_override
        .unwrap_or_else(|| "/auth/oidc/callback".to_string());

    // Replace internal issuer with public one for the browser URL.
    let public_auth_endpoint = auth_endpoint.replace(
        cfg.issuer_internal.trim_end_matches('/'),
        cfg.issuer_public.trim_end_matches('/'),
    );

    let authorize_url = format!(
        "{public_auth_endpoint}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
        urlencoded(&cfg.client_id),
        urlencoded(&redirect_uri),
        urlencoded(&cfg.scopes),
        urlencoded(&id), // state = auth request id
        urlencoded(&nonce),
        urlencoded(&challenge),
    );

    Json(serde_json::json!({
        "authorize_url": authorize_url,
        "cli_nonce": cli_nonce,
    }))
    .into_response()
}

// ─── GET /auth/oidc/cli-poll/{cli_nonce} ─────────────────────────────────────
//
// CLI polls this after opening the browser.  Returns 202 (pending) or 200 with
// the session token once the browser leg completes.

#[derive(serde::Deserialize)]
struct CliPollPathParams {
    cli_nonce: String,
}

async fn cli_poll(
    State(state): State<Arc<AppState>>,
    Path(params): Path<CliPollPathParams>,
    headers: axum::http::HeaderMap,
) -> Response {
    let now = Utc::now().naive_utc();
    let cfg = OidcConfig::from_env();

    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Find an OidcLoginGrant linked to this cli_nonce.
    let grant = sqlx::query(
        "SELECT id, subject, email FROM oidclogingrant \
         WHERE cli_nonce=$1 AND used_at IS NULL AND expires_at > $2",
    )
    .bind(&params.cli_nonce)
    .bind(now)
    .fetch_optional(&state.db)
    .await;

    let grant = match grant {
        Err(e) => {
            tracing::error!("cli_poll: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(None) => {
            // Check if the auth request even exists (to distinguish "pending" vs "unknown").
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM oidcauthrequest WHERE cli_nonce=$1 AND expires_at > $2)",
            )
            .bind(&params.cli_nonce)
            .bind(now)
            .fetch_one(&state.db)
            .await
            .unwrap_or(false);

            if exists {
                return (
                    StatusCode::ACCEPTED,
                    Json(serde_json::json!({"status": "pending"})),
                )
                    .into_response();
            } else {
                return json_err(StatusCode::NOT_FOUND, "cli_nonce not found or expired");
            }
        }
        Ok(Some(r)) => r,
    };

    let grant_id: String = grant.get("id");
    let subject: String = grant.get("subject");

    let (token, _session_id, expires_at) =
        match issue_session_token(&state.db, &subject, &ua, cfg.session_ttl_hours).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("cli_poll: issue_session: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    // Mark grant used.
    let _ = sqlx::query("UPDATE oidclogingrant SET used_at=$1 WHERE id=$2")
        .bind(now)
        .bind(&grant_id)
        .execute(&state.db)
        .await;

    Json(serde_json::json!({
        "status": "complete",
        "access_token": token,
        "token_type": "Bearer",
        "subject": subject,
        "expires_at": expires_at,
    }))
    .into_response()
}

// ═══════════════════════════════════════════════════════════════════════════════
// Browser PKCE flow
// ═══════════════════════════════════════════════════════════════════════════════

// ─── GET /auth/oidc/start ─────────────────────────────────────────────────────
//
// Browser hits this to start a fresh PKCE login (no CLI context).

#[derive(serde::Deserialize)]
struct OidcStartQuery {
    redirect_path: Option<String>,
}

async fn oidc_start(
    State(state): State<Arc<AppState>>,
    Query(q): Query<OidcStartQuery>,
) -> Response {
    let cfg = OidcConfig::from_env();

    let discovery = match fetch_discovery(&cfg.issuer_internal).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("oidc_start: {e}");
            return Html(error_page("OIDC provider unavailable.")).into_response();
        }
    };

    let auth_endpoint = match discovery["authorization_endpoint"].as_str() {
        Some(u) => u.to_string(),
        None => return Html(error_page("No authorization_endpoint in OIDC discovery.")).into_response(),
    };

    let verifier = generate_verifier();
    let challenge = pkce_challenge(&verifier);
    let nonce = random_b64url(16);
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();
    let expires_at = now + Duration::hours(cfg.session_ttl_hours);
    let redirect_path = q.redirect_path.unwrap_or_else(|| "/ui/".to_string());

    let result = sqlx::query(
        "INSERT INTO oidcauthrequest \
         (id, state, code_verifier, nonce, redirect_path, cli_nonce, created_at, expires_at) \
         VALUES ($1,$2,$3,$4,$5,NULL,$6,$7)",
    )
    .bind(&id)
    .bind(&id)
    .bind(&verifier)
    .bind(&nonce)
    .bind(&redirect_path)
    .bind(now)
    .bind(expires_at)
    .execute(&state.db)
    .await;

    if let Err(e) = result {
        tracing::error!("oidc_start: db: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let redirect_uri = cfg
        .redirect_uri_override
        .unwrap_or_else(|| "/auth/oidc/callback".to_string());

    let public_auth_endpoint = auth_endpoint.replace(
        cfg.issuer_internal.trim_end_matches('/'),
        cfg.issuer_public.trim_end_matches('/'),
    );

    let authorize_url = format!(
        "{public_auth_endpoint}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
        urlencoded(&cfg.client_id),
        urlencoded(&redirect_uri),
        urlencoded(&cfg.scopes),
        urlencoded(&id),
        urlencoded(&nonce),
        urlencoded(&challenge),
    );

    Redirect::to(&authorize_url).into_response()
}

// ─── GET /auth/oidc/callback ──────────────────────────────────────────────────
//
// OIDC provider redirects the browser here after the user authenticates.
// Exchanges the authorization code for tokens, validates (basic), creates an
// OidcLoginGrant so CLI / device pollers can retrieve a session.

#[derive(serde::Deserialize)]
struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn oidc_callback(
    State(state): State<Arc<AppState>>,
    Query(q): Query<OidcCallbackQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Surface any error from the OIDC provider.
    if let Some(ref err) = q.error {
        let desc = q.error_description.as_deref().unwrap_or("");
        tracing::warn!("oidc_callback: provider error={err} desc={desc}");
        return Html(error_page(&format!("OIDC error: {err}. {desc}"))).into_response();
    }

    let code = match q.code {
        Some(c) => c,
        None => return Html(error_page("Missing authorization code.")).into_response(),
    };

    let oidc_state = match q.state {
        Some(s) => s,
        None => return Html(error_page("Missing state parameter.")).into_response(),
    };

    let cfg = OidcConfig::from_env();
    let now = Utc::now().naive_utc();

    // Determine flow type from state.
    // device flow: "device:{device_code}:{request_id}"
    // browser/cli: state == request_id (UUID)
    let (is_device, device_code_opt, request_id) = if oidc_state.starts_with("device:") {
        let parts: Vec<&str> = oidc_state.splitn(3, ':').collect();
        if parts.len() == 3 {
            (true, Some(parts[1].to_string()), parts[2].to_string())
        } else {
            return Html(error_page("Malformed state parameter.")).into_response();
        }
    } else {
        (false, None, oidc_state.clone())
    };

    // Load the auth request.
    let auth_req = sqlx::query(
        "SELECT id, state, code_verifier, nonce, redirect_path, cli_nonce, expires_at \
         FROM oidcauthrequest WHERE id=$1",
    )
    .bind(&request_id)
    .fetch_optional(&state.db)
    .await;

    let auth_req = match auth_req {
        Err(e) => {
            tracing::error!("oidc_callback: db lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(None) => return Html(error_page("Unknown auth request — may have expired.")).into_response(),
        Ok(Some(r)) => r,
    };

    let expires_at: chrono::NaiveDateTime = auth_req.get("expires_at");
    if expires_at < now {
        return Html(error_page("Auth request has expired. Please try again.")).into_response();
    }

    let code_verifier: String = auth_req.get("code_verifier");
    let redirect_path: String = auth_req.get("redirect_path");
    let cli_nonce: Option<String> = auth_req.get("cli_nonce");

    // Fetch discovery (internal URL for server-side call).
    let discovery = match fetch_discovery(&cfg.issuer_internal).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("oidc_callback: discovery: {e}");
            return Html(error_page("OIDC provider unavailable.")).into_response();
        }
    };

    let token_endpoint = match discovery["token_endpoint"].as_str() {
        Some(u) => u.to_string(),
        None => return Html(error_page("No token_endpoint in OIDC discovery.")).into_response(),
    };

    let redirect_uri = cfg
        .redirect_uri_override
        .unwrap_or_else(|| "/auth/oidc/callback".to_string());

    // Exchange authorization code for tokens.
    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.clone()),
        ("redirect_uri", redirect_uri.clone()),
        ("client_id", cfg.client_id.clone()),
        ("code_verifier", code_verifier.clone()),
    ];
    if let Some(ref secret) = cfg.client_secret {
        params.push(("client_secret", secret.clone()));
    }

    let http_client = reqwest::Client::new();
    let token_resp = http_client
        .post(&token_endpoint)
        .form(&params)
        .send()
        .await;

    let token_resp = match token_resp {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("oidc_callback: token exchange: {e}");
            return Html(error_page("Failed to contact OIDC provider.")).into_response();
        }
    };

    if !token_resp.status().is_success() {
        let status = token_resp.status();
        let body = token_resp.text().await.unwrap_or_default();
        tracing::error!("oidc_callback: token endpoint {status}: {body}");
        return Html(error_page("Token exchange failed.")).into_response();
    }

    let token_data: serde_json::Value = match token_resp.json().await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("oidc_callback: token parse: {e}");
            return Html(error_page("Failed to parse token response.")).into_response();
        }
    };

    let id_token = match token_data["id_token"].as_str() {
        Some(t) => t.to_string(),
        None => {
            tracing::error!("oidc_callback: no id_token in response");
            return Html(error_page("No id_token in provider response.")).into_response();
        }
    };

    // Extract claims (basic, no signature verification).
    let claims = extract_jwt_claims(&id_token);
    let subject = claims["sub"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let email = claims["email"]
        .as_str()
        .unwrap_or("")
        .to_string();

    if subject.is_empty() {
        tracing::error!("oidc_callback: empty subject in id_token claims");
        return Html(error_page("Could not extract subject from id_token.")).into_response();
    }

    // Mark auth request used.
    let _ = sqlx::query("UPDATE oidcauthrequest SET used_at=$1 WHERE id=$2")
        .bind(now)
        .bind(&request_id)
        .execute(&state.db)
        .await;

    // Create OidcLoginGrant.
    // For device flow, store device_code in cli_nonce so the device/token poller finds it.
    let grant_cli_nonce = if is_device {
        device_code_opt.clone()
    } else {
        cli_nonce.clone()
    };

    let grant_id = Uuid::new_v4().to_string();
    let grant_expires_at = now + Duration::hours(cfg.session_ttl_hours);

    let grant_result = sqlx::query(
        "INSERT INTO oidclogingrant \
         (id, auth_request_id, subject, email, cli_nonce, created_at, expires_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(&grant_id)
    .bind(&request_id)
    .bind(&subject)
    .bind(&email)
    .bind(&grant_cli_nonce)
    .bind(now)
    .bind(grant_expires_at)
    .execute(&state.db)
    .await;

    if let Err(e) = grant_result {
        tracing::error!("oidc_callback: insert grant: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // For device flow: redirect to success page (browser can close).
    if is_device {
        return Redirect::to("/auth/oidc/device/success").into_response();
    }

    // For CLI flow: the CLI is polling /cli-poll/{cli_nonce}, so redirect browser to cli-success.
    if cli_nonce.is_some() {
        let url = format!("/auth/oidc/cli-success?grant_id={grant_id}");
        return Redirect::to(&url).into_response();
    }

    // Browser PKCE flow — issue session token and set cookie, redirect to redirect_path.
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let (token, _session_id, token_expires_at) =
        match issue_session_token(&state.db, &subject, &ua, cfg.session_ttl_hours).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("oidc_callback: issue_session: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    // Mark grant used immediately (browser consumed it).
    let _ = sqlx::query("UPDATE oidclogingrant SET used_at=$1 WHERE id=$2")
        .bind(now)
        .bind(&grant_id)
        .execute(&state.db)
        .await;

    let cookie = session_cookie(&token, token_expires_at, cfg.session_cookie_secure);
    let target = if redirect_path.is_empty() { "/ui/".to_string() } else { redirect_path };

    (
        StatusCode::FOUND,
        [
            (header::LOCATION, target),
            (header::SET_COOKIE, cookie),
        ],
    )
        .into_response()
}

// ─── GET /auth/oidc/cli-success ───────────────────────────────────────────────
//
// Browser lands here after CLI PKCE flow completes.  Shows the grant ID and
// a note to return to the terminal.

#[derive(serde::Deserialize)]
struct CliSuccessQuery {
    grant_id: Option<String>,
}

async fn cli_success_page(Query(q): Query<CliSuccessQuery>) -> impl IntoResponse {
    let grant_id = q.grant_id.unwrap_or_else(|| "(unknown)".to_string());
    let html = format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Login Complete — MissionControl</title>
  <style>
    *, *::before, *::after {{ box-sizing: border-box; }}
    body {{
      font-family: system-ui, -apple-system, sans-serif;
      background: #0f172a; color: #e2e8f0;
      display: flex; align-items: center; justify-content: center;
      min-height: 100vh; margin: 0;
    }}
    .card {{
      background: #1e293b; border-radius: 12px;
      padding: 2.5rem 3rem; text-align: center; max-width: 480px; width: 90%;
      box-shadow: 0 25px 50px rgba(0,0,0,.5);
    }}
    .check {{ font-size: 4rem; color: #22c55e; margin-bottom: 1rem; }}
    h1 {{ color: #f1f5f9; font-size: 1.6rem; margin: 0 0 0.75rem; }}
    p {{ color: #94a3b8; line-height: 1.6; margin: 0 0 1.25rem; }}
    .grant-box {{
      background: #0f172a; border: 1px solid #334155; border-radius: 8px;
      padding: 0.75rem 1rem; margin: 1.25rem 0; position: relative;
    }}
    .grant-label {{ font-size: 0.7rem; color: #64748b; text-transform: uppercase; letter-spacing: 0.05em; margin-bottom: 0.4rem; }}
    .grant-id {{
      font-family: 'Courier New', monospace; font-size: 0.85rem; color: #a5f3fc;
      word-break: break-all; user-select: all;
    }}
    .copy-btn {{
      margin-top: 0.75rem; background: #334155; color: #e2e8f0;
      border: none; border-radius: 6px; padding: 0.4rem 1rem;
      cursor: pointer; font-size: 0.85rem; transition: background 0.15s;
    }}
    .copy-btn:hover {{ background: #475569; }}
    .copy-btn.copied {{ background: #166534; color: #bbf7d0; }}
    .note {{ font-size: 0.8rem; color: #64748b; margin-top: 1.5rem; }}
  </style>
</head>
<body>
  <div class="card">
    <div class="check">&#10003;</div>
    <h1>Login Successful</h1>
    <p>Authentication complete. Your terminal is now logged in.</p>

    <div class="grant-box">
      <div class="grant-label">Grant ID</div>
      <div class="grant-id" id="grantId">{grant_id}</div>
      <button class="copy-btn" onclick="copyGrant()" id="copyBtn">Copy</button>
    </div>

    <p>Return to your terminal — it should detect the login automatically.<br>
       If polling timed out, paste the grant ID when prompted.</p>

    <div class="note">This window can be closed safely.</div>
  </div>

  <script>
    function copyGrant() {{
      const id = document.getElementById('grantId').textContent;
      navigator.clipboard.writeText(id).then(() => {{
        const btn = document.getElementById('copyBtn');
        btn.textContent = 'Copied!';
        btn.classList.add('copied');
        setTimeout(() => {{ btn.textContent = 'Copy'; btn.classList.remove('copied'); }}, 2000);
      }});
    }}
  </script>
</body>
</html>"#);
    Html(html)
}

// ═══════════════════════════════════════════════════════════════════════════════
// Grant exchange
// ═══════════════════════════════════════════════════════════════════════════════

// ─── POST /auth/oidc/exchange ─────────────────────────────────────────────────
//
// Exchange an OidcLoginGrant for a session token.  Used by CLI or any client
// that has the grant_id (e.g. obtained from the cli-success page).

#[derive(serde::Deserialize)]
struct ExchangeRequest {
    grant_id: String,
}

async fn exchange_grant(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ExchangeRequest>,
) -> Response {
    let now = Utc::now().naive_utc();
    let cfg = OidcConfig::from_env();

    let grant = sqlx::query(
        "SELECT id, subject, email FROM oidclogingrant \
         WHERE id=$1 AND used_at IS NULL AND expires_at > $2",
    )
    .bind(&body.grant_id)
    .fetch_optional(&state.db)
    .await;

    let grant = match grant {
        Err(e) => {
            tracing::error!("exchange_grant: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(None) => {
            return json_err(StatusCode::NOT_FOUND, "grant not found or already used");
        }
        Ok(Some(r)) => r,
    };

    let grant_id: String = grant.get("id");
    let subject: String = grant.get("subject");

    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let (token, _session_id, expires_at) =
        match issue_session_token(&state.db, &subject, &ua, cfg.session_ttl_hours).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("exchange_grant: issue_session: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    // Mark grant used.
    let _ = sqlx::query("UPDATE oidclogingrant SET used_at=$1 WHERE id=$2")
        .bind(now)
        .bind(&grant_id)
        .execute(&state.db)
        .await;

    let cookie = session_cookie(&token, expires_at, cfg.session_cookie_secure);

    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({
            "access_token": token,
            "token_type": "Bearer",
            "subject": subject,
            "expires_at": expires_at,
        })),
    )
        .into_response()
}

// ═══════════════════════════════════════════════════════════════════════════════
// Utility functions
// ═══════════════════════════════════════════════════════════════════════════════

/// Minimal percent-encoding for URL query parameter values.
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

/// Parse application/x-www-form-urlencoded body into key-value pairs.
fn form_urlencoded(body: &[u8]) -> Vec<(String, String)> {
    let s = std::str::from_utf8(body).unwrap_or("");
    s.split('&')
        .filter_map(|pair| {
            let mut it = pair.splitn(2, '=');
            let k = it.next()?;
            let v = it.next().unwrap_or("");
            Some((pct_decode(k), pct_decode(v)))
        })
        .collect()
}

fn pct_decode(s: &str) -> String {
    let s = s.replace('+', " ");
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hi) = hex_digit(bytes[i + 1]) {
                if let Ok(lo) = hex_digit(bytes[i + 2]) {
                    out.push((hi << 4) | lo);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Result<u8, ()> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(()),
    }
}

/// Simple error HTML page.
fn error_page(message: &str) -> String {
    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>Authentication Error — MissionControl</title>
  <style>
    body {{ font-family: system-ui, sans-serif; background: #0f172a; color: #e2e8f0;
           display: flex; align-items: center; justify-content: center; min-height: 100vh; margin: 0; }}
    .card {{ background: #1e293b; border-radius: 12px; padding: 2.5rem 3rem; text-align: center; max-width: 440px; }}
    h1 {{ color: #ef4444; font-size: 1.5rem; margin: 0 0 1rem; }}
    p {{ color: #94a3b8; line-height: 1.6; }}
    a {{ color: #60a5fa; }}
  </style>
</head>
<body>
  <div class="card">
    <h1>Authentication Error</h1>
    <p>{message}</p>
    <p><a href="/auth/oidc/start">Try again</a></p>
  </div>
</body>
</html>"#)
}
