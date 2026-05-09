use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{env, sync::Arc};

use crate::state::AppState;

/// Caller identity extracted from request headers.
pub struct Principal {
    pub subject: String,
    pub is_admin: bool,
    pub session_id: Option<i32>,
    /// One of: "static", "session", "service_account", "anonymous"
    pub auth_type: String,
}

impl FromRequestParts<Arc<AppState>> for Principal {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Self::Rejection> {
        let admin_token = env::var("MC_TOKEN").ok();
        let bearer = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.trim().to_string());

        let agent_id_header = parts
            .headers
            .get("x-mc-agent-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string());

        // Static admin token
        if let (Some(t), Some(b)) = (&admin_token, &bearer) {
            if t == b {
                let subject = agent_id_header.clone().unwrap_or_else(|| "admin".into());
                return Ok(Principal { subject, is_admin: true, session_id: None, auth_type: "static".into() });
            }
        }

        if let Some(ref token) = bearer {
            let hash = hash_token(token);
            let now = chrono::Utc::now().naive_utc();

            if token.starts_with("mcs_sa_") {
                // Service account token — validate against serviceaccounttoken + serviceaccount
                let row = sqlx::query(
                    "SELECT sat.id, sa.name \
                     FROM serviceaccounttoken sat \
                     JOIN serviceaccount sa ON sa.id = sat.service_account_id \
                     WHERE sat.token_hash = $1 AND sat.revoked = false AND sa.revoked = false \
                     AND (sat.expires_at IS NULL OR sat.expires_at > $2)"
                )
                .bind(&hash)
                .bind(now)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten();

                if let Some(row) = row {
                    let sa_name: String = row.get("name");
                    let token_id: i32 = row.get("id");
                    let subject = format!("sa:{sa_name}");
                    let db = state.db.clone();
                    let h = hash.clone();
                    tokio::spawn(async move {
                        let _ = sqlx::query(
                            "UPDATE serviceaccounttoken SET last_used_at = NOW() WHERE token_hash = $1"
                        )
                        .bind(&h)
                        .execute(&db)
                        .await;
                    });
                    return Ok(Principal {
                        subject,
                        is_admin: false,
                        session_id: Some(token_id),
                        auth_type: "service_account".into(),
                    });
                }
            } else if token.starts_with("mcs_") {
                // User session token — validate against usersession
                let row = sqlx::query(
                    "SELECT id, subject FROM usersession \
                     WHERE token_hash = $1 AND revoked = false AND expires_at > $2"
                )
                .bind(&hash)
                .bind(now)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten();

                if let Some(row) = row {
                    let subject: String = row.get("subject");
                    let session_id: i32 = row.get("id");
                    let db = state.db.clone();
                    let h = hash.clone();
                    tokio::spawn(async move {
                        let _ = sqlx::query(
                            "UPDATE usersession SET last_used_at = NOW() WHERE token_hash = $1"
                        )
                        .bind(&h)
                        .execute(&db)
                        .await;
                    });
                    return Ok(Principal {
                        subject,
                        is_admin: false,
                        session_id: Some(session_id),
                        auth_type: "session".into(),
                    });
                }
            }
        }

        // Anonymous / unrecognized
        let subject = agent_id_header
            .or(bearer.clone())
            .unwrap_or_else(|| "anonymous".into());

        Ok(Principal { subject, is_admin: false, session_id: None, auth_type: "anonymous".into() })
    }
}

/// SHA-256 hex digest of a token string.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Generate a new token with the given prefix (e.g. `"mcs_"`, `"mcs_sa_"`).
/// Suffix is 32 random bytes base64url-encoded (no padding), same entropy as
/// Python's `secrets.token_urlsafe(32)`.
pub fn make_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{}{}", prefix, URL_SAFE_NO_PAD.encode(bytes))
}
