use axum::{
    extract::{Path, Query, State},
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::{broadcast, Mutex as TokioMutex};
use base64::Engine;
use chrono::Utc;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::{auth::Principal, state::AppState};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn hash_token_local(token: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn make_token_local() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn json_dump(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

fn config_hash(config: &serde_json::Value) -> String {
    hash_token_local(&serde_json::to_string(config).unwrap_or_default())
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn attach_token_prefix() -> String {
    format!("{:x}", rand::random::<u32>())
}

// ── Row converters ────────────────────────────────────────────────────────────

fn row_to_node(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "node_name": row.get::<String, _>("node_name"),
        "hostname": row.get::<String, _>("hostname"),
        "status": row.get::<String, _>("status"),
        "trust_tier": row.get::<String, _>("trust_tier"),
        "labels": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("labels_json")).unwrap_or(serde_json::json!({})),
        "capacity": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("capacity_json")).unwrap_or(serde_json::json!({})),
        "capabilities": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("capabilities_json")).unwrap_or(serde_json::json!([])),
        "runtime_version": row.get::<String, _>("runtime_version"),
        "tailscale_ip": row.get::<Option<String>, _>("tailscale_ip"),
        "tailscale_fqdn": row.get::<Option<String>, _>("tailscale_fqdn"),
        "last_heartbeat_at": row.get::<Option<chrono::NaiveDateTime>, _>("last_heartbeat_at"),
        "registered_at": row.get::<chrono::NaiveDateTime, _>("registered_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_job(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "domain_id": row.get::<String, _>("domain_id"),
        "task_id": row.get::<Option<i32>, _>("task_id"),
        "runtime_session_id": row.get::<String, _>("runtime_session_id"),
        "runtime_class": row.get::<String, _>("runtime_class"),
        "image": row.get::<String, _>("image"),
        "command": row.get::<String, _>("command"),
        "args": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("args_json")).unwrap_or(serde_json::json!([])),
        "env": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("env_json")).unwrap_or(serde_json::json!({})),
        "cwd": row.get::<String, _>("cwd"),
        "mounts": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("mounts_json")).unwrap_or(serde_json::json!([])),
        "artifact_rules": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("artifact_rules_json")).unwrap_or(serde_json::json!({})),
        "timeout_seconds": row.get::<i32, _>("timeout_seconds"),
        "restart_policy": row.get::<String, _>("restart_policy"),
        "required_capabilities": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("required_capabilities_json")).unwrap_or(serde_json::json!([])),
        "preferred_labels": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("preferred_labels_json")).unwrap_or(serde_json::json!({})),
        "status": row.get::<String, _>("status"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_lease(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "job_id": row.get::<String, _>("job_id"),
        "node_id": row.get::<String, _>("node_id"),
        "status": row.get::<String, _>("status"),
        "claimed_at": row.get::<chrono::NaiveDateTime, _>("claimed_at"),
        "heartbeat_at": row.get::<Option<chrono::NaiveDateTime>, _>("heartbeat_at"),
        "started_at": row.get::<Option<chrono::NaiveDateTime>, _>("started_at"),
        "finished_at": row.get::<Option<chrono::NaiveDateTime>, _>("finished_at"),
        "exit_code": row.get::<Option<i32>, _>("exit_code"),
        "error_message": row.get::<String, _>("error_message"),
        "cleanup_status": row.get::<String, _>("cleanup_status"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_spec(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "node_id": row.get::<String, _>("node_id"),
        "config": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("config_json")).unwrap_or(serde_json::json!({})),
        "desired_version": row.get::<String, _>("desired_version"),
        "upgrade_channel": row.get::<String, _>("upgrade_channel"),
        "drain_state": row.get::<String, _>("drain_state"),
        "health_summary": row.get::<String, _>("health_summary"),
        "config_hash": row.get::<String, _>("config_hash"),
        "last_reconcile_at": row.get::<chrono::NaiveDateTime, _>("last_reconcile_at"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_join_token(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "node_id": row.get::<Option<String>, _>("node_id"),
        "upgrade_channel": row.get::<String, _>("upgrade_channel"),
        "desired_version": row.get::<String, _>("desired_version"),
        "status": row.get::<String, _>("status"),
        "expires_at": row.get::<Option<chrono::NaiveDateTime>, _>("expires_at"),
        "used_at": row.get::<Option<chrono::NaiveDateTime>, _>("used_at"),
        "rotation_count": row.get::<i32, _>("rotation_count"),
        "config": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("config_json")).unwrap_or(serde_json::json!({})),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_execution_session(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "lease_id": row.get::<String, _>("lease_id"),
        "runtime_class": row.get::<String, _>("runtime_class"),
        "pty_requested": row.get::<bool, _>("pty_requested"),
        "attach_token_prefix": row.get::<String, _>("attach_token_prefix"),
        "status": row.get::<String, _>("status"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

// ── ensure_node_spec ──────────────────────────────────────────────────────────

async fn ensure_node_spec(
    db: &sqlx::PgPool,
    subject: &str,
    node_id: &str,
    node_name: &str,
    trust_tier: &str,
    labels_json: &str,
    _runtime_version: &str,
    desired_version: &str,
    upgrade_channel: &str,
) -> Result<sqlx::postgres::PgRow, sqlx::Error> {
    if let Some(row) =
        sqlx::query("SELECT * FROM runtimenodespec WHERE node_id=$1 AND owner_subject=$2")
            .bind(node_id)
            .bind(subject)
            .fetch_optional(db)
            .await?
    {
        return Ok(row);
    }
    let config = serde_json::json!({
        "node_name": node_name,
        "trust_tier": trust_tier,
        "labels": serde_json::from_str::<serde_json::Value>(labels_json).unwrap_or(serde_json::json!({})),
    });
    let config_str = json_dump(&config);
    let ch = config_hash(&config);
    let now = Utc::now().naive_utc();
    let row = sqlx::query(
        "INSERT INTO runtimenodespec \
         (owner_subject, node_id, config_json, desired_version, upgrade_channel, drain_state, health_summary, config_hash, last_reconcile_at, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,'active','',$6,$7,$7,$7) RETURNING *",
    )
    .bind(subject)
    .bind(node_id)
    .bind(&config_str)
    .bind(desired_version)
    .bind(upgrade_channel)
    .bind(&ch)
    .bind(now)
    .fetch_one(db)
    .await?;
    Ok(row)
}

// ── mutate_node_spec_state ────────────────────────────────────────────────────

async fn mutate_node_spec_state(
    state: Arc<AppState>,
    node_id: String,
    subject: String,
    drain_state: &str,
) -> axum::response::Response {
    let now = Utc::now().naive_utc();

    // Fetch node and verify ownership
    let node_row = match sqlx::query(
        "SELECT * FROM runtimenode WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&node_id)
    .bind(&subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("node not found"),
        Err(e) => {
            tracing::error!("mutate_node_spec_state fetch node: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let node_name: String = node_row.get("node_name");
    let trust_tier: String = node_row.get("trust_tier");
    let labels_json: String = node_row.get("labels_json");
    let runtime_version: String = node_row.get("runtime_version");

    // Ensure spec exists
    let spec_row = match ensure_node_spec(
        &state.db,
        &subject,
        &node_id,
        &node_name,
        &trust_tier,
        &labels_json,
        &runtime_version,
        "",
        "stable",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("mutate_node_spec_state ensure_spec: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let spec_id: i32 = spec_row.get("id");

    // Update drain_state
    if let Err(e) = sqlx::query(
        "UPDATE runtimenodespec SET drain_state=$1, updated_at=$2 WHERE id=$3",
    )
    .bind(drain_state)
    .bind(now)
    .bind(spec_id)
    .execute(&state.db)
    .await
    {
        tracing::error!("mutate_node_spec_state update: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Insert NodeEvent
    let payload = json_dump(&serde_json::json!({"drain_state": drain_state}));
    let _ = sqlx::query(
        "INSERT INTO nodeevent (node_id, lease_id, event_type, payload_json, created_at) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Some(&node_id))
    .bind(None::<String>)
    .bind(format!("node.spec.{drain_state}"))
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await;

    // Fetch updated spec
    match sqlx::query("SELECT * FROM runtimenodespec WHERE id=$1")
        .bind(spec_id)
        .fetch_one(&state.db)
        .await
    {
        Ok(row) => Json(serde_json::json!({"spec": row_to_spec(&row)})).into_response(),
        Err(e) => {
            tracing::error!("mutate_node_spec_state refetch: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct JoinTokenCreate {
    #[serde(default = "default_3600_i64")]
    expires_in_seconds: i64,
    #[serde(default = "default_stable")]
    upgrade_channel: String,
    #[serde(default)]
    desired_version: String,
    #[serde(default)]
    config: serde_json::Value,
}
fn default_3600_i64() -> i64 {
    3600
}
fn default_stable() -> String {
    "stable".to_string()
}

#[derive(serde::Deserialize)]
struct NodeRegister {
    node_name: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_untrusted")]
    trust_tier: String,
    #[serde(default)]
    labels: serde_json::Value,
    #[serde(default)]
    capacity: serde_json::Value,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    runtime_version: String,
    #[serde(default)]
    bootstrap_token: String,
    #[serde(default)]
    tailscale_ip: Option<String>,
    #[serde(default)]
    tailscale_fqdn: Option<String>,
}
fn default_untrusted() -> String {
    "untrusted".to_string()
}

#[derive(serde::Deserialize)]
struct NodeHeartbeat {
    #[serde(default = "default_online")]
    status: String,
    labels: Option<serde_json::Value>,
    capacity: Option<serde_json::Value>,
    capabilities: Option<Vec<String>>,
    runtime_version: Option<String>,
    #[serde(default)]
    tailscale_ip: Option<String>,
    #[serde(default)]
    tailscale_fqdn: Option<String>,
}
fn default_online() -> String {
    "online".to_string()
}

#[derive(serde::Deserialize)]
struct JobCreate {
    #[serde(default)]
    domain_id: String,
    task_id: Option<i32>,
    #[serde(default)]
    runtime_session_id: String,
    #[serde(default = "default_container")]
    runtime_class: String,
    #[serde(default)]
    image: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: serde_json::Value,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    mounts: Vec<serde_json::Value>,
    #[serde(default)]
    artifact_rules: serde_json::Value,
    #[serde(default = "default_3600")]
    timeout_seconds: i32,
    #[serde(default = "default_never")]
    restart_policy: String,
    #[serde(default)]
    required_capabilities: Vec<String>,
    #[serde(default)]
    preferred_labels: serde_json::Value,
}
fn default_container() -> String {
    "container".to_string()
}
fn default_3600() -> i32 {
    3600
}
fn default_never() -> String {
    "never".to_string()
}

#[derive(serde::Deserialize)]
struct LeaseStatus {
    status: String,
    heartbeat_at: Option<chrono::DateTime<chrono::Utc>>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(serde::Deserialize)]
struct LeaseComplete {
    #[serde(default)]
    exit_code: i32,
    #[serde(default)]
    error_message: String,
}

#[derive(serde::Deserialize)]
struct ExecutionSessionCreate {
    lease_id: String,
    #[serde(default = "default_container")]
    runtime_class: String,
    #[serde(default)]
    pty_requested: bool,
}

#[derive(serde::Deserialize)]
struct NodeReconcile {
    drain_state: Option<String>,
    desired_version: Option<String>,
    health_summary: Option<String>,
}

#[derive(serde::Deserialize)]
struct ListQuery {
    status: Option<String>,
    limit: Option<i64>,
}

#[derive(serde::Deserialize)]
struct AppendLogsBody {
    #[serde(default)]
    logs: String,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Join tokens — both path aliases
        .route("/runtime/tokens", post(create_join_token))
        .route("/runtime/join-tokens", post(create_join_token))
        .route("/runtime/tokens/{token_id}", get(get_join_token))
        .route("/runtime/join-tokens/{token_id}", get(get_join_token))
        .route(
            "/runtime/tokens/{token_id}/rotate",
            post(rotate_join_token),
        )
        .route(
            "/runtime/join-tokens/{token_id}/rotate",
            post(rotate_join_token),
        )
        // Releases
        .route("/runtime/releases/latest.json", get(get_release_manifest))
        .route(
            "/runtime/releases/latest/download",
            get(download_release),
        )
        // Channels
        .route("/runtime/channels", get(list_channels))
        // Node operations — static route BEFORE dynamic {node_id} routes
        .route("/runtime/nodes/register", post(register_node))
        .route("/runtime/nodes", get(list_nodes))
        .route("/runtime/nodes/{node_id}/rotate-token", post(rotate_node_token))
        .route("/runtime/nodes/{node_id}/heartbeat", post(heartbeat_node))
        .route("/runtime/nodes/{node_id}/config", get(get_node_config))
        .route(
            "/runtime/nodes/{node_id}/install-bundle",
            get(get_node_install_bundle),
        )
        .route(
            "/runtime/nodes/{node_id}/install-script",
            get(get_node_install_script),
        )
        .route(
            "/runtime/nodes/{node_id}/reconcile",
            post(reconcile_node),
        )
        .route("/runtime/nodes/{node_id}/cordon", post(cordon_node))
        .route("/runtime/nodes/{node_id}/drain", post(drain_node))
        .route("/runtime/nodes/{node_id}/upgrade", post(upgrade_node))
        // Node leases
        .route(
            "/runtime/nodes/{node_id}/leases/claim",
            post(claim_lease),
        )
        // Jobs
        .route("/runtime/jobs", get(list_jobs).post(create_job))
        .route("/runtime/jobs/{job_id}/leases", post(create_lease))
        // Leases
        .route("/runtime/leases/{lease_id}", get(get_lease))
        .route(
            "/runtime/leases/{lease_id}/status",
            post(update_lease_status),
        )
        .route(
            "/runtime/leases/{lease_id}/complete",
            post(complete_lease),
        )
        .route("/runtime/leases/{lease_id}/logs", post(append_lease_logs))
        // Execution sessions
        .route(
            "/runtime/execution-sessions",
            post(create_execution_session),
        )
        .route(
            "/runtime/execution-sessions/{session_id}/attach-token",
            get(get_attach_token),
        )
        .route(
            "/runtime/execution-sessions/{session_id}/attach",
            post(attach_execution_session),
        )
        .route(
            "/runtime/execution-sessions/{session_id}/pty",
            get(execution_session_pty),
        )
        // Mesh agent attach proxy: browser WS → controlplane → edgeplaned node WS.
        // The browser uses ?ep_token=<bearer> for auth; controlplane signs a
        // short-lived HMAC token to dial the mesh node.
        .route(
            "/runtime/nodes/{node_id}/agents/{agent_id}/attach",
            get(agent_attach_proxy),
        )
        // Phase 4a — controlplane-driven enrollment for edgeplaned.
        // List meshagent rows assigned to this runtime node; daemons call
        // this on startup and after assignment-change notifications to pick
        // up new/removed/reassigned agents.
        // Phase 6 adds POST (assign) + DELETE (revoke) alongside.
        .route(
            "/runtime/nodes/{node_id}/agents",
            get(list_node_agents).post(assign_node_agent),
        )
        .route(
            "/runtime/nodes/{node_id}/agents/{agent_id}",
            delete(revoke_node_agent),
        )
        // WS — daemons subscribe here to receive `agent.assigned` /
        // `agent.unassigned` / `agent.reassigned` notifications for their
        // node, so they can rebalance supervisors live without polling or
        // restart.
        .route(
            "/runtime/nodes/{node_id}/notify",
            get(node_notify_ws),
        )
}

// ── Join tokens ───────────────────────────────────────────────────────────────

async fn create_join_token(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<JoinTokenCreate>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();
    let token = make_token_local();
    let token_hash = hash_token_local(&token);
    let token_id = Uuid::new_v4().to_string();

    let expires_at = if body.expires_in_seconds > 0 {
        Some(now + chrono::Duration::seconds(body.expires_in_seconds))
    } else {
        None
    };

    let config_str = json_dump(&body.config);

    match sqlx::query(
        "INSERT INTO runtimejointoken \
         (id, owner_subject, token_hash, config_json, upgrade_channel, desired_version, \
          expires_at, used_at, status, rotation_count, node_id, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,NULL,'active',0,NULL,$8,$8) RETURNING *",
    )
    .bind(&token_id)
    .bind(&principal.subject)
    .bind(&token_hash)
    .bind(&config_str)
    .bind(&body.upgrade_channel)
    .bind(&body.desired_version)
    .bind(expires_at)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => {
            let mut resp = row_to_join_token(&row);
            // Include the raw token once at creation time
            resp["token"] = serde_json::Value::String(token);
            (StatusCode::CREATED, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!("create_join_token: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_join_token(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(token_id): Path<String>,
) -> impl IntoResponse {
    match sqlx::query(
        "SELECT * FROM runtimejointoken WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&token_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => Json(row_to_join_token(&row)).into_response(),
        Ok(None) => not_found("join token not found"),
        Err(e) => {
            tracing::error!("get_join_token: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn rotate_join_token(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(token_id): Path<String>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // Fetch existing token
    let existing = match sqlx::query(
        "SELECT * FROM runtimejointoken WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&token_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("join token not found"),
        Err(e) => {
            tracing::error!("rotate_join_token fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let rotation_count: i32 = existing.get("rotation_count");
    let new_token = make_token_local();
    let new_hash = hash_token_local(&new_token);

    match sqlx::query(
        "UPDATE runtimejointoken \
         SET token_hash=$1, rotation_count=$2, status='active', used_at=NULL, updated_at=$3 \
         WHERE id=$4 RETURNING *",
    )
    .bind(&new_hash)
    .bind(rotation_count + 1)
    .bind(now)
    .bind(&token_id)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => {
            let mut resp = row_to_join_token(&row);
            resp["token"] = serde_json::Value::String(new_token);
            Json(resp).into_response()
        }
        Err(e) => {
            tracing::error!("rotate_join_token update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Release / channel endpoints ───────────────────────────────────────────────

async fn get_release_manifest() -> impl IntoResponse {
    let version = std::env::var("EP_RUNTIME_RELEASE_VERSION")
        .unwrap_or_else(|_| "0.2.0".to_string());
    let base_url = std::env::var("EP_RUNTIME_RELEASE_BASE_URL")
        .unwrap_or_else(|_| {
            "https://github.com/RyanMerlin/edgeplane/releases/latest/download"
                .to_string()
        });
    Json(serde_json::json!({
        "version": version,
        "files": [
            {"os": "linux", "arch": "x86_64", "url": format!("{base_url}/edgeplane-linux-x86_64"), "sha256": null}
        ]
    }))
}

async fn download_release() -> impl IntoResponse {
    let base_url = std::env::var("EP_RUNTIME_RELEASE_BASE_URL")
        .unwrap_or_else(|_| {
            "https://github.com/RyanMerlin/edgeplane/releases/latest/download"
                .to_string()
        });
    axum::response::Redirect::temporary(&format!("{base_url}/edgeplane-linux-x86_64"))
}

async fn list_channels() -> impl IntoResponse {
    Json(serde_json::json!({
        "channels": [
            {"name": "stable"},
            {"name": "latest"},
            {"name": "testing"}
        ]
    }))
}

// ── Node registration ─────────────────────────────────────────────────────────

// `slug_hostname` + `provision_home_for_node` removed in 0.15.9. The per-node
// `home-{hostname}` domain pattern + `Domain.kind='home'` write was walked
// back across 0.15.4 / 0.15.8; the bootstrap module in edgeplaned now ensures a
// single global `home` domain instead. This helper had zero remaining callers
// after the cleanup.

async fn register_node(
    State(state): State<Arc<AppState>>,
    // No Principal required — the join token IS the credential.
    // owner_subject is extracted from the join token row itself.
    Json(body): Json<NodeRegister>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // 1. Hash the bootstrap token and look it up (no owner_subject filter —
    //    the token is self-authenticating; its owner becomes the node's owner).
    let token_hash = hash_token_local(&body.bootstrap_token);
    let token_row = match sqlx::query(
        "SELECT * FROM runtimejointoken WHERE token_hash=$1 AND status='active'",
    )
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("register_node token lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let token_row = match token_row {
        Some(r) => r,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"detail": "Invalid bootstrap token"})),
            )
                .into_response()
        }
    };

    // 2. Check expiry
    let expires_at: Option<chrono::NaiveDateTime> = token_row.get("expires_at");
    if let Some(exp) = expires_at {
        if exp < now {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"detail": "Bootstrap token expired"})),
            )
                .into_response();
        }
    }

    // 3. Check already used
    let used_at: Option<chrono::NaiveDateTime> = token_row.get("used_at");
    if used_at.is_some() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"detail": "Bootstrap token already used"})),
        )
            .into_response();
    }

    let token_id: String = token_row.get("id");
    let subject: String = token_row.get("owner_subject");

    // 4. Check node_name uniqueness
    match sqlx::query("SELECT id FROM runtimenode WHERE node_name=$1")
        .bind(&body.node_name)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(_)) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"detail": "node_name already in use"})),
            )
                .into_response()
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!("register_node name check: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // 5. Create the node
    let node_id = Uuid::new_v4().to_string();
    let labels_json = json_dump(&body.labels);
    let capacity_json = json_dump(&body.capacity);
    let capabilities_json = serde_json::to_string(&body.capabilities)
        .unwrap_or_else(|_| "[]".to_string());
    // Mint a per-node attach secret. Returned plaintext once; stored in DB for
    // the controlplane proxy to sign short-lived HMAC tokens when dialing this
    // node's attach-WS endpoint. Not included in list/get node responses.
    let attach_secret = hex::encode(rand::random::<[u8; 32]>());

    let node_row = match sqlx::query(
        "INSERT INTO runtimenode \
         (id, owner_subject, node_name, hostname, status, trust_tier, labels_json, capacity_json, \
          capabilities_json, runtime_version, bootstrap_token_prefix, tailscale_ip, tailscale_fqdn, \
          attach_secret, last_heartbeat_at, registered_at, updated_at) \
         VALUES ($1,$2,$3,$4,'registered',$5,$6,$7,$8,$9,$10,$11,$12,$13,NULL,$14,$14) RETURNING *",
    )
    .bind(&node_id)
    .bind(&subject)
    .bind(&body.node_name)
    .bind(&body.hostname)
    .bind(&body.trust_tier)
    .bind(&labels_json)
    .bind(&capacity_json)
    .bind(&capabilities_json)
    .bind(&body.runtime_version)
    .bind(body.bootstrap_token.get(..8).unwrap_or(&body.bootstrap_token))
    .bind(&body.tailscale_ip)
    .bind(&body.tailscale_fqdn)
    .bind(&attach_secret)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("register_node insert: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // 6. Mark token used
    let _ = sqlx::query(
        "UPDATE runtimejointoken SET used_at=$1, status='used', node_id=$2, updated_at=$1 WHERE id=$3",
    )
    .bind(now)
    .bind(&node_id)
    .bind(&token_id)
    .execute(&state.db)
    .await;

    // 7. Insert NodeEvent
    let payload = json_dump(&serde_json::json!({"node_id": node_id, "node_name": body.node_name}));
    let _ = sqlx::query(
        "INSERT INTO nodeevent (node_id, lease_id, event_type, payload_json, created_at) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Some(&node_id))
    .bind(None::<String>)
    .bind("node.registered")
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await;

    // 8. Ensure node spec
    let _ = ensure_node_spec(
        &state.db,
        &subject,
        &node_id,
        &body.node_name,
        &body.trust_tier,
        &labels_json,
        &body.runtime_version,
        "",
        "stable",
    )
    .await;

    // 9. Issue RS256 node JWT (90-day TTL).
    let (node_jwt, jti) = match crate::jwt::sign_node_jwt(&node_id, &state.jwt_encoding_key, 90) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("register_node jwt sign: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let jwt_expires = now + chrono::Duration::days(90);
    let _ = sqlx::query(
        "INSERT INTO nodetoken (jti, node_id, revoked, issued_at, expires_at) VALUES ($1,$2,false,$3,$4)",
    )
    .bind(&jti)
    .bind(&node_id)
    .bind(now)
    .bind(jwt_expires)
    .execute(&state.db)
    .await;

    // Return node fields + attach_secret + node_jwt (plaintext, this response only).
    // Per-node home-domain auto-provisioning was removed in 0.15.9 — edgeplaned's
    // bootstrap module owns the single-global `home` domain now.
    let mut resp = row_to_node(&node_row);
    resp["attach_secret"] = serde_json::Value::String(attach_secret);
    resp["node_jwt"] = serde_json::Value::String(node_jwt);
    (StatusCode::CREATED, Json(resp)).into_response()
}

async fn rotate_node_token(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // Authorisation: the node itself (JWT subject = "node:{id}"), an admin,
    // or the owner user/service-account.
    let authorized = if principal.is_admin {
        true
    } else if principal.auth_type == "node" {
        principal.subject == format!("node:{node_id}")
    } else {
        matches!(
            sqlx::query("SELECT id FROM runtimenode WHERE id=$1 AND owner_subject=$2")
                .bind(&node_id)
                .bind(&principal.subject)
                .fetch_optional(&state.db)
                .await,
            Ok(Some(_))
        )
    };

    if !authorized {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "not authorized to rotate this node's token"})),
        )
            .into_response();
    }

    // Revoke all current active JTIs for this node then issue a fresh one.
    let _ = sqlx::query(
        "UPDATE nodetoken SET revoked=true, revoked_at=$1 WHERE node_id=$2 AND revoked=false",
    )
    .bind(now)
    .bind(&node_id)
    .execute(&state.db)
    .await;

    let (node_jwt, jti) = match crate::jwt::sign_node_jwt(&node_id, &state.jwt_encoding_key, 90) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::error!("rotate_node_token jwt sign: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let jwt_expires = now + chrono::Duration::days(90);
    let _ = sqlx::query(
        "INSERT INTO nodetoken (jti, node_id, revoked, issued_at, expires_at) VALUES ($1,$2,false,$3,$4)",
    )
    .bind(&jti)
    .bind(&node_id)
    .bind(now)
    .bind(jwt_expires)
    .execute(&state.db)
    .await;

    (StatusCode::OK, Json(serde_json::json!({"node_jwt": node_jwt}))).into_response()
}

async fn list_nodes(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    let rows = if let Some(ref s) = q.status {
        sqlx::query(
            "SELECT * FROM runtimenode WHERE owner_subject=$1 AND status=$2 \
             ORDER BY registered_at DESC LIMIT $3",
        )
        .bind(&principal.subject)
        .bind(s)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query(
            "SELECT * FROM runtimenode WHERE owner_subject=$1 \
             ORDER BY registered_at DESC LIMIT $2",
        )
        .bind(&principal.subject)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    };

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_node).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_nodes: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn heartbeat_node(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
    Json(body): Json<NodeHeartbeat>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // Verify ownership
    match sqlx::query(
        "SELECT id FROM runtimenode WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&node_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(None) => return not_found("node not found"),
        Err(e) => {
            tracing::error!("heartbeat_node verify: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    let labels_json = body
        .labels
        .as_ref()
        .map(|v| json_dump(v))
        .unwrap_or_else(|| "{}".to_string());
    let capacity_json = body
        .capacity
        .as_ref()
        .map(|v| json_dump(v))
        .unwrap_or_else(|| "{}".to_string());
    let capabilities_json = body
        .capabilities
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string()))
        .unwrap_or_else(|| "[]".to_string());
    let runtime_version = body.runtime_version.unwrap_or_default();

    let row = match sqlx::query(
        "UPDATE runtimenode SET status=$1, last_heartbeat_at=$2, updated_at=$2, \
         labels_json=CASE WHEN $3='' THEN labels_json ELSE $3 END, \
         capacity_json=CASE WHEN $4='' THEN capacity_json ELSE $4 END, \
         capabilities_json=CASE WHEN $5='[]' THEN capabilities_json ELSE $5 END, \
         runtime_version=CASE WHEN $6='' THEN runtime_version ELSE $6 END, \
         tailscale_ip=COALESCE($8, tailscale_ip), \
         tailscale_fqdn=COALESCE($9, tailscale_fqdn) \
         WHERE id=$7 RETURNING *",
    )
    .bind(&body.status)
    .bind(now)
    .bind(&labels_json)
    .bind(&capacity_json)
    .bind(&capabilities_json)
    .bind(&runtime_version)
    .bind(&node_id)
    .bind(&body.tailscale_ip)
    .bind(&body.tailscale_fqdn)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("heartbeat_node update: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let payload = json_dump(&serde_json::json!({"status": body.status}));
    let _ = sqlx::query(
        "INSERT INTO nodeevent (node_id, lease_id, event_type, payload_json, created_at) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Some(&node_id))
    .bind(None::<String>)
    .bind("node.heartbeat")
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await;

    Json(row_to_node(&row)).into_response()
}

// ── Node config / install ─────────────────────────────────────────────────────

async fn get_node_config(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    let node_row = match sqlx::query(
        "SELECT * FROM runtimenode WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&node_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("node not found"),
        Err(e) => {
            tracing::error!("get_node_config: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let node_name: String = node_row.get("node_name");
    let hostname: String = node_row.get("hostname");
    let trust_tier: String = node_row.get("trust_tier");
    let labels_json: String = node_row.get("labels_json");
    let capabilities_json: String = node_row.get("capabilities_json");
    let runtime_version: String = node_row.get("runtime_version");

    let spec_row = match ensure_node_spec(
        &state.db,
        &principal.subject,
        &node_id,
        &node_name,
        &trust_tier,
        &labels_json,
        &runtime_version,
        "",
        "stable",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("get_node_config ensure_spec: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let spec = row_to_spec(&spec_row);
    let mut config = spec["config"].clone();
    if let serde_json::Value::Object(ref mut m) = config {
        m.entry("node_name").or_insert(serde_json::Value::String(node_name));
        m.entry("hostname").or_insert(serde_json::Value::String(hostname));
        m.entry("trust_tier").or_insert(serde_json::Value::String(trust_tier));
        m.entry("labels").or_insert(
            serde_json::from_str::<serde_json::Value>(&labels_json)
                .unwrap_or(serde_json::json!({})),
        );
        m.entry("capabilities").or_insert(
            serde_json::from_str::<serde_json::Value>(&capabilities_json)
                .unwrap_or(serde_json::json!([])),
        );
        m.entry("upgrade_channel")
            .or_insert(spec["upgrade_channel"].clone());
        m.entry("desired_version")
            .or_insert(spec["desired_version"].clone());
    }

    Json(serde_json::json!({"node_id": node_id, "config": config, "spec": spec})).into_response()
}

fn build_install_script(base_url: &str, env_lines: &str) -> String {
    format!(
        r#"#!/bin/sh
set -eu
ep_bin='{base_url}/runtime/releases/latest/download'
if [ -n "$ep_bin" ]; then
  install -d /usr/local/bin
  curl -fsSL "$ep_bin" -o /usr/local/bin/edgeplane
  chmod 0755 /usr/local/bin/edgeplane
elif ! command -v edgeplane >/dev/null 2>&1; then
  echo '[ERROR] edgeplane binary not found and release artifact could not be resolved' >&2
  exit 1
fi
install -d /etc/edgeplane /etc/systemd/system
cat > /etc/edgeplane/edgeplane-node.service.env <<'EOF'
# Edgeplane node settings
{env_lines}
EOF
chmod 0600 /etc/edgeplane/edgeplane-node.service.env
cat > /etc/systemd/system/edgeplane-node.service <<'EOF'
[Unit]
Description=Edgeplane Node Agent
Wants=network-online.target
After=network-online.target

[Install]
WantedBy=multi-user.target

[Service]
Type=simple
User=root
Group=root
EnvironmentFile=-/etc/edgeplane/edgeplane-node.service.env
ExecStart=/usr/local/bin/edgeplane node run
Restart=always
RestartSec=5s
KillMode=control-group
TimeoutStartSec=0
LimitNOFILE=1048576
EOF
systemctl daemon-reload
systemctl enable --now edgeplane-node.service
"#,
        base_url = base_url,
        env_lines = env_lines,
    )
}

async fn get_node_install_bundle(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    let base_url = std::env::var("EP_RUNTIME_RELEASE_BASE_URL").unwrap_or_else(|_| {
        "https://github.com/RyanMerlin/edgeplane/releases/latest/download".to_string()
    });

    let node_row = match sqlx::query(
        "SELECT * FROM runtimenode WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&node_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("node not found"),
        Err(e) => {
            tracing::error!("get_node_install_bundle node: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let node_name: String = node_row.get("node_name");
    let hostname: String = node_row.get("hostname");
    let trust_tier: String = node_row.get("trust_tier");
    let labels_json: String = node_row.get("labels_json");
    let runtime_version: String = node_row.get("runtime_version");

    let spec_row = match ensure_node_spec(
        &state.db,
        &principal.subject,
        &node_id,
        &node_name,
        &trust_tier,
        &labels_json,
        &runtime_version,
        "",
        "stable",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("get_node_install_bundle spec: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let upgrade_channel: String = spec_row.get("upgrade_channel");
    let desired_version: String = spec_row.get("desired_version");

    // Find most recent join token for this node
    let token_row = sqlx::query(
        "SELECT * FROM runtimejointoken WHERE node_id=$1 AND owner_subject=$2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&node_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let token_id = token_row
        .as_ref()
        .map(|r| r.get::<String, _>("id"))
        .unwrap_or_default();

    // Build env dict
    let mut env: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    env.insert("EP_BASE_URL".into(), base_url.clone());
    env.insert("EP_NODE_NAME".into(), node_name.clone());
    env.insert("EP_NODE_HOSTNAME".into(), hostname.clone());
    env.insert("EP_NODE_TRUST_TIER".into(), trust_tier.clone());
    env.insert("EP_NODE_UPGRADE_CHANNEL".into(), upgrade_channel.clone());
    env.insert("EP_NODE_DESIRED_VERSION".into(), desired_version.clone());
    env.insert("EP_NODE_POLL_SECONDS".into(), "30".into());
    env.insert("EP_NODE_HEARTBEAT_SECONDS".into(), "15".into());
    if !token_id.is_empty() {
        env.insert("EP_NODE_TOKEN_ID".into(), token_id.clone());
    }

    let env_lines: String = env
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");

    let install_script = build_install_script(&base_url, &env_lines);

    let config = row_to_spec(&spec_row)["config"].clone();
    let env_json: serde_json::Value = env
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect::<serde_json::Map<_, _>>()
        .into();

    Json(serde_json::json!({
        "node_id": node_id,
        "node_name": node_name,
        "install_script": install_script,
        "config": config,
        "env": env_json,
        "service": {
            "name": "edgeplane-node",
            "env_file": "/etc/edgeplane/edgeplane-node.service.env",
        },
        "join_token": token_id,
    }))
    .into_response()
}

async fn get_node_install_script(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    let base_url = std::env::var("EP_RUNTIME_RELEASE_BASE_URL").unwrap_or_else(|_| {
        "https://github.com/RyanMerlin/edgeplane/releases/latest/download".to_string()
    });

    let node_row = match sqlx::query(
        "SELECT * FROM runtimenode WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&node_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "text/plain")],
                "node not found",
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("get_node_install_script: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let node_name: String = node_row.get("node_name");
    let hostname: String = node_row.get("hostname");
    let trust_tier: String = node_row.get("trust_tier");
    let labels_json: String = node_row.get("labels_json");
    let runtime_version: String = node_row.get("runtime_version");

    let spec_row = match ensure_node_spec(
        &state.db,
        &principal.subject,
        &node_id,
        &node_name,
        &trust_tier,
        &labels_json,
        &runtime_version,
        "",
        "stable",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("get_node_install_script spec: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let upgrade_channel: String = spec_row.get("upgrade_channel");
    let desired_version: String = spec_row.get("desired_version");

    let token_row = sqlx::query(
        "SELECT id FROM runtimejointoken WHERE node_id=$1 AND owner_subject=$2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&node_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let token_id = token_row
        .as_ref()
        .map(|r| r.get::<String, _>("id"))
        .unwrap_or_default();

    let mut env_pairs: Vec<String> = vec![
        format!("EP_BASE_URL={base_url}"),
        format!("EP_NODE_NAME={node_name}"),
        format!("EP_NODE_HOSTNAME={hostname}"),
        format!("EP_NODE_TRUST_TIER={trust_tier}"),
        format!("EP_NODE_UPGRADE_CHANNEL={upgrade_channel}"),
        format!("EP_NODE_DESIRED_VERSION={desired_version}"),
        "EP_NODE_POLL_SECONDS=30".into(),
        "EP_NODE_HEARTBEAT_SECONDS=15".into(),
    ];
    if !token_id.is_empty() {
        env_pairs.push(format!("EP_NODE_TOKEN_ID={token_id}"));
    }
    let env_lines = env_pairs.join("\n");
    let script = build_install_script(&base_url, &env_lines);

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        script,
    )
        .into_response()
}

// ── Node spec mutations ───────────────────────────────────────────────────────

async fn reconcile_node(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
    Json(body): Json<NodeReconcile>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    let node_row = match sqlx::query(
        "SELECT * FROM runtimenode WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&node_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("node not found"),
        Err(e) => {
            tracing::error!("reconcile_node fetch node: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let node_name: String = node_row.get("node_name");
    let trust_tier: String = node_row.get("trust_tier");
    let labels_json: String = node_row.get("labels_json");
    let runtime_version: String = node_row.get("runtime_version");

    let spec_row = match ensure_node_spec(
        &state.db,
        &principal.subject,
        &node_id,
        &node_name,
        &trust_tier,
        &labels_json,
        &runtime_version,
        "",
        "stable",
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("reconcile_node ensure_spec: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let spec_id: i32 = spec_row.get("id");

    // Build update clauses dynamically
    let mut set_clauses: Vec<String> = vec!["last_reconcile_at=$1".into(), "updated_at=$1".into()];
    let mut idx = 2i32;
    let mut param_drain: Option<String> = None;
    let mut param_version: Option<String> = None;
    let mut param_health: Option<String> = None;

    if body.drain_state.is_some() {
        set_clauses.push(format!("drain_state=${idx}"));
        param_drain = body.drain_state.clone();
        idx += 1;
    }
    if body.desired_version.is_some() {
        set_clauses.push(format!("desired_version=${idx}"));
        param_version = body.desired_version.clone();
        idx += 1;
    }
    if body.health_summary.is_some() {
        set_clauses.push(format!("health_summary=${idx}"));
        param_health = body.health_summary.clone();
        idx += 1;
    }

    let id_placeholder = idx;
    let sql = format!(
        "UPDATE runtimenodespec SET {} WHERE id=${} RETURNING *",
        set_clauses.join(", "),
        id_placeholder
    );

    let mut q = sqlx::query(&sql).bind(now);
    if let Some(v) = param_drain {
        q = q.bind(v);
    }
    if let Some(v) = param_version {
        q = q.bind(v);
    }
    if let Some(v) = param_health {
        q = q.bind(v);
    }
    q = q.bind(spec_id);

    let updated_spec = match q.fetch_one(&state.db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("reconcile_node update: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let payload = json_dump(&serde_json::json!({"node_id": node_id}));
    let _ = sqlx::query(
        "INSERT INTO nodeevent (node_id, lease_id, event_type, payload_json, created_at) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Some(&node_id))
    .bind(None::<String>)
    .bind("node.spec.reconcile")
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await;

    Json(serde_json::json!({"spec": row_to_spec(&updated_spec)})).into_response()
}

async fn cordon_node(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    mutate_node_spec_state(state, node_id, principal.subject, "cordoned").await
}

async fn drain_node(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    mutate_node_spec_state(state, node_id, principal.subject, "draining").await
}

async fn upgrade_node(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    mutate_node_spec_state(state, node_id, principal.subject, "upgrading").await
}

// ── Node-scoped agents (Phase 6 sync-loop substrate) ──────────────────────────

#[derive(serde::Deserialize)]
struct NodeAgentAssign {
    domain_id: String,
    runtime_kind: String,
    #[serde(default)]
    runtime_version: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    labels: serde_json::Value,
    /// `"task"` | `"persistent"`. Defaults to `"task"` when omitted.
    #[serde(default)]
    supervision_mode: Option<String>,
    #[serde(default)]
    profile: Option<serde_json::Value>,
    #[serde(default)]
    machine: Option<serde_json::Value>,
    #[serde(default)]
    runtime: Option<serde_json::Value>,
    /// Optional canonical name for the persistent agent identity this
    /// assignment represents. See `AgentEnroll.agent_name` in
    /// `routes/work.rs` for the rationale.
    #[serde(default)]
    agent_name: Option<String>,
}

/// Verify the node exists and the caller owns it (or is admin). Returns the
/// owner subject on success, or an error response.
async fn require_node_owner(
    state: &Arc<AppState>,
    principal: &Principal,
    node_id: &str,
) -> Result<String, axum::response::Response> {
    let row = sqlx::query("SELECT owner_subject FROM runtimenode WHERE id=$1")
        .bind(node_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("require_node_owner: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;

    let owner: String = match row {
        Some(r) => r.get("owner_subject"),
        None => return Err(not_found("Node not found")),
    };
    if owner != principal.subject && !principal.is_admin {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    Ok(owner)
}

/// Assign a new agent to this node in the requested domain. Mirrors
/// `enroll_agent` (which is domain-scoped); this variant is convenient for
/// the controlplane / orchestrator to push assignments to a specific node.
async fn assign_node_agent(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
    Json(body): Json<NodeAgentAssign>,
) -> impl IntoResponse {
    if let Err(resp) = require_node_owner(&state, &principal, &node_id).await {
        return resp;
    }

    // Verify the target domain exists.
    let domain_ok: Option<i32> = sqlx::query_scalar("SELECT 1 FROM domain WHERE id=$1")
        .bind(&body.domain_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if domain_ok.is_none() {
        return not_found("Domain not found");
    }

    let agent_id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();
    let caps_json = serde_json::to_string(&body.capabilities).unwrap_or_else(|_| "[]".to_string());
    let labels_json = serde_json::to_string(&body.labels).unwrap_or_else(|_| "{}".to_string());
    let supervision = body.supervision_mode.as_deref().unwrap_or("task");
    let profile_json = body
        .profile
        .as_ref()
        .and_then(|v| serde_json::to_string(v).ok());
    let machine_json = body
        .machine
        .as_ref()
        .and_then(|v| serde_json::to_string(v).ok());
    let runtime_json = body
        .runtime
        .as_ref()
        .and_then(|v| serde_json::to_string(v).ok());

    // Derive the node's short name for the canonical agent name prefix.
    // Format: `<node_short>-<agent_name>` → public_id `<node_short>-<agent_name>-<hex>`.
    // `node_short` is the first DNS label of the Tailscale FQDN, or the
    // raw node_name if it contains no dots (e.g. "excalibur").
    let node_short: String = sqlx::query_scalar(
        "SELECT SPLIT_PART(COALESCE(tailscale_fqdn, node_name), '.', 1) FROM runtimenode WHERE id=$1",
    )
    .bind(&node_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None)
    .unwrap_or_else(|| "node".to_string());

    let agent_public_id = match &body.agent_name {
        Some(n) if !n.trim().is_empty() => {
            // Canonical name: <node_short>-<agent_name>
            let canonical = format!("{node_short}-{}", n.trim());
            match crate::routes::agents::upsert_agent_by_name(&state.db, &canonical, &caps_json)
                .await
            {
                Ok(pid) => Some(pid),
                Err(e) => {
                    tracing::error!("assign_node_agent agent link: {e}");
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"detail": e.to_string()})),
                    )
                        .into_response();
                }
            }
        }
        _ => None,
    };

    let row = sqlx::query(
        "INSERT INTO meshagent \
         (id, domain_id, node_id, runtime_kind, runtime_version, capabilities, labels, \
          status, current_task_id, enrolled_by_subject, enrolled_at, last_heartbeat_at, \
          runtime_node_id, profile_json, machine_json, runtime_json, supervision_mode, \
          agent_public_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,'online',NULL,$8,$9,NULL,$3,$10,$11,$12,$13,$14) \
         RETURNING *",
    )
    .bind(&agent_id)
    .bind(&body.domain_id)
    .bind(&node_id)
    .bind(&body.runtime_kind)
    .bind(&body.runtime_version)
    .bind(&caps_json)
    .bind(&labels_json)
    .bind(&principal.subject)
    .bind(now)
    .bind(&profile_json)
    .bind(&machine_json)
    .bind(&runtime_json)
    .bind(supervision)
    .bind(&agent_public_id)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(r) => {
            let agent_json = crate::routes::work::row_to_agent(&r);
            crate::routes::work::broadcast_assignment_changed(
                &node_id,
                serde_json::json!({
                    "type": "agent.assigned",
                    "agent_id": agent_json["id"],
                    "agent": agent_json,
                }),
            )
            .await;
            (StatusCode::CREATED, Json(agent_json)).into_response()
        }
        Err(e) => {
            tracing::error!("assign_node_agent: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Revoke an agent assignment from this node. The agent row is deleted so the
/// daemon's sync loop will tear down the corresponding task_loop on next poll.
async fn revoke_node_agent(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((node_id, agent_id)): Path<(String, String)>,
) -> impl IntoResponse {
    if let Err(resp) = require_node_owner(&state, &principal, &node_id).await {
        return resp;
    }

    // Ensure the agent belongs to this node before deleting.
    let owner_row = sqlx::query(
        "SELECT domain_id FROM meshagent \
         WHERE id=$1 AND (runtime_node_id=$2 OR node_id=$2)",
    )
    .bind(&agent_id)
    .bind(&node_id)
    .fetch_optional(&state.db)
    .await;
    let domain_id: String = match owner_row {
        Ok(Some(r)) => r.get("domain_id"),
        Ok(None) => return not_found("Agent not found on this node"),
        Err(e) => {
            tracing::error!("revoke_node_agent lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Err(e) = sqlx::query("DELETE FROM meshagent WHERE id=$1")
        .bind(&agent_id)
        .execute(&state.db)
        .await
    {
        tracing::error!("revoke_node_agent delete: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    crate::routes::work::broadcast_assignment_changed(
        &node_id,
        serde_json::json!({
            "type": "agent.revoked",
            "agent_id": agent_id,
            "domain_id": domain_id,
        }),
    )
    .await;

    StatusCode::NO_CONTENT.into_response()
}

// ── Jobs ──────────────────────────────────────────────────────────────────────

async fn create_job(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<JobCreate>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();
    let job_id = Uuid::new_v4().to_string();

    let args_json = serde_json::to_string(&body.args).unwrap_or_else(|_| "[]".to_string());
    let env_json = json_dump(&body.env);
    let mounts_json = serde_json::to_string(&body.mounts).unwrap_or_else(|_| "[]".to_string());
    let artifact_rules_json = json_dump(&body.artifact_rules);
    let required_capabilities_json =
        serde_json::to_string(&body.required_capabilities).unwrap_or_else(|_| "[]".to_string());
    let preferred_labels_json = json_dump(&body.preferred_labels);

    match sqlx::query(
        "INSERT INTO runtimejob \
         (id, owner_subject, domain_id, task_id, runtime_session_id, runtime_class, image, \
          command, args_json, env_json, cwd, mounts_json, artifact_rules_json, timeout_seconds, \
          restart_policy, required_capabilities_json, preferred_labels_json, status, \
          created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,'pending',$18,$18) \
         RETURNING *",
    )
    .bind(&job_id)
    .bind(&principal.subject)
    .bind(&body.domain_id)
    .bind(body.task_id)
    .bind(&body.runtime_session_id)
    .bind(&body.runtime_class)
    .bind(&body.image)
    .bind(&body.command)
    .bind(&args_json)
    .bind(&env_json)
    .bind(&body.cwd)
    .bind(&mounts_json)
    .bind(&artifact_rules_json)
    .bind(body.timeout_seconds)
    .bind(&body.restart_policy)
    .bind(&required_capabilities_json)
    .bind(&preferred_labels_json)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::CREATED, Json(row_to_job(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_job: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_jobs(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    let rows = if let Some(ref s) = q.status {
        sqlx::query(
            "SELECT * FROM runtimejob WHERE owner_subject=$1 AND status=$2 \
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(&principal.subject)
        .bind(s)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query(
            "SELECT * FROM runtimejob WHERE owner_subject=$1 \
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(&principal.subject)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    };

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_job).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_jobs: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Leases ────────────────────────────────────────────────────────────────────

async fn create_lease(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(job_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // Verify job ownership
    match sqlx::query(
        "SELECT id FROM runtimejob WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&job_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(None) => return not_found("job not found"),
        Err(e) => {
            tracing::error!("create_lease verify job: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    let node_id = body
        .get("node_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let lease_id = Uuid::new_v4().to_string();

    match sqlx::query(
        "INSERT INTO joblease \
         (id, job_id, node_id, status, claimed_at, heartbeat_at, started_at, finished_at, \
          exit_code, error_message, cleanup_status, created_at, updated_at) \
         VALUES ($1,$2,$3,'pending',$4,NULL,NULL,NULL,NULL,'',' ',$4,$4) RETURNING *",
    )
    .bind(&lease_id)
    .bind(&job_id)
    .bind(&node_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => {
            let payload = json_dump(&serde_json::json!({"job_id": job_id, "node_id": node_id}));
            let _ = sqlx::query(
                "INSERT INTO nodeevent (node_id, lease_id, event_type, payload_json, created_at) VALUES ($1,$2,$3,$4,$5)",
            )
            .bind(Some(&node_id))
            .bind(Some(&lease_id))
            .bind("lease.created")
            .bind(&payload)
            .bind(now)
            .execute(&state.db)
            .await;
            (StatusCode::CREATED, Json(row_to_lease(&row))).into_response()
        }
        Err(e) => {
            tracing::error!("create_lease insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn claim_lease(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // Verify node ownership
    match sqlx::query(
        "SELECT id FROM runtimenode WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&node_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(None) => return not_found("node not found"),
        Err(e) => {
            tracing::error!("claim_lease verify node: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    // Find a pending lease for this node
    let lease_row = match sqlx::query(
        "UPDATE joblease SET status='claimed', claimed_at=$1, updated_at=$1 \
         WHERE id = ( \
           SELECT id FROM joblease \
           WHERE node_id=$2 AND status='pending' \
           ORDER BY created_at ASC LIMIT 1 \
         ) RETURNING *",
    )
    .bind(now)
    .bind(&node_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("claim_lease update: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    match lease_row {
        None => (
            StatusCode::NO_CONTENT,
            Json(serde_json::json!({"detail": "no pending leases"})),
        )
            .into_response(),
        Some(row) => {
            let lease_id: String = row.get("id");
            let payload = json_dump(&serde_json::json!({"node_id": node_id}));
            let _ = sqlx::query(
                "INSERT INTO nodeevent (node_id, lease_id, event_type, payload_json, created_at) VALUES ($1,$2,$3,$4,$5)",
            )
            .bind(Some(&node_id))
            .bind(Some(&lease_id))
            .bind("lease.claimed")
            .bind(&payload)
            .bind(now)
            .execute(&state.db)
            .await;
            Json(row_to_lease(&row)).into_response()
        }
    }
}

async fn get_lease(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(lease_id): Path<String>,
) -> impl IntoResponse {
    // Verify through job ownership
    match sqlx::query(
        "SELECT l.* FROM joblease l \
         JOIN runtimejob j ON j.id = l.job_id \
         WHERE l.id=$1 AND j.owner_subject=$2",
    )
    .bind(&lease_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => Json(row_to_lease(&row)).into_response(),
        Ok(None) => not_found("lease not found"),
        Err(e) => {
            tracing::error!("get_lease: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_lease_status(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(lease_id): Path<String>,
    Json(body): Json<LeaseStatus>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();
    let heartbeat_at = body
        .heartbeat_at
        .map(|t| t.naive_utc())
        .unwrap_or(now);
    let started_at = body.started_at.map(|t| t.naive_utc());

    // Verify ownership via job
    match sqlx::query(
        "SELECT l.id FROM joblease l \
         JOIN runtimejob j ON j.id = l.job_id \
         WHERE l.id=$1 AND j.owner_subject=$2",
    )
    .bind(&lease_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(None) => return not_found("lease not found"),
        Err(e) => {
            tracing::error!("update_lease_status verify: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    let row = match sqlx::query(
        "UPDATE joblease \
         SET status=$1, heartbeat_at=$2, started_at=COALESCE($3, started_at), updated_at=$4 \
         WHERE id=$5 RETURNING *",
    )
    .bind(&body.status)
    .bind(heartbeat_at)
    .bind(started_at)
    .bind(now)
    .bind(&lease_id)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("update_lease_status update: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let node_id: String = row.get("node_id");
    let payload = json_dump(&serde_json::json!({"status": body.status}));
    let _ = sqlx::query(
        "INSERT INTO nodeevent (node_id, lease_id, event_type, payload_json, created_at) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Some(&node_id))
    .bind(Some(&lease_id))
    .bind("lease.status")
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await;

    Json(row_to_lease(&row)).into_response()
}

async fn complete_lease(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(lease_id): Path<String>,
    Json(body): Json<LeaseComplete>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // Verify ownership via job
    match sqlx::query(
        "SELECT l.id FROM joblease l \
         JOIN runtimejob j ON j.id = l.job_id \
         WHERE l.id=$1 AND j.owner_subject=$2",
    )
    .bind(&lease_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(None) => return not_found("lease not found"),
        Err(e) => {
            tracing::error!("complete_lease verify: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    let status = if body.exit_code == 0 {
        "completed"
    } else {
        "failed"
    };

    let row = match sqlx::query(
        "UPDATE joblease \
         SET status=$1, exit_code=$2, error_message=$3, finished_at=$4, updated_at=$4 \
         WHERE id=$5 RETURNING *",
    )
    .bind(status)
    .bind(body.exit_code)
    .bind(&body.error_message)
    .bind(now)
    .bind(&lease_id)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("complete_lease update: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let node_id: String = row.get("node_id");
    let job_id: String = row.get("job_id");

    // Also update the job status
    let _ = sqlx::query(
        "UPDATE runtimejob SET status=$1, updated_at=$2 WHERE id=$3",
    )
    .bind(status)
    .bind(now)
    .bind(&job_id)
    .execute(&state.db)
    .await;

    let payload = json_dump(
        &serde_json::json!({"exit_code": body.exit_code, "error_message": body.error_message}),
    );
    let _ = sqlx::query(
        "INSERT INTO nodeevent (node_id, lease_id, event_type, payload_json, created_at) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Some(&node_id))
    .bind(Some(&lease_id))
    .bind("lease.complete")
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await;

    Json(row_to_lease(&row)).into_response()
}

async fn append_lease_logs(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(lease_id): Path<String>,
    Json(body): Json<AppendLogsBody>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // Verify ownership via job
    match sqlx::query(
        "SELECT l.id FROM joblease l \
         JOIN runtimejob j ON j.id = l.job_id \
         WHERE l.id=$1 AND j.owner_subject=$2",
    )
    .bind(&lease_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(None) => return not_found("lease not found"),
        Err(e) => {
            tracing::error!("append_lease_logs verify: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    // Fetch node_id for event
    let node_id = sqlx::query_scalar::<_, String>(
        "SELECT node_id FROM joblease WHERE id=$1",
    )
    .bind(&lease_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .unwrap_or_default();

    let payload = json_dump(&serde_json::json!({"logs": body.logs}));
    let _ = sqlx::query(
        "INSERT INTO nodeevent (node_id, lease_id, event_type, payload_json, created_at) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(Some(&node_id))
    .bind(Some(&lease_id))
    .bind("lease.logs")
    .bind(&payload)
    .bind(now)
    .execute(&state.db)
    .await;

    StatusCode::NO_CONTENT.into_response()
}

// ── Execution sessions ────────────────────────────────────────────────────────

async fn create_execution_session(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<ExecutionSessionCreate>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // Verify lease ownership via job
    match sqlx::query(
        "SELECT l.id FROM joblease l \
         JOIN runtimejob j ON j.id = l.job_id \
         WHERE l.id=$1 AND j.owner_subject=$2",
    )
    .bind(&body.lease_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(None) => return not_found("lease not found"),
        Err(e) => {
            tracing::error!("create_execution_session verify: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(Some(_)) => {}
    }

    let session_id = Uuid::new_v4().to_string();
    let token_prefix = attach_token_prefix();

    match sqlx::query(
        "INSERT INTO executionsession \
         (id, lease_id, runtime_class, pty_requested, attach_token_prefix, status, \
          created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,'pending',$6,$6) RETURNING *",
    )
    .bind(&session_id)
    .bind(&body.lease_id)
    .bind(&body.runtime_class)
    .bind(body.pty_requested)
    .bind(&token_prefix)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::CREATED, Json(row_to_execution_session(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_execution_session insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_attach_token(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    // Verify ownership: execution session → lease → job → owner_subject
    let row = match sqlx::query(
        "SELECT es.*, j.owner_subject FROM executionsession es \
         JOIN joblease l ON l.id = es.lease_id \
         JOIN runtimejob j ON j.id = l.job_id \
         WHERE es.id=$1 AND j.owner_subject=$2",
    )
    .bind(&session_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("execution session not found"),
        Err(e) => {
            tracing::error!("get_attach_token: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let token_prefix: String = row.get("attach_token_prefix");
    let token = make_token_local();

    Json(serde_json::json!({
        "session_id": session_id,
        "attach_token": token,
        "attach_token_prefix": token_prefix,
    }))
    .into_response()
}

async fn attach_execution_session(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(session_id): Path<String>,
    Json(_body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // Verify ownership
    let row = match sqlx::query(
        "SELECT es.* FROM executionsession es \
         JOIN joblease l ON l.id = es.lease_id \
         JOIN runtimejob j ON j.id = l.job_id \
         WHERE es.id=$1 AND j.owner_subject=$2",
    )
    .bind(&session_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("execution session not found"),
        Err(e) => {
            tracing::error!("attach_execution_session verify: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Transition to "attached"
    let updated = match sqlx::query(
        "UPDATE executionsession SET status='attached', updated_at=$1 WHERE id=$2 RETURNING *",
    )
    .bind(now)
    .bind(&session_id)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("attach_execution_session update: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let _ = row; // consumed above to verify ownership
    Json(row_to_execution_session(&updated)).into_response()
}

// ── PTY WebSocket relay ───────────────────────────────────────────────────────
// Per-session broadcast channel registry: session_id → (sender, conn_id_of_sender)

type PtyRegistry = TokioMutex<HashMap<String, broadcast::Sender<(String, String)>>>;

static PTY_REGISTRY: OnceLock<PtyRegistry> = OnceLock::new();

fn pty_registry() -> &'static PtyRegistry {
    PTY_REGISTRY.get_or_init(|| TokioMutex::new(HashMap::new()))
}

async fn execution_session_pty(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    // Token from query param or skip (auth is best-effort for WS upgrades)
    let token = params.get("token").cloned().unwrap_or_default();

    // Verify token and session ownership before upgrading
    let allowed = async {
        if token.is_empty() { return false; }
        let hash = hash_token_local(&token);
        let row = sqlx::query(
            "SELECT es.id FROM executionsession es \
             WHERE es.id=$1 AND es.attach_token_hash=$2 LIMIT 1",
        )
        .bind(&session_id)
        .bind(&hash)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        row.is_some()
    }
    .await;

    if !allowed {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    let conn_id = uuid::Uuid::new_v4().to_string();
    ws.on_upgrade(move |socket| handle_pty_ws(socket, session_id, conn_id))
}

async fn handle_pty_ws(socket: WebSocket, session_id: String, conn_id: String) {
    use futures_util::{SinkExt, StreamExt};

    let (mut sink, mut stream) = socket.split();

    // Get or create broadcast channel for this session (capacity 256 messages)
    let sender = {
        let mut reg = pty_registry().lock().await;
        let tx = reg.entry(session_id.clone()).or_insert_with(|| {
            let (tx, _) = broadcast::channel::<(String, String)>(256);
            tx
        });
        tx.clone()
    };
    let mut receiver = sender.subscribe();

    let conn_id_read = conn_id.clone();
    let sender_clone = sender.clone();
    let session_id_clean = session_id.clone();

    // Read task: forward incoming WebSocket messages to broadcast channel
    let read_task = tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(txt)) => {
                    let _ = sender_clone.send((conn_id_read.clone(), txt.to_string()));
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
        // Cleanup empty sessions
        let mut reg = pty_registry().lock().await;
        if let Some(tx) = reg.get(&session_id_clean) {
            if tx.receiver_count() == 0 {
                reg.remove(&session_id_clean);
            }
        }
    });

    // Write task: forward broadcast messages (from other clients) to this WebSocket
    let write_task = tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok((src_conn_id, msg)) => {
                    if src_conn_id == conn_id { continue; } // skip own messages
                    if sink.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    tokio::select! {
        _ = read_task => {}
        _ = write_task => {}
    }
}

// ── Mesh agent attach proxy ───────────────────────────────────────────────────

/// `GET /runtime/nodes/{node_id}/agents/{agent_id}/attach`
///
/// Auth: bearer token in `Authorization` header **or** `?ep_token=` query
/// param (browsers can't set headers on a WS upgrade). The token is checked
/// the same way `Principal` extraction does — admin token or a valid user
/// session.
///
/// Behavior: resolve `node_id` → tailscale_fqdn/ip + per-node attach_secret
/// from the runtimenode row, sign a short-lived HMAC token, dial the mesh
/// node's `/attach/{agent_id}` WS endpoint over Tailscale, then proxy frames
/// bidirectionally between the browser socket and the mesh socket.
async fn agent_attach_proxy(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path((node_id, agent_id)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // 1) Auth — accept either Authorization header or ep_token query.
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| params.get("ep_token").cloned())
        .unwrap_or_default();

    if !verify_attach_caller_token(&state, &token).await {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    // 2) Resolve node → reachable address + per-node attach secret.
    let row = sqlx::query(
        "SELECT tailscale_fqdn, tailscale_ip, attach_secret FROM runtimenode WHERE id = $1",
    )
    .bind(&node_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let Some(row) = row else {
        return (StatusCode::NOT_FOUND, "node not found").into_response();
    };
    let fqdn: Option<String> = row.try_get("tailscale_fqdn").ok();
    let ip: Option<String> = row.try_get("tailscale_ip").ok();
    let host = match fqdn.filter(|s| !s.is_empty()).or(ip.filter(|s| !s.is_empty())) {
        Some(h) => h,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "node has no tailscale address registered",
            )
                .into_response();
        }
    };

    // 3) Sign HMAC attach token with the per-node secret stored at registration.
    let secret: Option<String> = row.try_get("attach_secret").ok().flatten();
    let secret = match secret.filter(|s| !s.is_empty()) {
        Some(s) => s,
        None => {
            tracing::warn!(node_id = %node_id, "agent_attach_proxy: node has no attach_secret — refusing to dial mesh");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "node has no attach_secret configured",
            )
                .into_response();
        }
    };
    // 60-second window is plenty — only used to gate the upgrade itself.
    let exp = Utc::now().timestamp() + 60;
    let mesh_token = sign_attach_token(&secret, &agent_id, exp);
    let mesh_url = format!("ws://{host}:8009/attach/{agent_id}?token={mesh_token}&exp={exp}");

    // 4) Upgrade caller, then dial mesh.
    ws.on_upgrade(move |browser_socket| async move {
        if let Err(e) = run_attach_proxy(browser_socket, mesh_url).await {
            tracing::debug!("attach proxy session ended: {e:#}");
        }
    })
}

async fn verify_attach_caller_token(state: &AppState, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    // Node JWT fast path — verify in-process, then confirm JTI not revoked.
    if token.matches('.').count() == 2 {
        if let Ok(claims) = crate::jwt::verify_node_jwt(token, &state.jwt_decoding_key) {
            let now = chrono::Utc::now().naive_utc();
            return matches!(
                sqlx::query("SELECT revoked FROM nodetoken WHERE jti=$1 AND expires_at > $2 AND revoked=false")
                    .bind(&claims.jti)
                    .bind(now)
                    .fetch_optional(&state.db)
                    .await,
                Ok(Some(_))
            );
        }
        return false;
    }
    // Look up either a user session or service-account token by hash.
    let hash = hash_token_local(token);
    let now = Utc::now().naive_utc();
    let user_ok = sqlx::query("SELECT 1 FROM usersession WHERE token_hash = $1 AND (expires_at IS NULL OR expires_at > $2) LIMIT 1")
        .bind(&hash)
        .bind(now)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .is_some();
    if user_ok {
        return true;
    }
    sqlx::query("SELECT 1 FROM serviceaccounttoken WHERE token_hash = $1 AND (expires_at IS NULL OR expires_at > $2) LIMIT 1")
        .bind(&hash)
        .bind(now)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .is_some()
}

fn sign_attach_token(secret: &str, agent_id: &str, exp: i64) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(format!("attach:{agent_id}:{exp}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod attach_proxy_tests {
    use super::sign_attach_token;

    /// Locks the wire format. edgeplaned's `attach_ws::sign_attach` MUST produce
    /// the same output for the same inputs — see the matching test there.
    #[test]
    fn sign_attach_token_is_stable() {
        let sig = sign_attach_token("s3cret", "agent-1", 1_700_000_000);
        assert_eq!(sig.len(), 64);
        // Re-sign produces the same value.
        assert_eq!(sig, sign_attach_token("s3cret", "agent-1", 1_700_000_000));
        // Different inputs produce different signatures.
        assert_ne!(sig, sign_attach_token("s3cret", "agent-2", 1_700_000_000));
        assert_ne!(sig, sign_attach_token("other", "agent-1", 1_700_000_000));
    }
}

async fn run_attach_proxy(
    browser_socket: WebSocket,
    mesh_url: String,
) -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as TgMessage;

    let (mesh_ws, _) = tokio_tungstenite::connect_async(&mesh_url)
        .await
        .map_err(|e| anyhow::anyhow!("dial mesh {mesh_url}: {e}"))?;

    let (mut browser_sink, mut browser_stream) = browser_socket.split();
    let (mut mesh_sink, mut mesh_stream) = mesh_ws.split();

    // Browser → mesh
    let b2m = tokio::spawn(async move {
        while let Some(msg) = browser_stream.next().await {
            let Ok(msg) = msg else { break };
            let out = match msg {
                Message::Binary(b) => TgMessage::Binary(b.to_vec().into()),
                Message::Text(t) => TgMessage::Text(t.to_string().into()),
                Message::Close(_) => break,
                _ => continue,
            };
            if mesh_sink.send(out).await.is_err() {
                break;
            }
        }
    });

    // Mesh → browser
    let m2b = tokio::spawn(async move {
        while let Some(msg) = mesh_stream.next().await {
            let Ok(msg) = msg else { break };
            let out = match msg {
                TgMessage::Binary(b) => Message::Binary(b.to_vec().into()),
                TgMessage::Text(t) => Message::Text(t.to_string().into()),
                TgMessage::Close(_) => break,
                _ => continue,
            };
            if browser_sink.send(out).await.is_err() {
                break;
            }
        }
    });

    // Either direction closing tears down the session.
    tokio::select! {
        _ = b2m => {}
        _ = m2b => {}
    }
    Ok(())
}

// ── Phase 4a: controlplane-driven enrollment ─────────────────────────────────
//
// `list_node_agents` and `node_notify_ws` are the two surfaces edgeplaned
// daemons consume to drive the new "no domains in yaml" flow:
//
//   1. On daemon start, GET this list to discover what to spawn locally.
//   2. Subscribe to `node_notify_ws` so add/remove/reassign mutations
//      land as live rebalances instead of yaml edits + restarts.
//
// The plan: docs/plans/2026-05-10-edgeplaned-controlplane-driven-enrollment.md.

/// `GET /runtime/nodes/{node_id}/agents`
///
/// List the meshagent rows assigned to this runtime node — i.e. rows where
/// `runtime_node_id = $1`. Owner-scoped: the principal must own the node.
///
/// The daemon calls this once at startup and again after any
/// `agent.assignment_changed` event from `node_notify_ws` to reconcile its
/// local supervisor set against the controlplane.
async fn list_node_agents(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    // Verify the node exists and the caller owns it (or is admin).
    let owner_check = sqlx::query_scalar::<_, String>(
        "SELECT owner_subject FROM runtimenode WHERE id = $1",
    )
    .bind(&node_id)
    .fetch_optional(&state.db)
    .await;

    let owner = match owner_check {
        Ok(Some(o)) => o,
        Ok(None) => return (StatusCode::NOT_FOUND, "node not found").into_response(),
        Err(e) => {
            tracing::error!("list_node_agents owner lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if owner != principal.subject && !principal.is_admin {
        return (StatusCode::FORBIDDEN, "node does not belong to you").into_response();
    }

    // Pull meshagent rows assigned to this node (Phase 6 also matches the
    // legacy `node_id` column so older enrollments aren't missed) and join
    // domain for name/kind so the daemon's sync loop can identify home-domain
    // assignments without a second round-trip.
    let rows = sqlx::query(
        "SELECT a.*, m.name AS domain_name, m.kind AS domain_kind \
         FROM meshagent a JOIN domain m ON a.domain_id = m.id \
         WHERE a.runtime_node_id = $1 OR a.node_id = $1 \
         ORDER BY a.enrolled_at ASC",
    )
    .bind(&node_id)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rs) => {
            let payload: Vec<serde_json::Value> = rs
                .iter()
                .map(|r| {
                    let mut v = crate::routes::work::row_to_agent(r);
                    v["domain_name"] =
                        serde_json::Value::String(r.get::<String, _>("domain_name"));
                    v["domain_kind"] =
                        serde_json::Value::String(r.get::<String, _>("domain_kind"));
                    v
                })
                .collect();
            Json(payload).into_response()
        }
        Err(e) => {
            tracing::error!("list_node_agents fetch: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /runtime/nodes/{node_id}/notify` (WebSocket)
///
/// Long-lived subscription for the daemon owning `node_id`. The controlplane
/// publishes JSON messages here whenever a meshagent assignment for this
/// node changes (assigned / unassigned / reassigned). 30s ping keeps the
/// connection alive through Tailscale + reverse proxies.
///
/// See `crate::routes::work::broadcast_assignment_changed` for the publisher
/// side.
async fn node_notify_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    // Owner check up-front; on failure, refuse the upgrade with a 403.
    let owner_check = sqlx::query_scalar::<_, String>(
        "SELECT owner_subject FROM runtimenode WHERE id = $1",
    )
    .bind(&node_id)
    .fetch_optional(&state.db)
    .await;
    match owner_check {
        Ok(Some(owner)) if owner == principal.subject || principal.is_admin => {}
        Ok(Some(_)) => {
            return (StatusCode::FORBIDDEN, "node does not belong to you").into_response();
        }
        Ok(None) => return (StatusCode::NOT_FOUND, "node not found").into_response(),
        Err(e) => {
            tracing::error!("node_notify_ws owner lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    ws.on_upgrade(move |socket| handle_node_notify(socket, node_id))
}

async fn handle_node_notify(mut socket: WebSocket, node_id: String) {
    use crate::routes::work::node_notify_registry;
    use std::time::Duration;
    use tokio::sync::broadcast::error::RecvError;

    // Subscribe to the per-node broadcast channel, creating it on demand.
    let mut rx = {
        let mut reg = node_notify_registry().lock().await;
        let tx = reg
            .entry(node_id.clone())
            .or_insert_with(|| broadcast::channel::<String>(64).0);
        tx.subscribe()
    };

    let ping_msg = r#"{"type":"ping"}"#;
    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(payload) => {
                        if socket.send(Message::Text(payload.into())).await.is_err() {
                            return;
                        }
                    }
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
                if socket.send(Message::Text(ping_msg.into())).await.is_err() {
                    return;
                }
            }
        }
    }
}
