use axum::{
    extract::{Path, Query, State},
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::StatusCode,
    response::{sse::Event, sse::Sse, IntoResponse},
    routing::{get, patch, post},
    Json, Router,
};
use tokio_stream::wrappers::ReceiverStream;
use chrono::Utc;
use sqlx::Row;
use std::{collections::HashMap, sync::{Arc, OnceLock}};
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{auth::Principal, state::AppState};

// ── Task-available notification registry ──────────────────────────────────────
// Keyed by mission_id. Agents subscribe per-mission; senders are created on
// demand and never removed (the channel stays alive for the process lifetime,
// which is fine since we never have more missions than fit in RAM).

type NotifyRegistry = tokio::sync::Mutex<HashMap<String, broadcast::Sender<String>>>;

static NOTIFY_REGISTRY: OnceLock<NotifyRegistry> = OnceLock::new();

pub fn notify_registry() -> &'static NotifyRegistry {
    NOTIFY_REGISTRY.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

pub async fn broadcast_task_available(mission_id: &str, kluster_id: &str, task_id: &str) {
    let msg = serde_json::json!({
        "type": "task_available",
        "kluster_id": kluster_id,
        "task_id": task_id,
    })
    .to_string();
    let reg = notify_registry().lock().await;
    if let Some(tx) = reg.get(mission_id) {
        let _ = tx.send(msg); // best-effort; no subscribers is fine
    }
}

// ── Node-keyed assignment-change notifications ────────────────────────────────
//
// Parallel to `notify_registry()` above, but keyed by `runtime_node_id`
// (the runtimenode UUID) instead of mission_id. mc-mesh daemons subscribe
// here at startup and react to add/remove/reassign by spawning, shutting
// down, or rebalancing supervisors live — no daemon restart, no yaml edit.
//
// Wire payload shapes:
//   {"type":"agent.assigned",   "agent_id":"…", "agent": {…}}
//   {"type":"agent.unassigned", "agent_id":"…"}
//   {"type":"agent.reassigned", "agent_id":"…", "agent": {…},
//                               "old_mission_id":"…", "new_mission_id":"…"}

static NODE_NOTIFY_REGISTRY: OnceLock<NotifyRegistry> = OnceLock::new();

pub fn node_notify_registry() -> &'static NotifyRegistry {
    NODE_NOTIFY_REGISTRY.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// Notify the daemon at `runtime_node_id` that one of its agents was newly
/// assigned, removed, or reassigned. Best-effort — silent when no daemon is
/// currently subscribed for that node.
pub async fn broadcast_assignment_changed(
    runtime_node_id: &str,
    payload: serde_json::Value,
) {
    let reg = node_notify_registry().lock().await;
    if let Some(tx) = reg.get(runtime_node_id) {
        let _ = tx.send(payload.to_string());
    }
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Tasks
        .route("/work/klusters/{kluster_id}/tasks", get(list_tasks).post(create_task))
        .route("/work/klusters/{kluster_id}/graph", get(task_graph))
        .route("/work/tasks/{task_id}", get(get_task))
        .route("/work/tasks/{task_id}/cancel", post(cancel_task))
        .route("/work/tasks/{task_id}/retry", post(retry_task))
        .route("/work/tasks/{task_id}/claim", post(claim_task))
        .route("/work/tasks/{task_id}/heartbeat", post(heartbeat_task))
        .route("/work/tasks/{task_id}/progress", get(get_task_progress).post(append_progress))
        .route("/work/tasks/{task_id}/complete", post(complete_task))
        .route("/work/tasks/{task_id}/fail", post(fail_task))
        .route("/work/tasks/{task_id}/block", post(block_task))
        .route("/work/tasks/{task_id}/unblock", post(unblock_task))
        .route("/work/tasks/{task_id}/gates", get(list_gates).post(create_gate))
        .route("/work/tasks/{task_id}/gates/{gate_id}/resolve", post(resolve_gate))
        // Missions
        .route("/work/missions/{mission_id}/agents/enroll", post(enroll_agent))
        .route("/work/missions/{mission_id}/agents", get(list_mission_agents))
        .route("/work/missions/{mission_id}/messages", get(list_mission_messages).post(send_mission_message))
        .route("/work/missions/{mission_id}/roster", get(mission_roster))
        .route("/work/missions/{mission_id}/stream", get(mission_stream))
        // Agents
        .route("/work/agents/{agent_id}/heartbeat", post(agent_heartbeat))
        .route("/work/agents/{agent_id}/status", post(set_agent_status))
        .route("/work/agents/{agent_id}/profile", patch(update_agent_profile))
        .route("/work/agents/{agent_id}", get(get_agent))
        .route("/work/agents/{agent_id}/messages", get(get_agent_messages))
        .route("/work/agents/{agent_id}/notify", get(agent_notify_ws))
        // Kluster messages + stream
        .route("/work/klusters/{kluster_id}/messages", get(list_kluster_messages).post(send_kluster_message))
        .route("/work/klusters/{kluster_id}/stream", get(kluster_stream))
        // Global SSE feed — TUI agent feed; polls meshprogressevent for all agents
        .route("/sse", get(global_sse))
}

// ── Error helpers ──────────────────────────────────────────────────────────────

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}
fn conflict(msg: &str) -> axum::response::Response {
    (StatusCode::CONFLICT, Json(serde_json::json!({"detail": msg}))).into_response()
}
fn bad_request(msg: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({"detail": msg}))).into_response()
}

// ── Row helpers ────────────────────────────────────────────────────────────────

fn row_to_task(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "kluster_id": row.get::<String, _>("kluster_id"),
        "mission_id": row.get::<String, _>("mission_id"),
        "parent_task_id": row.get::<Option<String>, _>("parent_task_id"),
        "title": row.get::<String, _>("title"),
        "description": row.get::<String, _>("description"),
        "claim_policy": row.get::<String, _>("claim_policy"),
        "depends_on": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("depends_on")).unwrap_or(serde_json::json!([])),
        "produces": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("produces")).unwrap_or(serde_json::json!({})),
        "consumes": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("consumes")).unwrap_or(serde_json::json!({})),
        "required_capabilities": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("required_capabilities")).unwrap_or(serde_json::json!([])),
        "status": row.get::<String, _>("status"),
        "claimed_by_agent_id": row.get::<Option<String>, _>("claimed_by_agent_id"),
        "result_artifact_id": row.get::<Option<String>, _>("result_artifact_id"),
        "priority": row.get::<i32, _>("priority"),
        "lease_expires_at": row.get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at").map(|dt| dt.and_utc()),
        "created_by_subject": row.get::<String, _>("created_by_subject"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at").and_utc(),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at").and_utc(),
    })
}

pub fn row_to_agent(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let profile: Option<serde_json::Value> = row.get::<Option<&str>, _>("profile_json")
        .and_then(|s| serde_json::from_str(s).ok());
    let machine: Option<serde_json::Value> = row.get::<Option<&str>, _>("machine_json")
        .and_then(|s| serde_json::from_str(s).ok());
    let runtime: Option<serde_json::Value> = row.get::<Option<&str>, _>("runtime_json")
        .and_then(|s| serde_json::from_str(s).ok());
    let discovered_capabilities: serde_json::Value = row
        .try_get::<Option<&str>, _>("discovered_capabilities")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::json!([]));
    let meshagent_id: String = row.get::<String, _>("id");
    // `public_id` is the wire identifier mc-mesh uses for the poll loop
    // (`/agents/{public_id}/messages`) and that the mc CLI passes via
    // `--to-agent-id`. Prefer the linked `agent.public_id` when this
    // meshagent points at a persistent agent identity (Step 5 of the
    // public_id plan); fall back to the meshagent's own UUID when no
    // linkage was supplied at enrollment so older rows keep working.
    let agent_public_id: Option<String> = row
        .try_get::<Option<String>, _>("agent_public_id")
        .ok()
        .flatten();
    let public_id = agent_public_id.clone().unwrap_or_else(|| meshagent_id.clone());
    serde_json::json!({
        "id": &meshagent_id,
        "public_id": public_id,
        "agent_public_id": agent_public_id,
        "mission_id": row.get::<String, _>("mission_id"),
        "node_id": row.get::<Option<String>, _>("node_id"),
        "runtime_kind": row.get::<String, _>("runtime_kind"),
        "runtime_version": row.get::<String, _>("runtime_version"),
        "capabilities": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("capabilities")).unwrap_or(serde_json::json!([])),
        "labels": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("labels")).unwrap_or(serde_json::json!({})),
        "status": row.get::<String, _>("status"),
        "current_task_id": row.get::<Option<String>, _>("current_task_id"),
        "enrolled_at": row.get::<chrono::NaiveDateTime, _>("enrolled_at"),
        "last_heartbeat_at": row.get::<Option<chrono::NaiveDateTime>, _>("last_heartbeat_at"),
        // Daemon-driving fields — required by mc-mesh's controlplane-driven
        // enrollment loop (Phase 4 plan 2026-05-10).
        "runtime_node_id": row.get::<Option<String>, _>("runtime_node_id"),
        "supervision_mode": row.get::<Option<String>, _>("supervision_mode"),
        "profile": profile,
        "machine": machine,
        "runtime": runtime,
        "discovered_capabilities": discovered_capabilities,
    })
}

fn row_to_gate(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "mesh_task_id": row.get::<String, _>("mesh_task_id"),
        "run_id": row.get::<Option<String>, _>("run_id"),
        "gate_type": row.get::<String, _>("gate_type"),
        "required_approvals": row.get::<String, _>("required_approvals"),
        "status": row.get::<String, _>("status"),
        "approval_request_id": row.get::<Option<String>, _>("approval_request_id"),
        "ai_pending_action_id": row.get::<Option<String>, _>("ai_pending_action_id"),
        "policy_rule_id": row.get::<Option<String>, _>("policy_rule_id"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "resolved_at": row.get::<Option<chrono::NaiveDateTime>, _>("resolved_at"),
    })
}

fn row_to_message(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let body_json: serde_json::Value = serde_json::from_str(row.get::<&str, _>("body_json"))
        .unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "id": row.get::<i64, _>("id"),
        "mission_id": row.get::<String, _>("mission_id"),
        "kluster_id": row.get::<Option<String>, _>("kluster_id"),
        "from_agent_id": row.get::<String, _>("from_agent_id"),
        "to_agent_id": row.get::<Option<String>, _>("to_agent_id"),
        "task_id": row.get::<Option<String>, _>("task_id"),
        "channel": row.get::<String, _>("channel"),
        "body_json": body_json,
        "in_reply_to": row.get::<Option<i64>, _>("in_reply_to"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "read_at": row.get::<Option<chrono::NaiveDateTime>, _>("read_at"),
    })
}

// ── Body / query structs ───────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct TaskCreate {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_input_json")]
    input_json: String,
    #[serde(default = "default_first_claim")]
    claim_policy: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    produces: serde_json::Value,
    #[serde(default)]
    consumes: serde_json::Value,
    #[serde(default)]
    required_capabilities: Vec<String>,
    #[serde(default)]
    priority: i32,
    parent_task_id: Option<String>,
}
fn default_input_json() -> String { "{}".to_string() }
fn default_first_claim() -> String { "first_claim".to_string() }

#[derive(serde::Deserialize, Default)]
struct HeartbeatBody {
    claim_lease_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct CompleteBody {
    result_artifact_id: Option<String>,
    claim_lease_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct FailBody {
    #[serde(default)]
    #[allow(dead_code)]
    error: String,
    claim_lease_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct ProgressCreate {
    event_type: String,
    phase: Option<String>,
    step: Option<String>,
    #[serde(default)]
    summary: String,
    #[serde(default = "default_input_json")]
    payload_json: String,
    agent_run_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct MessageCreate {
    to_agent_id: Option<String>,
    task_id: Option<String>,
    #[serde(default = "default_coordination")]
    channel: String,
    body: Option<serde_json::Value>,
    #[serde(default = "default_empty_obj")]
    body_json: String,
    in_reply_to: Option<i64>,
}
fn default_coordination() -> String { "coordination".to_string() }
fn default_empty_obj() -> String { "{}".to_string() }

#[derive(serde::Deserialize)]
struct AgentEnroll {
    runtime_kind: String,
    #[serde(default)]
    runtime_version: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    labels: serde_json::Value,
    node_id: Option<String>,
    runtime_node_id: Option<String>,
    profile: Option<serde_json::Value>,
    machine: Option<serde_json::Value>,
    runtime: Option<serde_json::Value>,
    /// Optional canonical name for the persistent agent identity this
    /// enrollment represents (e.g. `aria-work`). When set, the controlplane
    /// upserts the matching `agent` row and stores its `public_id` on this
    /// meshagent. mc-mesh then receives the public_id as the wire identifier
    /// and uses it to poll `/agents/{public_id}/messages`. See
    /// `docs/plans/2026-05-11-agent-public-id-mc-mesh-fix.md`.
    #[serde(default)]
    agent_name: Option<String>,
}

#[derive(serde::Deserialize)]
struct GateCreate {
    gate_type: String,
    #[serde(default = "default_human")]
    required_approvals: String,
    run_id: Option<String>,
    approval_request_id: Option<String>,
}
fn default_human() -> String { "human".to_string() }

#[derive(serde::Deserialize)]
struct GateResolve {
    decision: String,
    #[serde(default)]
    #[allow(dead_code)]
    notes: String,
}

#[derive(serde::Deserialize)]
struct AgentProfileUpdate {
    profile: Option<serde_json::Value>,
    machine: Option<serde_json::Value>,
    runtime: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct TaskListQuery {
    status: Option<String>,
}

#[derive(serde::Deserialize)]
struct AgentStatusQuery {
    status: String,
}

#[derive(serde::Deserialize)]
struct MessageListQuery {
    channel: Option<String>,
    since_id: Option<i64>,
}

#[derive(serde::Deserialize)]
struct ProgressQuery {
    #[serde(default = "default_neg_one")]
    since_seq: i32,
}
fn default_neg_one() -> i32 { -1 }

#[derive(serde::Deserialize)]
struct AgentMessagesQuery {
    #[serde(default)]
    since_id: i64,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

const LEASE_TTL_SECS: i64 = 120;

/// Expire stale leases for a kluster before listing tasks.
async fn expire_stale_leases(db: &sqlx::PgPool, kluster_id: &str) {
    let now = Utc::now().naive_utc();
    let _ = sqlx::query(
        "UPDATE meshtask SET status='ready', claimed_by_agent_id=NULL, lease_expires_at=NULL, updated_at=$1 \
         WHERE kluster_id=$2 AND status IN ('claimed','running') AND claim_policy != 'broadcast' \
           AND lease_expires_at IS NOT NULL AND lease_expires_at < $1",
    )
    .bind(now)
    .bind(kluster_id)
    .execute(db)
    .await;
}

/// DFS cycle detection when adding a new task with dependencies.
async fn detect_cycle(
    db: &sqlx::PgPool,
    kluster_id: &str,
    new_id: &str,
    depends_on: &[String],
) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query("SELECT id, depends_on FROM meshtask WHERE kluster_id=$1")
        .bind(kluster_id)
        .fetch_all(db)
        .await?;
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for r in &rows {
        let id: String = r.get("id");
        let deps: Vec<String> =
            serde_json::from_str(r.get::<&str, _>("depends_on")).unwrap_or_default();
        adj.insert(id, deps);
    }
    adj.insert(new_id.to_string(), depends_on.to_vec());

    // Iterative DFS
    let mut color: HashMap<String, u8> = HashMap::new();
    let mut stack: Vec<(String, usize)> = vec![(new_id.to_string(), 0)];
    color.insert(new_id.to_string(), 1);
    while let Some(top) = stack.last_mut() {
        let (node, idx) = top;
        let neighbors: Vec<String> = adj.get(node.as_str()).cloned().unwrap_or_default();
        if *idx >= neighbors.len() {
            color.insert(node.clone(), 2);
            stack.pop();
            continue;
        }
        let nb = neighbors[*idx].clone();
        *idx += 1;
        let s = *color.get(&nb).unwrap_or(&0);
        if s == 1 {
            return Ok(true);
        }
        if s == 0 {
            color.insert(nb.clone(), 1);
            stack.push((nb, 0));
        }
    }
    Ok(false)
}

/// After a task finishes, find and unblock any dependents whose deps are all finished.
async fn unblock_dependents(db: &sqlx::PgPool, kluster_id: &str, finished_id: &str) -> Vec<String> {
    let candidates = sqlx::query(
        "SELECT id, depends_on FROM meshtask WHERE kluster_id=$1 AND status IN ('pending','blocked')",
    )
    .bind(kluster_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let now = Utc::now().naive_utc();
    let mut ready_ids = Vec::new();

    for c in &candidates {
        let cid: String = c.get("id");
        let dep_ids: Vec<String> =
            serde_json::from_str(c.get::<&str, _>("depends_on")).unwrap_or_default();
        if !dep_ids.contains(&finished_id.to_string()) {
            continue;
        }
        // Check all deps are finished
        let dep_rows = sqlx::query("SELECT status FROM meshtask WHERE id = ANY($1)")
            .bind(dep_ids.as_slice())
            .fetch_all(db)
            .await
            .unwrap_or_default();
        if dep_rows.len() == dep_ids.len()
            && dep_rows
                .iter()
                .all(|r| r.get::<String, _>("status") == "finished")
        {
            let _ = sqlx::query("UPDATE meshtask SET status='ready', updated_at=$2 WHERE id=$1")
                .bind(&cid)
                .bind(now)
                .execute(db)
                .await;
            ready_ids.push(cid);
        }
    }
    ready_ids
}

// ── Task handlers ──────────────────────────────────────────────────────────────

async fn list_tasks(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(kluster_id): Path<String>,
    Query(q): Query<TaskListQuery>,
) -> impl IntoResponse {
    expire_stale_leases(&state.db, &kluster_id).await;

    let rows = if let Some(status) = &q.status {
        sqlx::query(
            "SELECT * FROM meshtask WHERE kluster_id=$1 AND status=$2 ORDER BY priority DESC, created_at ASC",
        )
        .bind(&kluster_id)
        .bind(status)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query(
            "SELECT * FROM meshtask WHERE kluster_id=$1 ORDER BY priority DESC, created_at ASC",
        )
        .bind(&kluster_id)
        .fetch_all(&state.db)
        .await
    };

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_task).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_tasks: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(kluster_id): Path<String>,
    Json(body): Json<TaskCreate>,
) -> impl IntoResponse {
    // Resolve mission_id from kluster
    let kluster_row = sqlx::query("SELECT id, mission_id FROM kluster WHERE id=$1")
        .bind(&kluster_id)
        .fetch_optional(&state.db)
        .await;
    let mission_id = match kluster_row {
        Ok(Some(r)) => r.get::<Option<String>, _>("mission_id").unwrap_or_default(),
        Ok(None) => return not_found("Kluster not found"),
        Err(e) => {
            tracing::error!("create_task fetch kluster: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Validate depends_on tasks exist
    for dep_id in &body.depends_on {
        let exists: Option<i32> =
            sqlx::query_scalar("SELECT 1 FROM meshtask WHERE id=$1 AND kluster_id=$2")
                .bind(dep_id)
                .bind(&kluster_id)
                .fetch_optional(&state.db)
                .await
                .unwrap_or(None);
        if exists.is_none() {
            return bad_request(&format!("Dependency task not found: {dep_id}"));
        }
    }

    let new_id = Uuid::new_v4().to_string();

    // Detect cycles if there are dependencies
    if !body.depends_on.is_empty() {
        match detect_cycle(&state.db, &kluster_id, &new_id, &body.depends_on).await {
            Ok(true) => return bad_request("Dependency cycle detected"),
            Ok(false) => {}
            Err(e) => {
                tracing::error!("detect_cycle: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    // Determine initial status: pending if has unfinished deps, else ready
    let initial_status = if body.depends_on.is_empty() {
        "ready"
    } else {
        // Check if all deps are already finished
        let dep_rows = sqlx::query("SELECT status FROM meshtask WHERE id = ANY($1)")
            .bind(body.depends_on.as_slice())
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
        if dep_rows.len() == body.depends_on.len()
            && dep_rows
                .iter()
                .all(|r| r.get::<String, _>("status") == "finished")
        {
            "ready"
        } else {
            "pending"
        }
    };

    let now = Utc::now().naive_utc();
    let depends_on_json =
        serde_json::to_string(&body.depends_on).unwrap_or_else(|_| "[]".to_string());
    let produces_json =
        serde_json::to_string(&body.produces).unwrap_or_else(|_| "{}".to_string());
    let consumes_json =
        serde_json::to_string(&body.consumes).unwrap_or_else(|_| "{}".to_string());
    let req_caps_json =
        serde_json::to_string(&body.required_capabilities).unwrap_or_else(|_| "[]".to_string());

    let row = sqlx::query(
        "INSERT INTO meshtask (id, kluster_id, mission_id, parent_task_id, title, description, \
         input_json, claim_policy, depends_on, produces, consumes, required_capabilities, \
         status, claimed_by_agent_id, result_artifact_id, priority, \
         lease_expires_at, claim_lease_id, version_counter, \
         created_by_subject, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,NULL,NULL,$14,NULL,NULL,0,$15,$16,$16) \
         RETURNING *",
    )
    .bind(&new_id)
    .bind(&kluster_id)
    .bind(&mission_id)
    .bind(&body.parent_task_id)
    .bind(&body.title)
    .bind(&body.description)
    .bind(&body.input_json)
    .bind(&body.claim_policy)
    .bind(&depends_on_json)
    .bind(&produces_json)
    .bind(&consumes_json)
    .bind(&req_caps_json)
    .bind(initial_status)
    .bind(body.priority)
    .bind(&principal.subject)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(r) => {
            if initial_status == "ready" {
                let tid: String = r.get("id");
                broadcast_task_available(&mission_id, &kluster_id, &tid).await;
            }
            (StatusCode::CREATED, Json(row_to_task(&r))).into_response()
        }
        Err(e) => {
            tracing::error!("create_task insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn task_graph(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(kluster_id): Path<String>,
) -> impl IntoResponse {
    let rows = sqlx::query("SELECT id, title, status, depends_on FROM meshtask WHERE kluster_id=$1")
        .bind(&kluster_id)
        .fetch_all(&state.db)
        .await;

    match rows {
        Ok(rows) => {
            let nodes: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<String, _>("id"),
                        "title": r.get::<String, _>("title"),
                        "status": r.get::<String, _>("status"),
                    })
                })
                .collect();

            let mut edges: Vec<serde_json::Value> = Vec::new();
            for r in &rows {
                let from: String = r.get("id");
                let deps: Vec<String> =
                    serde_json::from_str(r.get::<&str, _>("depends_on")).unwrap_or_default();
                for dep in deps {
                    edges.push(serde_json::json!({"from": dep, "to": from}));
                }
            }

            Json(serde_json::json!({"nodes": nodes, "edges": edges})).into_response()
        }
        Err(e) => {
            tracing::error!("task_graph: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_task(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    match sqlx::query("SELECT * FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => not_found("Task not found"),
        Err(e) => {
            tracing::error!("get_task: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn cancel_task(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await;

    let task_row = match row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("cancel_task fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let status: String = task_row.get("status");
    if status == "finished" || status == "cancelled" {
        return conflict(&format!("Task is already {status}"));
    }

    let now = Utc::now().naive_utc();
    match sqlx::query(
        "UPDATE meshtask SET status='cancelled', claimed_by_agent_id=NULL, \
         lease_expires_at=NULL, updated_at=$2 WHERE id=$1 RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => Json(row_to_task(&r)).into_response(),
        Err(e) => {
            tracing::error!("cancel_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn retry_task(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT * FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await;

    let task_row = match row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("retry_task fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let status: String = task_row.get("status");
    if status != "failed" && status != "cancelled" {
        return conflict(&format!("Task cannot be retried from status: {status}"));
    }

    let now = Utc::now().naive_utc();
    match sqlx::query(
        "UPDATE meshtask SET status='ready', claimed_by_agent_id=NULL, result_artifact_id=NULL, \
         lease_expires_at=NULL, claim_lease_id=NULL, updated_at=$2 WHERE id=$1 RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => Json(row_to_task(&r)).into_response(),
        Err(e) => {
            tracing::error!("retry_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn claim_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let agent_id = body
        .as_ref()
        .and_then(|b| b.get("agent_id"))
        .and_then(|v| v.as_str())
        .unwrap_or(&principal.subject)
        .to_string();

    // First fetch the task to check claim_policy
    let task_row = match sqlx::query("SELECT * FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("claim_task fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let claim_policy: String = task_row.get("claim_policy");
    let status: String = task_row.get("status");

    if status != "ready" {
        return (
            StatusCode::LOCKED,
            Json(serde_json::json!({"detail": "Task not available for claiming"})),
        )
            .into_response();
    }

    let now = Utc::now().naive_utc();
    let lease_expires =
        now + chrono::Duration::seconds(LEASE_TTL_SECS);
    let lease_id = Uuid::new_v4().to_string();

    // Broadcast: no locking needed, just update status to running
    if claim_policy == "broadcast" {
        let row = sqlx::query(
            "UPDATE meshtask SET status='running', claimed_by_agent_id=$2, \
             claim_lease_id=$3, lease_expires_at=$4, updated_at=$5 \
             WHERE id=$1 RETURNING *",
        )
        .bind(&task_id)
        .bind(&agent_id)
        .bind(&lease_id)
        .bind(lease_expires)
        .bind(now)
        .fetch_optional(&state.db)
        .await;

        return match row {
            Ok(Some(r)) => {
                let mut val = row_to_task(&r);
                val["claim_lease_id"] = serde_json::json!(lease_id);
                val["task_id"] = serde_json::json!(task_id);
                Json(val).into_response()
            }
            Ok(None) => (
                StatusCode::LOCKED,
                Json(serde_json::json!({"detail": "Task not available for claiming"})),
            )
                .into_response(),
            Err(e) => {
                tracing::error!("claim_task broadcast update: {e}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        };
    }

    // Exclusive claim: use FOR UPDATE SKIP LOCKED
    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("claim_task begin tx: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let locked = sqlx::query(
        "SELECT * FROM meshtask WHERE id=$1 AND status='ready' FOR UPDATE SKIP LOCKED",
    )
    .bind(&task_id)
    .fetch_optional(&mut *tx)
    .await;

    let locked_row = match locked {
        Ok(Some(r)) => r,
        Ok(None) => {
            let _ = tx.rollback().await;
            return (
                StatusCode::LOCKED,
                Json(serde_json::json!({"detail": "Task not available for claiming"})),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("claim_task lock: {e}");
            let _ = tx.rollback().await;
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let version_counter: i32 = locked_row.get("version_counter");
    let new_version = version_counter + 1;

    let updated = sqlx::query(
        "UPDATE meshtask SET status='claimed', claimed_by_agent_id=$2, claim_lease_id=$3, \
         version_counter=$4, lease_expires_at=$5, updated_at=$6 \
         WHERE id=$1 AND version_counter=$7 RETURNING *",
    )
    .bind(&task_id)
    .bind(&agent_id)
    .bind(&lease_id)
    .bind(new_version)
    .bind(lease_expires)
    .bind(now)
    .bind(version_counter)
    .fetch_optional(&mut *tx)
    .await;

    match updated {
        Ok(Some(r)) => {
            if let Err(e) = tx.commit().await {
                tracing::error!("claim_task commit: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            let mut val = row_to_task(&r);
            val["claim_lease_id"] = serde_json::json!(lease_id);
            val["task_id"] = serde_json::json!(task_id);
            Json(val).into_response()
        }
        Ok(None) => {
            let _ = tx.rollback().await;
            conflict("Claim lost to concurrent claimer")
        }
        Err(e) => {
            tracing::error!("claim_task CAS update: {e}");
            let _ = tx.rollback().await;
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn heartbeat_task(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<HeartbeatBody>>,
) -> impl IntoResponse {
    let body = body.map(|b| b.0).unwrap_or_default();

    let row = sqlx::query("SELECT * FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await;

    let task_row = match row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("heartbeat_task fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let status: String = task_row.get("status");
    if status != "claimed" && status != "running" {
        return conflict(&format!("Task is not in a claimable state: {status}"));
    }

    // Lease ID mismatch check
    if let Some(caller_lease) = &body.claim_lease_id {
        let task_lease: Option<String> = task_row.get("claim_lease_id");
        if let Some(tl) = task_lease {
            if &tl != caller_lease {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"detail": "Lease ID mismatch"})),
                )
                    .into_response();
            }
        }
    }

    let now = Utc::now().naive_utc();
    let lease_expires = now + chrono::Duration::seconds(LEASE_TTL_SECS);

    match sqlx::query(
        "UPDATE meshtask SET status='running', lease_expires_at=$2, updated_at=$3 WHERE id=$1 RETURNING *",
    )
    .bind(&task_id)
    .bind(lease_expires)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => Json(row_to_task(&r)).into_response(),
        Err(e) => {
            tracing::error!("heartbeat_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn append_progress(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
    Json(body): Json<ProgressCreate>,
) -> impl IntoResponse {
    // Verify task exists
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Task not found");
    }

    // Get next sequence number
    let seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), -1) + 1 FROM meshprogressevent WHERE task_id=$1",
    )
    .bind(&task_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    let now = Utc::now().naive_utc();
    // agent_id from body optional field or empty
    let agent_id = ""; // caller may pass agent_id in body; using empty default

    let row = sqlx::query(
        "INSERT INTO meshprogressevent (task_id, agent_id, seq, event_type, phase, step, summary, \
         payload_json, occurred_at, agent_run_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING *",
    )
    .bind(&task_id)
    .bind(agent_id)
    .bind(seq as i32)
    .bind(&body.event_type)
    .bind(&body.phase)
    .bind(&body.step)
    .bind(&body.summary)
    .bind(&body.payload_json)
    .bind(now)
    .bind(&body.agent_run_id)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(r) => {
            Json(serde_json::json!({
                "id": r.get::<i32, _>("id"),
                "task_id": r.get::<String, _>("task_id"),
                "agent_id": r.get::<String, _>("agent_id"),
                "seq": r.get::<i32, _>("seq"),
                "event_type": r.get::<String, _>("event_type"),
                "phase": r.get::<Option<String>, _>("phase"),
                "step": r.get::<Option<String>, _>("step"),
                "summary": r.get::<String, _>("summary"),
                "payload_json": serde_json::from_str::<serde_json::Value>(r.get::<&str, _>("payload_json")).unwrap_or(serde_json::json!({})),
                "occurred_at": r.get::<chrono::NaiveDateTime, _>("occurred_at"),
                "agent_run_id": r.get::<Option<String>, _>("agent_run_id"),
            }))
            .into_response()
        }
        Err(e) => {
            tracing::error!("append_progress: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn complete_task(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<CompleteBody>>,
) -> impl IntoResponse {
    let body = body.map(|b| b.0).unwrap_or_default();

    let task_row = match sqlx::query("SELECT * FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("complete_task fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let status: String = task_row.get("status");
    if status != "claimed" && status != "running" && status != "waiting_review" {
        return conflict(&format!("Task cannot be completed from status: {status}"));
    }

    // Lease ID mismatch check
    if let Some(caller_lease) = &body.claim_lease_id {
        let task_lease: Option<String> = task_row.get("claim_lease_id");
        if let Some(tl) = task_lease {
            if &tl != caller_lease {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"detail": "Lease ID mismatch"})),
                )
                    .into_response();
            }
        }
    }

    let kluster_id: String = task_row.get("kluster_id");
    let now = Utc::now().naive_utc();

    // Check for pending review gates
    let pending_gates = sqlx::query(
        "SELECT id FROM reviewgate WHERE mesh_task_id=$1 AND status='pending'",
    )
    .bind(&task_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    if !pending_gates.is_empty() {
        let gate_ids: Vec<String> = pending_gates.iter().map(|r| r.get::<String, _>("id")).collect();
        let _ = sqlx::query(
            "UPDATE meshtask SET status='waiting_review', updated_at=$2 WHERE id=$1",
        )
        .bind(&task_id)
        .bind(now)
        .execute(&state.db)
        .await;

        return Json(serde_json::json!({
            "status": "waiting_review",
            "pending_gates": gate_ids,
            "task_id": task_id,
        }))
        .into_response();
    }

    // Complete the task
    match sqlx::query(
        "UPDATE meshtask SET status='finished', result_artifact_id=$2, \
         lease_expires_at=NULL, updated_at=$3 WHERE id=$1 RETURNING *",
    )
    .bind(&task_id)
    .bind(&body.result_artifact_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => {
            let mission_id: String = r.get("mission_id");
            let unblocked = unblock_dependents(&state.db, &kluster_id, &task_id).await;
            for tid in &unblocked {
                broadcast_task_available(&mission_id, &kluster_id, tid).await;
            }
            let mut val = row_to_task(&r);
            val["unblocked_tasks"] = serde_json::json!(unblocked);
            Json(val).into_response()
        }
        Err(e) => {
            tracing::error!("complete_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn fail_task(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<FailBody>>,
) -> impl IntoResponse {
    let body = body.map(|b| b.0).unwrap_or_default();

    let task_row = match sqlx::query("SELECT * FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("fail_task fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let status: String = task_row.get("status");
    if status != "claimed" && status != "running" && status != "waiting_review" {
        return conflict(&format!("Task cannot be failed from status: {status}"));
    }

    // Lease ID mismatch check
    if let Some(caller_lease) = &body.claim_lease_id {
        let task_lease: Option<String> = task_row.get("claim_lease_id");
        if let Some(tl) = task_lease {
            if &tl != caller_lease {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"detail": "Lease ID mismatch"})),
                )
                    .into_response();
            }
        }
    }

    let now = Utc::now().naive_utc();
    match sqlx::query(
        "UPDATE meshtask SET status='failed', lease_expires_at=NULL, updated_at=$2 WHERE id=$1 RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => Json(row_to_task(&r)).into_response(),
        Err(e) => {
            tracing::error!("fail_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn block_task(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Task not found");
    }

    let now = Utc::now().naive_utc();
    match sqlx::query(
        "UPDATE meshtask SET status='blocked', updated_at=$2 WHERE id=$1 RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => Json(row_to_task(&r)).into_response(),
        Err(e) => {
            tracing::error!("block_task: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn unblock_task(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Task not found");
    }

    let now = Utc::now().naive_utc();
    match sqlx::query(
        "UPDATE meshtask SET status='ready', updated_at=$2 WHERE id=$1 RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => Json(row_to_task(&r)).into_response(),
        Err(e) => {
            tracing::error!("unblock_task: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_task_progress(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
    Query(q): Query<ProgressQuery>,
) -> impl IntoResponse {
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Task not found");
    }

    let rows = sqlx::query(
        "SELECT * FROM meshprogressevent WHERE task_id=$1 AND seq > $2 ORDER BY seq ASC",
    )
    .bind(&task_id)
    .bind(q.since_seq)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => {
            let events: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.get::<i64, _>("id"),
                        "task_id": r.get::<String, _>("task_id"),
                        "agent_id": r.get::<String, _>("agent_id"),
                        "seq": r.get::<i32, _>("seq"),
                        "event_type": r.get::<String, _>("event_type"),
                        "phase": r.get::<Option<String>, _>("phase"),
                        "step": r.get::<Option<String>, _>("step"),
                        "summary": r.get::<String, _>("summary"),
                        "payload_json": serde_json::from_str::<serde_json::Value>(r.get::<&str, _>("payload_json")).unwrap_or(serde_json::json!({})),
                        "occurred_at": r.get::<chrono::NaiveDateTime, _>("occurred_at"),
                        "agent_run_id": r.get::<Option<String>, _>("agent_run_id"),
                    })
                })
                .collect();
            Json(events).into_response()
        }
        Err(e) => {
            tracing::error!("get_task_progress: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Gate handlers ──────────────────────────────────────────────────────────────

async fn create_gate(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    Json(body): Json<GateCreate>,
) -> impl IntoResponse {
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Task not found");
    }

    let gate_id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    let row = sqlx::query(
        "INSERT INTO reviewgate (id, owner_subject, mesh_task_id, run_id, gate_type, \
         required_approvals, status, approval_request_id, ai_pending_action_id, policy_rule_id, \
         created_at, resolved_at) \
         VALUES ($1,$2,$3,$4,$5,$6,'pending',$7,NULL,NULL,$8,NULL) RETURNING *",
    )
    .bind(&gate_id)
    .bind(&principal.subject)
    .bind(&task_id)
    .bind(&body.run_id)
    .bind(&body.gate_type)
    .bind(&body.required_approvals)
    .bind(&body.approval_request_id)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(r) => (StatusCode::CREATED, Json(row_to_gate(&r))).into_response(),
        Err(e) => {
            tracing::error!("create_gate: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_gates(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Task not found");
    }

    match sqlx::query("SELECT * FROM reviewgate WHERE mesh_task_id=$1 ORDER BY created_at ASC")
        .bind(&task_id)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_gate).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_gates: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn resolve_gate(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((task_id, gate_id)): Path<(String, String)>,
    Json(body): Json<GateResolve>,
) -> impl IntoResponse {
    if body.decision != "approved" && body.decision != "rejected" {
        return bad_request("decision must be 'approved' or 'rejected'");
    }

    // Fetch gate and verify it belongs to this task and owner
    let gate_row = match sqlx::query(
        "SELECT * FROM reviewgate WHERE id=$1 AND mesh_task_id=$2",
    )
    .bind(&gate_id)
    .bind(&task_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Gate not found"),
        Err(e) => {
            tracing::error!("resolve_gate fetch gate: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let owner: String = gate_row.get("owner_subject");
    if owner != principal.subject && !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "Not authorized to resolve this gate"})),
        )
            .into_response();
    }

    let gate_status: String = gate_row.get("status");
    if gate_status != "pending" {
        return conflict(&format!("Gate is already {gate_status}"));
    }

    let now = Utc::now().naive_utc();

    // Update the gate
    let updated_gate = sqlx::query(
        "UPDATE reviewgate SET status=$2, resolved_at=$3 WHERE id=$1 RETURNING *",
    )
    .bind(&gate_id)
    .bind(&body.decision)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    let gate_val = match updated_gate {
        Ok(r) => row_to_gate(&r),
        Err(e) => {
            tracing::error!("resolve_gate update: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Re-fetch task to check if waiting_review
    let task_row = match sqlx::query("SELECT * FROM meshtask WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return Json(gate_val).into_response(),
        Err(e) => {
            tracing::error!("resolve_gate fetch task: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let task_status: String = task_row.get("status");
    let kluster_id: String = task_row.get("kluster_id");

    if task_status == "waiting_review" {
        // Re-fetch all gates for this task
        let all_gates = sqlx::query("SELECT status FROM reviewgate WHERE mesh_task_id=$1")
            .bind(&task_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

        let any_rejected = all_gates
            .iter()
            .any(|r| r.get::<String, _>("status") == "rejected");
        let all_resolved = all_gates
            .iter()
            .all(|r| {
                let s: String = r.get("status");
                s == "approved" || s == "expired"
            });

        if any_rejected {
            let _ = sqlx::query(
                "UPDATE meshtask SET status='failed', updated_at=$2 WHERE id=$1",
            )
            .bind(&task_id)
            .bind(now)
            .execute(&state.db)
            .await;
        } else if all_resolved {
            let _ = sqlx::query(
                "UPDATE meshtask SET status='finished', lease_expires_at=NULL, updated_at=$2 WHERE id=$1",
            )
            .bind(&task_id)
            .bind(now)
            .execute(&state.db)
            .await;
            let mission_id: String = task_row.get("mission_id");
            let unblocked = unblock_dependents(&state.db, &kluster_id, &task_id).await;
            for tid in &unblocked {
                broadcast_task_available(&mission_id, &kluster_id, tid).await;
            }
        }
        // else: some still pending, leave as waiting_review
    }

    Json(gate_val).into_response()
}

// ── Agent notify WebSocket ────────────────────────────────────────────────────

async fn agent_notify_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_agent_notify(socket, state, agent_id))
}

async fn handle_agent_notify(mut socket: WebSocket, state: Arc<AppState>, agent_id: String) {
    // Look up the agent's mission so we can subscribe to the right channel.
    let mission_id = match sqlx::query_scalar::<_, String>(
        "SELECT mission_id FROM meshagent WHERE id=$1",
    )
    .bind(&agent_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(m)) => m,
        _ => return,
    };

    // Subscribe to the mission's broadcast channel, creating it on demand.
    let mut rx = {
        let mut reg = notify_registry().lock().await;
        let tx = reg
            .entry(mission_id.clone())
            .or_insert_with(|| broadcast::channel::<String>(64).0);
        tx.subscribe()
    };

    // Forward notifications; send pings every 30s to keep the connection alive.
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
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
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

// ── Agent handlers ─────────────────────────────────────────────────────────────

async fn enroll_agent(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Json(body): Json<AgentEnroll>,
) -> impl IntoResponse {
    // Verify mission exists
    let mission_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if mission_exists.is_none() {
        return not_found("Mission not found");
    }

    // If runtime_node_id provided, validate it exists and belongs to principal
    if let Some(ref rn_id) = body.runtime_node_id {
        let rn_row = sqlx::query("SELECT id, owner_subject FROM runtimenode WHERE id=$1")
            .bind(rn_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);
        match rn_row {
            None => return bad_request("RuntimeNode not found"),
            Some(r) => {
                let rn_owner: String = r.get("owner_subject");
                if rn_owner != principal.subject && !principal.is_admin {
                    return bad_request("RuntimeNode does not belong to you");
                }
            }
        }
    }

    let agent_id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    let caps_json = serde_json::to_string(&body.capabilities).unwrap_or_else(|_| "[]".to_string());
    let labels_json =
        serde_json::to_string(&body.labels).unwrap_or_else(|_| "{}".to_string());
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

    // Resolve the persistent agent identity link, if the caller asked for
    // one. Failures bubble up — a reserved name or DB error should fail the
    // enrollment cleanly rather than orphan a meshagent row.
    let agent_public_id = match &body.agent_name {
        Some(n) if !n.trim().is_empty() => {
            match crate::routes::agents::upsert_agent_by_name(&state.db, n.trim(), &caps_json)
                .await
            {
                Ok(pid) => Some(pid),
                Err(e) => {
                    tracing::error!("enroll_agent agent link: {e}");
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
        "INSERT INTO meshagent (id, mission_id, node_id, runtime_kind, runtime_version, \
         capabilities, labels, status, current_task_id, enrolled_by_subject, enrolled_at, \
         last_heartbeat_at, runtime_node_id, profile_json, machine_json, runtime_json, \
         supervision_mode, agent_public_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,'online',NULL,$8,$9,NULL,$10,$11,$12,$13,NULL,$14) RETURNING *",
    )
    .bind(&agent_id)
    .bind(&mission_id)
    .bind(&body.node_id)
    .bind(&body.runtime_kind)
    .bind(&body.runtime_version)
    .bind(&caps_json)
    .bind(&labels_json)
    .bind(&principal.subject)
    .bind(now)
    .bind(&body.runtime_node_id)
    .bind(&profile_json)
    .bind(&machine_json)
    .bind(&runtime_json)
    .bind(&agent_public_id)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(r) => {
            let agent_json = row_to_agent(&r);
            // Notify the daemon for this node, if any, so it can spawn the
            // supervisor live. No-op when no `runtime_node_id` was set.
            if let Some(rn_id) = body.runtime_node_id.as_deref() {
                broadcast_assignment_changed(
                    rn_id,
                    serde_json::json!({
                        "type": "agent.assigned",
                        "agent_id": agent_json["id"],
                        "agent": agent_json,
                    }),
                )
                .await;
            }
            (StatusCode::CREATED, Json(agent_json)).into_response()
        }
        Err(e) => {
            tracing::error!("enroll_agent: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_mission_agents(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    let mission_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if mission_exists.is_none() {
        return not_found("Mission not found");
    }

    match sqlx::query("SELECT * FROM meshagent WHERE mission_id=$1 ORDER BY enrolled_at ASC")
        .bind(&mission_id)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_agent).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_mission_agents: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn agent_heartbeat(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshagent WHERE id=$1")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Agent not found");
    }

    let now = Utc::now().naive_utc();
    match sqlx::query(
        "UPDATE meshagent SET last_heartbeat_at=$2 WHERE id=$1 RETURNING *",
    )
    .bind(&agent_id)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(r) => Json(row_to_agent(&r)).into_response(),
        Err(e) => {
            tracing::error!("agent_heartbeat: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn set_agent_status(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(agent_id): Path<String>,
    Query(q): Query<AgentStatusQuery>,
) -> impl IntoResponse {
    let valid = ["online", "busy", "idle", "offline", "errored"];
    if !valid.contains(&q.status.as_str()) {
        return bad_request(&format!("Invalid status: {}. Must be one of: online, busy, idle, offline, errored", q.status));
    }

    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshagent WHERE id=$1")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Agent not found");
    }

    match sqlx::query("UPDATE meshagent SET status=$2 WHERE id=$1 RETURNING *")
        .bind(&agent_id)
        .bind(&q.status)
        .fetch_one(&state.db)
        .await
    {
        Ok(r) => Json(row_to_agent(&r)).into_response(),
        Err(e) => {
            tracing::error!("set_agent_status: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_agent_profile(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(agent_id): Path<String>,
    Json(body): Json<AgentProfileUpdate>,
) -> impl IntoResponse {
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshagent WHERE id=$1")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Agent not found");
    }

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

    // Merge: only update fields that are provided
    let row = sqlx::query(
        "UPDATE meshagent SET \
         profile_json = COALESCE($2, profile_json), \
         machine_json = COALESCE($3, machine_json), \
         runtime_json = COALESCE($4, runtime_json) \
         WHERE id=$1 RETURNING *",
    )
    .bind(&agent_id)
    .bind(&profile_json)
    .bind(&machine_json)
    .bind(&runtime_json)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(r) => Json(row_to_agent(&r)).into_response(),
        Err(e) => {
            tracing::error!("update_agent_profile: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_agent(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match sqlx::query("SELECT * FROM meshagent WHERE id=$1")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => Json(row_to_agent(&r)).into_response(),
        Ok(None) => not_found("Agent not found"),
        Err(e) => {
            tracing::error!("get_agent: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_agent_messages(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(agent_id): Path<String>,
    Query(q): Query<AgentMessagesQuery>,
) -> impl IntoResponse {
    // Look up the agent's mission so we can also surface mission broadcasts.
    let mission_id: String = match sqlx::query("SELECT mission_id FROM meshagent WHERE id=$1")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r.get("mission_id"),
        Ok(None) => return not_found("Agent not found"),
        Err(e) => {
            tracing::error!("get_agent_messages lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Direct messages addressed to this agent + mission-scoped broadcasts.
    // since_id paging in the caller dedupes broadcasts across polls.
    match sqlx::query(
        "SELECT * FROM meshmessage \
         WHERE id > $1 \
           AND (to_agent_id = $2 \
                OR (to_agent_id IS NULL AND mission_id = $3)) \
         ORDER BY id ASC",
    )
    .bind(q.since_id)
    .bind(&agent_id)
    .bind(&mission_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            // Mark only direct messages as read — broadcasts have N recipients
            // and would need a per-recipient read table to track properly.
            let direct_ids: Vec<i64> = rows
                .iter()
                .filter(|r| r.get::<Option<String>, _>("to_agent_id").is_some())
                .map(|r| r.get::<i64, _>("id"))
                .collect();
            if !direct_ids.is_empty() {
                let now = Utc::now().naive_utc();
                let _ = sqlx::query("UPDATE meshmessage SET read_at=$2 WHERE id = ANY($1) AND read_at IS NULL")
                    .bind(direct_ids.as_slice())
                    .bind(now)
                    .execute(&state.db)
                    .await;
            }
            Json(rows.iter().map(row_to_message).collect::<Vec<_>>()).into_response()
        }
        Err(e) => {
            tracing::error!("get_agent_messages: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Mission message handlers ───────────────────────────────────────────────────

async fn list_mission_messages(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(mission_id): Path<String>,
    Query(q): Query<MessageListQuery>,
) -> impl IntoResponse {
    let mission_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if mission_exists.is_none() {
        return not_found("Mission not found");
    }

    let since_id = q.since_id.unwrap_or(0);

    let rows = if let Some(channel) = &q.channel {
        sqlx::query(
            "SELECT * FROM meshmessage WHERE mission_id=$1 AND channel=$2 AND id > $3 \
             ORDER BY id ASC",
        )
        .bind(&mission_id)
        .bind(channel)
        .bind(since_id)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query(
            "SELECT * FROM meshmessage WHERE mission_id=$1 AND id > $2 ORDER BY id ASC",
        )
        .bind(&mission_id)
        .bind(since_id)
        .fetch_all(&state.db)
        .await
    };

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_message).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_mission_messages: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn send_mission_message(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Json(body): Json<MessageCreate>,
) -> impl IntoResponse {
    let mission_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if mission_exists.is_none() {
        return not_found("Mission not found");
    }

    let body_json_str = if let Some(ref v) = body.body {
        serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
    } else {
        body.body_json.clone()
    };

    let now = Utc::now().naive_utc();

    let row = sqlx::query(
        "INSERT INTO meshmessage (mission_id, kluster_id, from_agent_id, to_agent_id, task_id, \
         channel, body_json, in_reply_to, created_at, read_at) \
         VALUES ($1,NULL,$2,$3,$4,$5,$6,$7,$8,NULL) RETURNING id, created_at",
    )
    .bind(&mission_id)
    .bind(&principal.subject)
    .bind(&body.to_agent_id)
    .bind(&body.task_id)
    .bind(&body.channel)
    .bind(&body_json_str)
    .bind(body.in_reply_to)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(r) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": r.get::<i64, _>("id"),
                "created_at": r.get::<chrono::NaiveDateTime, _>("created_at"),
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("send_mission_message: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn mission_roster(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    let mission_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if mission_exists.is_none() {
        return not_found("Mission not found");
    }

    // Return agents grouped by status
    let agents = sqlx::query("SELECT * FROM meshagent WHERE mission_id=$1 ORDER BY enrolled_at ASC")
        .bind(&mission_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let mut roster: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for row in &agents {
        let status: String = row.get("status");
        roster.entry(status).or_default().push(row_to_agent(row));
    }

    Json(serde_json::json!({
        "mission_id": mission_id,
        "agents": agents.iter().map(row_to_agent).collect::<Vec<_>>(),
        "by_status": roster,
        "total": agents.len(),
    }))
    .into_response()
}

// ── Kluster message handlers ───────────────────────────────────────────────────

async fn list_kluster_messages(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(kluster_id): Path<String>,
    Query(q): Query<MessageListQuery>,
) -> impl IntoResponse {
    let kluster_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM kluster WHERE id=$1")
        .bind(&kluster_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if kluster_exists.is_none() {
        return not_found("Kluster not found");
    }

    let since_id = q.since_id.unwrap_or(0);

    let rows = if let Some(channel) = &q.channel {
        sqlx::query(
            "SELECT * FROM meshmessage WHERE kluster_id=$1 AND channel=$2 AND id > $3 \
             ORDER BY id ASC",
        )
        .bind(&kluster_id)
        .bind(channel)
        .bind(since_id)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query(
            "SELECT * FROM meshmessage WHERE kluster_id=$1 AND id > $2 ORDER BY id ASC",
        )
        .bind(&kluster_id)
        .bind(since_id)
        .fetch_all(&state.db)
        .await
    };

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_message).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_kluster_messages: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn send_kluster_message(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(kluster_id): Path<String>,
    Json(body): Json<MessageCreate>,
) -> impl IntoResponse {
    let kluster_row = sqlx::query("SELECT id, mission_id FROM kluster WHERE id=$1")
        .bind(&kluster_id)
        .fetch_optional(&state.db)
        .await;

    let mission_id = match kluster_row {
        Ok(Some(r)) => r.get::<Option<String>, _>("mission_id").unwrap_or_default(),
        Ok(None) => return not_found("Kluster not found"),
        Err(e) => {
            tracing::error!("send_kluster_message fetch kluster: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let body_json_str = if let Some(ref v) = body.body {
        serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
    } else {
        body.body_json.clone()
    };

    let now = Utc::now().naive_utc();

    let row = sqlx::query(
        "INSERT INTO meshmessage (mission_id, kluster_id, from_agent_id, to_agent_id, task_id, \
         channel, body_json, in_reply_to, created_at, read_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,NULL) RETURNING id, created_at",
    )
    .bind(&mission_id)
    .bind(&kluster_id)
    .bind(&principal.subject)
    .bind(&body.to_agent_id)
    .bind(&body.task_id)
    .bind(&body.channel)
    .bind(&body_json_str)
    .bind(body.in_reply_to)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(r) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": r.get::<i64, _>("id"),
                "created_at": r.get::<chrono::NaiveDateTime, _>("created_at"),
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("send_kluster_message: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── WebSocket streams ──────────────────────────────────────────────────────────

async fn kluster_stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(kluster_id): Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| poll_ledger_stream(socket, state, "kluster_id".into(), kluster_id))
}

async fn mission_stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| poll_ledger_stream(socket, state, "mission_id".into(), mission_id))
}

async fn poll_ledger_stream(
    mut socket: WebSocket,
    state: Arc<AppState>,
    filter_col: String,
    filter_val: String,
) {
    let mut last_id: i32 = 0;
    let mut ticks_since_ping: u32 = 0;
    loop {
        // Fetch new events since last seen id
        let query_str = format!(
            "SELECT id, event_id, entity_type, entity_id, action, state, created_at \
             FROM ledgerevent WHERE {filter_col}=$1 AND id>$2 ORDER BY id ASC LIMIT 50"
        );
        let rows = sqlx::query(&query_str)
            .bind(&filter_val)
            .bind(last_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

        for row in &rows {
            let id: i32 = row.get("id");
            if id > last_id { last_id = id; }
            let evt = serde_json::json!({
                "type": "event",
                "id": id,
                "event_id": row.get::<String, _>("event_id"),
                "entity_type": row.get::<String, _>("entity_type"),
                "entity_id": row.get::<String, _>("entity_id"),
                "action": row.get::<String, _>("action"),
                "state": row.get::<String, _>("state"),
                "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
                filter_col.as_str(): filter_val,
            });
            if socket.send(Message::Text(evt.to_string().into())).await.is_err() {
                return;
            }
        }

        ticks_since_ping += 1;
        if ticks_since_ping >= 15 {
            ticks_since_ping = 0;
            let ping = serde_json::json!({"type": "ping"});
            if socket.send(Message::Text(ping.to_string().into())).await.is_err() {
                return;
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

// ── Global SSE feed: GET /sse ──────────────────────────────────────────────────
// Streams meshprogressevent rows for all tasks/agents. Heartbeat every 30s.
// Polls every 2 seconds for rows with id > last_seen.

async fn global_sse(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);
    let db = state.db.clone();

    tokio::spawn(async move {
        let mut last_id: i32 = 0;
        let mut ticks_since_heartbeat: u32 = 0;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let rows = sqlx::query(
                "SELECT id, task_id, agent_id, seq, event_type, phase, step, summary, occurred_at \
                 FROM meshprogressevent WHERE id > $1 ORDER BY id ASC LIMIT 100"
            )
            .bind(last_id)
            .fetch_all(&db)
            .await
            .unwrap_or_default();

            for row in &rows {
                let id: i32 = row.get("id");
                if id > last_id { last_id = id; }
                let data = serde_json::json!({
                    "id": id,
                    "task_id": row.get::<String, _>("task_id"),
                    "agent_id": row.get::<String, _>("agent_id"),
                    "seq": row.get::<i32, _>("seq"),
                    "event_type": row.get::<String, _>("event_type"),
                    "phase": row.get::<Option<String>, _>("phase"),
                    "step": row.get::<Option<String>, _>("step"),
                    "summary": row.get::<String, _>("summary"),
                    "occurred_at": row.get::<chrono::NaiveDateTime, _>("occurred_at"),
                });
                let evt = Event::default()
                    .id(id.to_string())
                    .event("progress")
                    .data(data.to_string());
                if tx.send(Ok(evt)).await.is_err() {
                    return;
                }
                ticks_since_heartbeat = 0;
            }

            ticks_since_heartbeat += 1;
            if ticks_since_heartbeat >= 15 {
                // 15 ticks × 2s = 30s heartbeat
                ticks_since_heartbeat = 0;
                let ping = Event::default().event("ping").data(r#"{"type":"ping"}"#);
                if tx.send(Ok(ping)).await.is_err() {
                    return;
                }
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream).into_response()
}
