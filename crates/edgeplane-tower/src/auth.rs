use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::collections::HashSet;
use std::sync::Arc;

use crate::state::AppState;

/// Pure admin-policy check: `true` when `email` (case-insensitive) is present
/// in the configured admin set. No DB or IO, so it is directly unit-testable.
/// Only the user-session auth branch calls this; node and service-account
/// principals are never admin, by construction.
pub(crate) fn is_admin_email(email: Option<&str>, admin_emails: &HashSet<String>) -> bool {
    email
        .map(|e| admin_emails.contains(&e.to_lowercase()))
        .unwrap_or(false)
}

/// Caller identity extracted from request headers.
///
/// Note: `auth_type` is one of `"session"`, `"service_account"`, or `"node"`.
/// The `"static"` EP_TOKEN path was removed — all callers authenticate via
/// OIDC session tokens (mcs_*), service-account tokens (mcs_sa_*), or
/// RS256 node JWTs. The extractor reads from request extensions (where the
/// `require_auth` middleware caches the resolved Principal) before falling
/// back to a full lookup, so handlers can still take `principal: Principal`
/// without a second DB round-trip per request.
#[derive(Clone)]
pub struct Principal {
    pub subject: String,
    pub is_admin: bool,
    pub session_id: Option<i32>,
    /// One of: "session", "service_account", "node"
    pub auth_type: String,
}

/// Rejection returned by the `Principal` extractor when no valid credential
/// is presented. Renders as a 401 with a JSON `{"detail": "..."}` body so
/// clients can distinguish auth failures from other 4xx classes.
pub enum AuthRejection {
    /// No bearer token, or the bearer didn't resolve to a known principal.
    Unauthenticated,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let detail = match self {
            AuthRejection::Unauthenticated => "authentication required",
        };
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"detail": detail})),
        )
            .into_response()
    }
}

impl FromRequestParts<Arc<AppState>> for Principal {
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Self::Rejection> {
        // Phase 1.6: the `require_auth` middleware resolves the principal
        // once per request and stashes it in extensions. Handlers that
        // extract `Principal` read it from there rather than re-running the
        // DB lookup. Fall back to the full lookup only when this extractor
        // is invoked outside the middleware path (tests, niche call sites).
        if let Some(p) = parts.extensions.get::<Principal>() {
            return Ok(p.clone());
        }
        let bearer = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|s| s.trim().to_string());

        // Also accept ep_session_token cookie (set by browser OIDC flow).
        let cookie_token = parts
            .headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                s.split(';').find_map(|pair| {
                    pair.trim()
                        .strip_prefix("ep_session_token=")
                        .map(|v| v.trim().to_string())
                })
            });

        // Bearer takes priority; fall back to cookie session.
        let token_credential = bearer.clone().or(cookie_token);

        if let Some(ref token) = token_credential {
            let hash = hash_token(token);
            let now = chrono::Utc::now().naive_utc();

            // Node JWT — exactly two dots means RS256 JWT (never present in
            // opaque mcs_* tokens). Validate signature in-process; only then
            // hit the DB to confirm the JTI is not revoked.
            if token.matches('.').count() == 2 {
                if let Ok(claims) = crate::jwt::verify_node_jwt(token, &state.jwt_decoding_key) {
                    let row = sqlx::query(
                        "SELECT revoked FROM nodetoken WHERE jti=$1 AND expires_at > $2",
                    )
                    .bind(&claims.jti)
                    .bind(now)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten();

                    if let Some(row) = row {
                        let revoked: bool = row.get("revoked");
                        if !revoked {
                            return Ok(Principal {
                                subject: claims.sub,
                                is_admin: false,
                                session_id: None,
                                auth_type: "node".into(),
                            });
                        }
                    }
                }
                // JWT present but invalid/revoked/unknown — fall through to reject.
                return Err(AuthRejection::Unauthenticated);
            }

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
                    "SELECT id, subject, email FROM usersession \
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
                    let email: Option<String> = row.get("email");
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
                        is_admin: is_admin_email(email.as_deref(), &state.admin_emails),
                        session_id: Some(session_id),
                        auth_type: "session".into(),
                    });
                }
            }
        }

        // No valid credential — reject. Routes that legitimately accept
        // unauthenticated callers either don't extract `Principal` or use
        // `Option<Principal>` (which axum auto-derives from this rejection).
        Err(AuthRejection::Unauthenticated)
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

// ── Auth middleware (Phase 1.6) ────────────────────────────────────────────────

/// Routes that bypass the global authentication middleware. Adding a path
/// here makes it publicly callable — audit on every code review.
///
/// Each entry is documented inline so future readers know *why* it's public:
///   * `/health`, `/mcp/health`, `/mcp/tools` — meta endpoints; no state mutation.
///   * `/auth/oidc/*` — OIDC bootstrap; the only path to acquire a credential
///     before authentication exists.
///   * `/webhooks/tailscale`, `/integrations/.../events|commands|interactions` —
///     webhook receivers that verify their own per-event signatures
///     (signing secret check inside the handler).
///   * `/raft/status` — cluster status; intentionally observable.
///   * `/agent-onboarding.json`, `/schema-pack` — public manifests/docs.
pub fn is_public_path(path: &str) -> bool {
    matches!(
        path,
        "/"
            | "/health"
            | "/mcp/health"
            | "/mcp/tools"
            | "/raft/status"
            | "/agent-onboarding.json"
            | "/schema-pack"
            | "/webhooks/tailscale"
            | "/integrations/slack/events"
            | "/integrations/slack/commands"
            | "/integrations/slack/interactions"
            | "/integrations/teams/events"
            | "/integrations/google-chat/events"
    ) || path.starts_with("/auth/oidc/")
        || path == "/auth/logout"
        || path == "/runtime/nodes/register" // bootstrap — join token is the sole credential
}

/// Tower middleware that gates the entire app on authentication.
///
/// Behaviour:
///   * Path matches `is_public_path` → pass through, no auth performed.
///   * Otherwise → resolve `Principal`; on success, insert it into request
///     extensions so downstream handlers can read it via the `Principal`
///     extractor without re-doing the DB lookup. On failure return 401.
///
/// This is the single centralised auth boundary for the controlplane.
/// Per-handler `principal: Principal` extractions are still useful (and
/// cheap, since they read from extensions), because they double as inline
/// documentation that a route consults the caller's identity.
pub async fn require_auth(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = req.uri().path().to_string();
    if is_public_path(&path) {
        return next.run(req).await;
    }

    let (mut parts, body) = req.into_parts();
    match Principal::from_request_parts(&mut parts, &state).await {
        Ok(principal) => {
            parts.extensions.insert(principal);
            let req = axum::extract::Request::from_parts(parts, body);
            next.run(req).await
        }
        Err(rejection) => rejection.into_response(),
    }
}

#[cfg(test)]
mod public_path_tests {
    use super::is_public_path;

    #[test]
    fn meta_endpoints_are_public() {
        for p in &["/health", "/mcp/health", "/mcp/tools", "/raft/status"] {
            assert!(is_public_path(p), "{p} should be public");
        }
    }

    #[test]
    fn oidc_bootstrap_is_public() {
        for p in &[
            "/auth/oidc/cli-initiate",
            "/auth/oidc/cli-poll/abc123",
            "/auth/oidc/exchange",
            "/auth/oidc/callback",
        ] {
            assert!(is_public_path(p), "{p} should be public");
        }
    }

    #[test]
    fn webhook_receivers_are_public() {
        for p in &[
            "/webhooks/tailscale",
            "/integrations/slack/events",
            "/integrations/slack/commands",
            "/integrations/slack/interactions",
            "/integrations/teams/events",
            "/integrations/google-chat/events",
        ] {
            assert!(is_public_path(p), "{p} should be public");
        }
    }

    #[test]
    fn private_paths_are_not_public() {
        for p in &[
            "/agents",
            "/agents/4",
            "/agents/aria-work-e88c006e",
            "/domains",
            "/mcp/call",
            "/auth/me",
            "/auth/sessions",
            "/integrations/slack/channels", // admin path, not the webhook
            "/work/domains/m/agents/enroll",
        ] {
            assert!(!is_public_path(p), "{p} should NOT be public");
        }
    }
}

#[cfg(test)]
mod admin_email_tests {
    use super::is_admin_email;
    use std::collections::HashSet;

    fn admins() -> HashSet<String> {
        ["admin@example.com".to_string()].into_iter().collect()
    }

    #[test]
    fn listed_email_is_admin() {
        assert!(is_admin_email(Some("admin@example.com"), &admins()));
    }

    #[test]
    fn listed_email_is_case_insensitive() {
        assert!(is_admin_email(Some("Admin@Example.COM"), &admins()));
    }

    #[test]
    fn unlisted_email_is_not_admin() {
        assert!(!is_admin_email(Some("someone@example.com"), &admins()));
    }

    #[test]
    fn null_email_is_not_admin() {
        assert!(!is_admin_email(None, &admins()));
    }

    #[test]
    fn empty_admin_set_is_never_admin() {
        let empty: HashSet<String> = HashSet::new();
        assert!(!is_admin_email(Some("admin@example.com"), &empty));
    }
}
