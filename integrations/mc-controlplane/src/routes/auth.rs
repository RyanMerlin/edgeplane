use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{Duration, Utc};
use sqlx::Row;
use std::sync::Arc;

use crate::{
    auth::{hash_token, make_token, Principal},
    models::auth::{
        MeResponse, RevokeTokenRequest, ServiceAccountCreateRequest, ServiceAccountCreatedResponse,
        SessionCreateRequest, SessionResponse, TokenRequest, TokenResponse,
    },
    state::AppState,
};

const SESSION_PREFIX: &str = "mcs_";
const SA_TOKEN_PREFIX: &str = "mcs_sa_";
const CS_PREFIX: &str = "mcs_cs_";
const DEFAULT_TTL_HOURS: i64 = 8;
const MAX_TTL_HOURS: i64 = 720; // 30 days

fn resolve_ttl(requested: Option<i64>) -> i64 {
    let env_ttl: i64 = std::env::var("MC_SESSION_TTL_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TTL_HOURS);
    requested.unwrap_or(env_ttl).max(1).min(MAX_TTL_HOURS)
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/sessions", post(create_session))
        .route("/auth/sessions/refresh", post(refresh_session))
        .route("/auth/sessions/current", delete(revoke_session))
        .route("/auth/me", get(whoami))
        .route("/auth/whoami", get(whoami))
        .route(
            "/auth/service-accounts",
            get(list_service_accounts).post(create_service_account),
        )
        .route("/auth/service-accounts/{id}", delete(revoke_service_account))
        .route("/auth/token", post(client_credentials_grant))
        .route("/auth/token/revoke", post(revoke_sa_token))
}

// ── User Sessions ─────────────────────────────────────────────────────────────

async fn create_session(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<SessionCreateRequest>,
) -> impl IntoResponse {
    if principal.auth_type == "anonymous" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"detail": "authentication required to create a session"})),
        )
            .into_response();
    }

    let ttl = resolve_ttl(payload.ttl_hours);
    let token = make_token(SESSION_PREFIX);
    let token_hash = hash_token(&token);
    let token_prefix = token[..token.len().min(12)].to_string();
    let now = Utc::now().naive_utc();
    let expires_at = now + Duration::hours(ttl);

    let result = sqlx::query_scalar::<_, i32>(
        "INSERT INTO usersession \
         (subject, token_hash, token_prefix, expires_at, created_at, last_used_at, user_agent, revoked, capability_scope) \
         VALUES ($1,$2,$3,$4,$5,$5,'',false,'') RETURNING id",
    )
    .bind(&principal.subject)
    .bind(&token_hash)
    .bind(&token_prefix)
    .bind(expires_at)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(session_id) => Json(SessionResponse {
            token,
            subject: principal.subject,
            expires_at,
            session_id,
            ttl_hours: ttl,
        })
        .into_response(),
        Err(e) => {
            tracing::error!("create_session: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn refresh_session(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    if principal.auth_type != "session" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"detail": "only mcs_* session tokens can be refreshed"})),
        )
            .into_response();
    }
    let session_id = principal.session_id.unwrap();
    let ttl = resolve_ttl(None);
    let new_expires_at = Utc::now().naive_utc() + Duration::hours(ttl);

    match sqlx::query(
        "UPDATE usersession SET expires_at=$2, last_used_at=NOW() WHERE id=$1 AND revoked=false",
    )
    .bind(session_id)
    .bind(new_expires_at)
    .execute(&state.db)
    .await
    {
        Ok(_) => Json(SessionResponse {
            token: "(not re-issued on refresh)".into(),
            subject: principal.subject,
            expires_at: new_expires_at,
            session_id,
            ttl_hours: ttl,
        })
        .into_response(),
        Err(e) => {
            tracing::error!("refresh_session: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn revoke_session(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    if let Some(id) = principal.session_id {
        if let Err(e) = sqlx::query("UPDATE usersession SET revoked=true WHERE id=$1")
            .bind(id)
            .execute(&state.db)
            .await
        {
            tracing::error!("revoke_session: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn whoami(principal: Principal) -> impl IntoResponse {
    Json(MeResponse {
        subject: principal.subject,
        auth_type: principal.auth_type,
        session_id: principal.session_id,
    })
}

// ── Service Accounts ──────────────────────────────────────────────────────────

async fn create_service_account(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<ServiceAccountCreateRequest>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "admin token required to create service accounts"})),
        )
            .into_response();
    }

    let client_secret = make_token(CS_PREFIX);
    let client_secret_hash = hash_token(&client_secret);
    let client_secret_prefix = client_secret[..client_secret.len().min(12)].to_string();
    let now = Utc::now().naive_utc();

    let result = sqlx::query_scalar::<_, i32>(
        "INSERT INTO serviceaccount \
         (name, owner_subject, client_secret_hash, client_secret_prefix, created_at, revoked) \
         VALUES ($1,$2,$3,$4,$5,false) RETURNING id",
    )
    .bind(&payload.name)
    .bind(&principal.subject)
    .bind(&client_secret_hash)
    .bind(&client_secret_prefix)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(id) => (
            StatusCode::CREATED,
            Json(ServiceAccountCreatedResponse {
                id,
                name: payload.name,
                client_id: id,
                client_secret,
                created_at: now,
            }),
        )
            .into_response(),
        Err(e) if e.to_string().contains("unique") || e.to_string().contains("duplicate") => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"detail": "service account name already exists"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("create_service_account: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_service_accounts(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    if !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "admin token required"})),
        )
            .into_response();
    }

    match sqlx::query(
        "SELECT id, name, owner_subject, client_secret_prefix, created_at, revoked \
         FROM serviceaccount ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            let accounts: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<i32, _>("id"),
                        "name": r.get::<String, _>("name"),
                        "owner_subject": r.get::<String, _>("owner_subject"),
                        "client_secret_prefix": r.get::<String, _>("client_secret_prefix"),
                        "created_at": r.get::<chrono::NaiveDateTime, _>("created_at"),
                        "revoked": r.get::<bool, _>("revoked"),
                    })
                })
                .collect();
            Json(accounts).into_response()
        }
        Err(e) => {
            tracing::error!("list_service_accounts: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn revoke_service_account(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "admin token required"})),
        )
            .into_response();
    }

    match sqlx::query(
        "UPDATE serviceaccount SET revoked=true WHERE id=$1 AND revoked=false",
    )
    .bind(id)
    .execute(&state.db)
    .await
    {
        Ok(r) if r.rows_affected() == 0 => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"detail": "service account not found or already revoked"})),
        )
            .into_response(),
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("revoke_service_account: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Client Credentials Grant ─────────────────────────────────────────────────

async fn client_credentials_grant(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<TokenRequest>,
) -> impl IntoResponse {
    if let Some(ref gt) = payload.grant_type {
        if gt != "client_credentials" {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"detail": "unsupported grant_type, use client_credentials"})),
            )
                .into_response();
        }
    }

    let secret_hash = hash_token(&payload.client_secret);
    let sa_row = sqlx::query(
        "SELECT id FROM serviceaccount WHERE id=$1 AND client_secret_hash=$2 AND revoked=false",
    )
    .bind(payload.client_id)
    .bind(&secret_hash)
    .fetch_optional(&state.db)
    .await;

    match sa_row {
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"detail": "invalid client_id or client_secret"})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("token grant SA lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    let token = make_token(SA_TOKEN_PREFIX);
    let token_hash = hash_token(&token);
    let token_prefix = token[..token.len().min(12)].to_string();
    let now = Utc::now().naive_utc();

    let (expires_at, expires_in_hours) = match payload.expires_in_hours {
        Some(h) => {
            let h = h.max(1).min(MAX_TTL_HOURS);
            (Some(now + Duration::hours(h)), Some(h))
        }
        None => (None, None),
    };

    let result = sqlx::query_scalar::<_, i32>(
        "INSERT INTO serviceaccounttoken \
         (service_account_id, token_hash, token_prefix, expires_at, created_at, revoked) \
         VALUES ($1,$2,$3,$4,$5,false) RETURNING id",
    )
    .bind(payload.client_id)
    .bind(&token_hash)
    .bind(&token_prefix)
    .bind(expires_at)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(token_id) => Json(TokenResponse {
            access_token: token,
            token_type: "Bearer".into(),
            expires_at,
            expires_in_hours,
            service_account_id: payload.client_id,
            token_id,
        })
        .into_response(),
        Err(e) => {
            tracing::error!("token grant insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn revoke_sa_token(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<RevokeTokenRequest>,
) -> impl IntoResponse {
    if principal.auth_type == "anonymous" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"detail": "authentication required"})),
        )
            .into_response();
    }

    let result = if let Some(ref token) = payload.token {
        let hash = hash_token(token);
        sqlx::query(
            "UPDATE serviceaccounttoken SET revoked=true WHERE token_hash=$1 AND revoked=false",
        )
        .bind(&hash)
        .execute(&state.db)
        .await
    } else if let Some(id) = payload.token_id {
        sqlx::query(
            "UPDATE serviceaccounttoken SET revoked=true WHERE id=$1 AND revoked=false",
        )
        .bind(id)
        .execute(&state.db)
        .await
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"detail": "provide token or token_id"})),
        )
            .into_response();
    };

    match result {
        Ok(r) if r.rows_affected() == 0 => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"detail": "token not found or already revoked"})),
        )
            .into_response(),
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("revoke_sa_token: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
