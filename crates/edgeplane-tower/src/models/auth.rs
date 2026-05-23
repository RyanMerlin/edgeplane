use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct UserSession {
    pub id: i32,
    pub subject: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub expires_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
    pub last_used_at: NaiveDateTime,
    pub user_agent: String,
    pub revoked: bool,
    pub capability_scope: String,
}

/// A service account — headless identity with a stable `client_id` + `client_secret`.
/// The `client_secret` is stored only as a SHA-256 hash; the plaintext is returned once
/// on creation and never again.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ServiceAccount {
    pub id: i32,
    pub name: String,
    pub owner_subject: String,
    pub client_secret_prefix: String,
    pub created_at: NaiveDateTime,
    pub revoked: bool,
}

/// A short- or long-lived token minted by the client_credentials grant.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ServiceAccountToken {
    pub id: i32,
    pub service_account_id: i32,
    pub token_hash: String,
    pub token_prefix: String,
    pub expires_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub last_used_at: Option<NaiveDateTime>,
    pub revoked: bool,
}

// ── Request/response types ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SessionCreateRequest {
    pub ttl_hours: Option<i64>,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub token: String,
    pub subject: String,
    pub expires_at: NaiveDateTime,
    pub session_id: i32,
    pub ttl_hours: i64,
}

#[derive(Serialize)]
pub struct MeResponse {
    pub subject: String,
    pub auth_type: String,
    pub session_id: Option<i32>,
}

#[derive(Deserialize)]
pub struct ServiceAccountCreateRequest {
    pub name: String,
}

#[derive(Serialize)]
pub struct ServiceAccountCreatedResponse {
    pub id: i32,
    pub name: String,
    /// `client_id` is the same as `id` — separate field for OAuth2 naming convention.
    pub client_id: i32,
    /// Plaintext secret — shown once, never stored.
    pub client_secret: String,
    pub created_at: NaiveDateTime,
}

#[derive(Deserialize)]
pub struct TokenRequest {
    pub client_id: i32,
    pub client_secret: String,
    /// OAuth2 grant type — must be "client_credentials" if supplied.
    pub grant_type: Option<String>,
    /// If omitted the token never expires (suitable for long-running agents).
    pub expires_in_hours: Option<i64>,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_at: Option<NaiveDateTime>,
    pub expires_in_hours: Option<i64>,
    pub service_account_id: i32,
    pub token_id: i32,
}

#[derive(Deserialize)]
pub struct RevokeTokenRequest {
    /// Revoke by raw token value.
    pub token: Option<String>,
    /// Revoke by token DB id.
    pub token_id: Option<i32>,
}
