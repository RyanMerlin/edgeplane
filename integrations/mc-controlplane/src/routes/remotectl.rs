use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Targets
        .route("/remotectl/targets", post(create_target).get(list_targets))
        .route(
            "/remotectl/targets/{target_id}",
            get(get_target).patch(update_target).delete(delete_target),
        )
        // Launches
        .route("/remotectl/launches", post(create_launch).get(list_launches))
        .route(
            "/remotectl/launches/{launch_id}",
            get(get_launch).delete(delete_launch),
        )
        .route("/remotectl/launches/{launch_id}/heartbeat", post(heartbeat_launch))
        .route("/remotectl/launches/{launch_id}/complete", post(complete_launch))
}

// ── Request body types ─────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TargetCreate {
    name: String,
    host: String,
    #[serde(default)]
    user: String,
    #[serde(default = "default_22")]
    port: i32,
    #[serde(default = "default_ssh")]
    transport: String,
    #[serde(default)]
    ssh_pubkey: String,
    #[serde(default)]
    key_fingerprint: String,
}

fn default_22() -> i32 {
    22
}
fn default_ssh() -> String {
    "ssh".to_string()
}

#[derive(serde::Deserialize)]
struct TargetUpdate {
    ssh_pubkey: Option<String>,
    key_fingerprint: Option<String>,
    last_used_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(serde::Deserialize)]
struct LaunchCreate {
    transport: String,
    target_id: Option<String>,
    #[serde(default)]
    target_host: String,
    #[serde(default)]
    target_namespace: String,
    agent_kind: String,
    #[serde(default)]
    agent_profile: String,
    #[serde(default)]
    capability_scope: Vec<String>,
    #[serde(default)]
    runtime_session_id: String,
    #[serde(default = "default_8")]
    ttl_hours: i64,
}

fn default_8() -> i64 {
    8
}

#[derive(serde::Deserialize, Default)]
struct HeartbeatUpdate {
    #[serde(default)]
    log_tail: String,
}

#[derive(serde::Deserialize)]
struct CompleteUpdate {
    exit_code: i32,
    #[serde(default)]
    error_message: String,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn row_to_target(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "name": row.get::<String, _>("name"),
        "host": row.get::<String, _>("host"),
        "user": row.get::<String, _>("user"),
        "port": row.get::<i32, _>("port"),
        "transport": row.get::<String, _>("transport"),
        "ssh_pubkey": row.get::<String, _>("ssh_pubkey"),
        "key_fingerprint": row.get::<String, _>("key_fingerprint"),
        "last_used_at": row.get::<Option<chrono::NaiveDateTime>, _>("last_used_at"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_launch(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "transport": row.get::<String, _>("transport"),
        "target_id": row.get::<Option<String>, _>("target_id"),
        "target_host": row.get::<String, _>("target_host"),
        "target_namespace": row.get::<String, _>("target_namespace"),
        "agent_kind": row.get::<String, _>("agent_kind"),
        "agent_profile": row.get::<String, _>("agent_profile"),
        "runtime_session_id": row.get::<String, _>("runtime_session_id"),
        "capability_scope": row.get::<String, _>("capability_scope"),
        "status": row.get::<String, _>("status"),
        "last_heartbeat_at": row.get::<Option<chrono::NaiveDateTime>, _>("last_heartbeat_at"),
        "exit_code": row.get::<Option<i32>, _>("exit_code"),
        "error_message": row.get::<String, _>("error_message"),
        "log_tail": row.get::<String, _>("log_tail"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

/// Issue a new usersession token. Returns (session_id, raw_token).
async fn issue_session_token(
    db: &sqlx::PgPool,
    subject: &str,
    ttl_hours: i64,
    capability_scope: &str,
) -> Result<(i32, String), sqlx::Error> {
    let mut raw_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw_bytes);
    let raw_token = format!("mcs_{}", hex::encode(&raw_bytes));
    let token_hash = hex::encode(Sha256::new().chain_update(raw_token.as_bytes()).finalize());
    let token_prefix = &raw_token[..8.min(raw_token.len())];
    let expires_at =
        chrono::Utc::now().naive_utc() + chrono::Duration::hours(ttl_hours);
    let now = chrono::Utc::now().naive_utc();

    let session_id: i32 = sqlx::query_scalar(
        "INSERT INTO usersession \
         (subject, token_hash, token_prefix, expires_at, created_at, last_used_at, user_agent, revoked, capability_scope) \
         VALUES ($1,$2,$3,$4,$5,$5,'mc-remotectl',false,$6) \
         RETURNING id",
    )
    .bind(subject)
    .bind(&token_hash)
    .bind(token_prefix)
    .bind(expires_at)
    .bind(now)
    .bind(capability_scope)
    .fetch_one(db)
    .await?;

    Ok((session_id, raw_token))
}

// ── Target handlers ────────────────────────────────────────────────────────────

async fn create_target(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<TargetCreate>,
) -> impl IntoResponse {
    // Check uniqueness: same owner + name
    let exists: Option<String> = sqlx::query_scalar(
        "SELECT id FROM remotetarget WHERE owner_subject=$1 AND name=$2",
    )
    .bind(&principal.subject)
    .bind(&body.name)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if exists.is_some() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"detail": "A target with that name already exists"})),
        )
            .into_response();
    }

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        r#"INSERT INTO remotetarget
           (id, owner_subject, name, host, "user", port, transport, ssh_pubkey, key_fingerprint,
            last_used_at, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,NULL,$10,$10)
           RETURNING *"#,
    )
    .bind(&id)
    .bind(&principal.subject)
    .bind(&body.name)
    .bind(&body.host)
    .bind(&body.user)
    .bind(body.port)
    .bind(&body.transport)
    .bind(&body.ssh_pubkey)
    .bind(&body.key_fingerprint)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => (StatusCode::CREATED, Json(row_to_target(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_target: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_targets(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT * FROM remotetarget WHERE owner_subject=$1 ORDER BY created_at DESC",
    )
    .bind(&principal.subject)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_target).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_targets: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_target(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(target_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query(
        "SELECT * FROM remotetarget WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&target_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await;

    match row {
        Ok(Some(r)) => Json(row_to_target(&r)).into_response(),
        Ok(None) => not_found("Target not found"),
        Err(e) => {
            tracing::error!("get_target: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_target(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(target_id): Path<String>,
    Json(body): Json<TargetUpdate>,
) -> impl IntoResponse {
    // Verify ownership
    let exists: Option<String> = sqlx::query_scalar(
        "SELECT id FROM remotetarget WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&target_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if exists.is_none() {
        return not_found("Target not found");
    }

    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        r#"UPDATE remotetarget SET
           ssh_pubkey      = COALESCE($3, ssh_pubkey),
           key_fingerprint = COALESCE($4, key_fingerprint),
           last_used_at    = COALESCE($5, last_used_at),
           updated_at      = $6
           WHERE id=$1 AND owner_subject=$2
           RETURNING *"#,
    )
    .bind(&target_id)
    .bind(&principal.subject)
    .bind(&body.ssh_pubkey)
    .bind(&body.key_fingerprint)
    .bind(body.last_used_at.map(|t| t.naive_utc()))
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => Json(row_to_target(&row)).into_response(),
        Err(e) => {
            tracing::error!("update_target: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_target(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(target_id): Path<String>,
) -> impl IntoResponse {
    let result =
        sqlx::query("DELETE FROM remotetarget WHERE id=$1 AND owner_subject=$2")
            .bind(&target_id)
            .bind(&principal.subject)
            .execute(&state.db)
            .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => not_found("Target not found"),
        Err(e) => {
            tracing::error!("delete_target: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Launch handlers ────────────────────────────────────────────────────────────

async fn create_launch(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<LaunchCreate>,
) -> impl IntoResponse {
    let capability_scope = body.capability_scope.join(",");

    let (session_id, raw_token) =
        match issue_session_token(&state.db, &principal.subject, body.ttl_hours, &capability_scope)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("create_launch issue_session_token: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    let id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        r#"INSERT INTO remotelaunchrecord
           (id, owner_subject, transport, target_id, target_host, target_namespace,
            agent_kind, agent_profile, runtime_session_id, session_token_id,
            capability_scope, status, last_heartbeat_at, exit_code, error_message,
            log_tail, mc_binary_path, agent_binary_path, k8s_job_name, mc_version,
            created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'launching',NULL,NULL,'','','','','','',
                   $12,$12)
           RETURNING *"#,
    )
    .bind(&id)
    .bind(&principal.subject)
    .bind(&body.transport)
    .bind(&body.target_id)
    .bind(&body.target_host)
    .bind(&body.target_namespace)
    .bind(&body.agent_kind)
    .bind(&body.agent_profile)
    .bind(&body.runtime_session_id)
    .bind(session_id)
    .bind(&capability_scope)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => {
            let mut launch = row_to_launch(&row);
            launch["session_token"] = serde_json::Value::String(raw_token);
            (StatusCode::CREATED, Json(launch)).into_response()
        }
        Err(e) => {
            tracing::error!("create_launch insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_launches(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT * FROM remotelaunchrecord WHERE owner_subject=$1 ORDER BY created_at DESC",
    )
    .bind(&principal.subject)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_launch).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_launches: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_launch(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(launch_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query(
        "SELECT * FROM remotelaunchrecord WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&launch_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await;

    match row {
        Ok(Some(r)) => {
            let mut launch = row_to_launch(&r);
            // Compute heartbeat_age_seconds
            let heartbeat_age =
                r.get::<Option<chrono::NaiveDateTime>, _>("last_heartbeat_at")
                    .map(|hb| {
                        let now = Utc::now().naive_utc();
                        (now - hb).num_seconds()
                    });
            launch["heartbeat_age_seconds"] = match heartbeat_age {
                Some(age) => serde_json::Value::Number(age.into()),
                None => serde_json::Value::Null,
            };
            Json(launch).into_response()
        }
        Ok(None) => not_found("Launch not found"),
        Err(e) => {
            tracing::error!("get_launch: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn heartbeat_launch(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(launch_id): Path<String>,
    Json(body): Json<HeartbeatUpdate>,
) -> impl IntoResponse {
    // Fetch current row to read status
    let current = sqlx::query(
        "SELECT status, log_tail FROM remotelaunchrecord WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&launch_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await;

    let (current_status, current_log) = match current {
        Ok(Some(r)) => (
            r.get::<String, _>("status"),
            r.get::<String, _>("log_tail"),
        ),
        Ok(None) => return not_found("Launch not found"),
        Err(e) => {
            tracing::error!("heartbeat_launch fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Transition launching → running on first heartbeat
    let new_status = if current_status == "launching" {
        "running"
    } else {
        &current_status
    };

    // Merge log_tail: append new tail, keep last 4096 chars
    let merged_log = if body.log_tail.is_empty() {
        current_log
    } else {
        let combined = format!("{}{}", current_log, body.log_tail);
        let len = combined.len();
        if len > 4096 {
            combined[len - 4096..].to_string()
        } else {
            combined
        }
    };

    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        "UPDATE remotelaunchrecord \
         SET status=$3, last_heartbeat_at=$4, log_tail=$5, updated_at=$4 \
         WHERE id=$1 AND owner_subject=$2 \
         RETURNING *",
    )
    .bind(&launch_id)
    .bind(&principal.subject)
    .bind(new_status)
    .bind(now)
    .bind(&merged_log)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => Json(row_to_launch(&row)).into_response(),
        Err(e) => {
            tracing::error!("heartbeat_launch update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn complete_launch(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(launch_id): Path<String>,
    Json(body): Json<CompleteUpdate>,
) -> impl IntoResponse {
    // Fetch session_token_id for revocation
    let current = sqlx::query(
        "SELECT session_token_id FROM remotelaunchrecord WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&launch_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await;

    let session_token_id: Option<i32> = match current {
        Ok(Some(r)) => r.get::<Option<i32>, _>("session_token_id"),
        Ok(None) => return not_found("Launch not found"),
        Err(e) => {
            tracing::error!("complete_launch fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let now = Utc::now().naive_utc();
    let status = if body.exit_code == 0 { "completed" } else { "failed" };

    let result = sqlx::query(
        "UPDATE remotelaunchrecord \
         SET status=$3, exit_code=$4, error_message=$5, updated_at=$6 \
         WHERE id=$1 AND owner_subject=$2 \
         RETURNING *",
    )
    .bind(&launch_id)
    .bind(&principal.subject)
    .bind(status)
    .bind(body.exit_code)
    .bind(&body.error_message)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    let row = match result {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("complete_launch update: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Revoke the session token
    if let Some(sid) = session_token_id {
        let _ = sqlx::query("UPDATE usersession SET revoked=true WHERE id=$1")
            .bind(sid)
            .execute(&state.db)
            .await;
    }

    Json(row_to_launch(&row)).into_response()
}

async fn delete_launch(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(launch_id): Path<String>,
) -> impl IntoResponse {
    // Fetch session_token_id for revocation
    let current = sqlx::query(
        "SELECT session_token_id FROM remotelaunchrecord WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&launch_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await;

    let session_token_id: Option<i32> = match current {
        Ok(Some(r)) => r.get::<Option<i32>, _>("session_token_id"),
        Ok(None) => return not_found("Launch not found"),
        Err(e) => {
            tracing::error!("delete_launch fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let now = Utc::now().naive_utc();

    // Mark as failed / killed
    let update = sqlx::query(
        "UPDATE remotelaunchrecord \
         SET status='failed', error_message='killed by owner', updated_at=$3 \
         WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&launch_id)
    .bind(&principal.subject)
    .bind(now)
    .execute(&state.db)
    .await;

    if let Err(e) = update {
        tracing::error!("delete_launch update: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Revoke the session token
    if let Some(sid) = session_token_id {
        let _ = sqlx::query("UPDATE usersession SET revoked=true WHERE id=$1")
            .bind(sid)
            .execute(&state.db)
            .await;
    }

    StatusCode::NO_CONTENT.into_response()
}
