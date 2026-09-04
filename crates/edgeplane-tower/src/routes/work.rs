use axum::{
    Json, Router,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response, sse::Event, sse::Sse},
    routing::{get, patch, post},
};
use chrono::Utc;
use sqlx::Row;
use std::time::Duration;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::{auth::Principal, state::AppState};

// ── Task-available notification registry ──────────────────────────────────────
// Keyed by domain_id. Agents subscribe per-domain; senders are created on
// demand and never removed (the channel stays alive for the process lifetime,
// which is fine since we never have more domains than fit in RAM).

type NotifyRegistry = tokio::sync::Mutex<HashMap<String, broadcast::Sender<String>>>;

static NOTIFY_REGISTRY: OnceLock<NotifyRegistry> = OnceLock::new();

pub fn notify_registry() -> &'static NotifyRegistry {
    NOTIFY_REGISTRY.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

pub async fn broadcast_task_available(domain_id: &str, mission_id: &str, task_id: &str) {
    let msg = serde_json::json!({
        "type": "task_available",
        "mission_id": mission_id,
        "task_id": task_id,
    })
    .to_string();
    let reg = notify_registry().lock().await;
    if let Some(tx) = reg.get(domain_id) {
        let _ = tx.send(msg); // best-effort; no subscribers is fine
    }
}

// ── Node-keyed assignment-change notifications ────────────────────────────────
//
// Parallel to `notify_registry()` above, but keyed by `runtime_node_id`
// (the runtimenode UUID) instead of domain_id. edgeplaned daemons subscribe
// here at startup and react to add/remove/reassign by spawning, shutting
// down, or rebalancing supervisors live — no daemon restart, no yaml edit.
//
// Wire payload shapes:
//   {"type":"agent.assigned",   "agent_id":"…", "agent": {…}}
//   {"type":"agent.unassigned", "agent_id":"…"}
//   {"type":"agent.reassigned", "agent_id":"…", "agent": {…},
//                               "old_domain_id":"…", "new_domain_id":"…"}

static NODE_NOTIFY_REGISTRY: OnceLock<NotifyRegistry> = OnceLock::new();

pub fn node_notify_registry() -> &'static NotifyRegistry {
    NODE_NOTIFY_REGISTRY.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// Notify the daemon at `runtime_node_id` that one of its agents was newly
/// assigned, removed, or reassigned. Best-effort — silent when no daemon is
/// currently subscribed for that node.
pub async fn broadcast_assignment_changed(runtime_node_id: &str, payload: serde_json::Value) {
    let reg = node_notify_registry().lock().await;
    if let Some(tx) = reg.get(runtime_node_id) {
        let _ = tx.send(payload.to_string());
    }
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Tasks
        .route(
            "/work/missions/{mission_id}/tasks",
            get(list_tasks).post(create_task),
        )
        .route("/work/missions/{mission_id}/graph", get(task_graph))
        .route("/work/tasks/{task_id}", get(get_task))
        .route("/work/tasks/{task_id}/cancel", post(cancel_task))
        .route("/work/tasks/{task_id}/retry", post(retry_task))
        .route("/work/tasks/{task_id}/claim", post(claim_task))
        .route("/work/tasks/{task_id}/heartbeat", post(heartbeat_task))
        .route(
            "/work/tasks/{task_id}/progress",
            get(get_task_progress).post(append_progress),
        )
        .route("/work/tasks/{task_id}/complete", post(complete_task))
        .route("/work/tasks/{task_id}/fail", post(fail_task))
        .route("/work/tasks/{task_id}/block", post(block_task))
        .route("/work/tasks/{task_id}/unblock", post(unblock_task))
        .route("/work/tasks/{task_id}/dispatched", post(dispatch_task))
        .route(
            "/work/tasks/{task_id}/gates",
            get(list_gates).post(create_gate),
        )
        .route(
            "/work/tasks/{task_id}/gates/{gate_id}/resolve",
            post(resolve_gate),
        )
        // Domains
        .route(
            "/work/domains/{domain_id}/agents/enroll",
            post(enroll_agent),
        )
        .route("/work/domains/{domain_id}/agents", get(list_domain_agents))
        .route(
            "/work/domains/{domain_id}/messages",
            get(list_domain_messages).post(send_domain_message),
        )
        .route("/work/domains/{domain_id}/roster", get(domain_roster))
        .route("/work/domains/{domain_id}/stream", get(domain_stream))
        // Agents
        .route("/work/agents/{agent_id}/heartbeat", post(agent_heartbeat))
        .route("/work/agents/{agent_id}/status", post(set_agent_status))
        .route(
            "/work/agents/{agent_id}/profile",
            patch(update_agent_profile),
        )
        .route(
            "/work/agents/{agent_id}",
            get(get_agent).delete(delete_agent),
        )
        .route("/work/agents/{agent_id}/messages", get(get_agent_messages))
        .route("/work/agents/{agent_id}/notify", get(agent_notify_ws))
        .route("/work/agents/{agent_id}/token", post(mint_agent_token))
        // Mission messages + stream
        .route(
            "/work/missions/{mission_id}/messages",
            get(list_mission_messages).post(send_mission_message),
        )
        .route("/work/missions/{mission_id}/stream", get(mission_stream))
        // Global SSE feed — TUI agent feed; polls meshprogressevent for all agents
        .route("/sse", get(global_sse))
}

// ── Error helpers ──────────────────────────────────────────────────────────────

pub(crate) fn not_found(msg: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"detail": msg})),
    )
        .into_response()
}
pub(crate) fn conflict(msg: &str) -> axum::response::Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({"detail": msg})),
    )
        .into_response()
}
pub(crate) fn bad_request(msg: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"detail": msg})),
    )
        .into_response()
}

// ── Row helpers ────────────────────────────────────────────────────────────────

pub(crate) fn row_to_task(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "mission_id": row.get::<String, _>("mission_id"),
        "domain_id": row.get::<String, _>("domain_id"),
        "parent_task_id": row.get::<Option<String>, _>("parent_task_id"),
        // kind discriminator ('assigned' | 'claimable') from migration 0014's
        // task/meshtask unification — this route family (`/work/...`) only
        // ever writes/lists kind='claimable' rows, but row_to_task is also
        // used to render single-row fetches (get_task), so surface it rather
        // than assume.
        "kind": row.get::<String, _>("kind"),
        "title": row.get::<String, _>("title"),
        "description": row.try_get::<Option<String>, _>("description").ok().flatten().unwrap_or_default(),
        // claim_policy is claimable-only and nullable post-unification (NULL
        // for kind='assigned' rows) — was a non-Option `row.get::<String,_>`
        // that would panic decoding a NULL from an assigned row.
        "claim_policy": row.get::<Option<String>, _>("claim_policy"),
        "depends_on": serde_json::from_str::<serde_json::Value>(&row.try_get::<Option<String>, _>("depends_on").ok().flatten().unwrap_or_default()).unwrap_or(serde_json::json!([])),
        "produces": serde_json::from_str::<serde_json::Value>(&row.try_get::<Option<String>, _>("produces").ok().flatten().unwrap_or_default()).unwrap_or(serde_json::json!({})),
        "consumes": serde_json::from_str::<serde_json::Value>(&row.try_get::<Option<String>, _>("consumes").ok().flatten().unwrap_or_default()).unwrap_or(serde_json::json!({})),
        "required_capabilities": serde_json::from_str::<serde_json::Value>(&row.try_get::<Option<String>, _>("required_capabilities").ok().flatten().unwrap_or_default()).unwrap_or(serde_json::json!([])),
        "status": row.get::<String, _>("status"),
        "claimed_by_agent_id": row.get::<Option<String>, _>("claimed_by_agent_id"),
        // result_artifact_id is now `integer` (matches artifact.id) — was varchar.
        "result_artifact_id": row.get::<Option<i32>, _>("result_artifact_id"),
        "priority": row.get::<i32, _>("priority"),
        "lease_expires_at": row.get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at").map(|dt| dt.and_utc()),
        "attempt": row.get::<i16, _>("attempt"),
        "max_attempts": row.get::<i16, _>("max_attempts"),
        "finalized_at": row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("finalized_at").ok().flatten(),
        "finalized_by_subject": row.try_get::<Option<String>, _>("finalized_by_subject").ok().flatten(),
        "created_by_subject": row.get::<String, _>("created_by_subject"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at").and_utc(),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at").and_utc(),
    })
}

pub fn row_to_agent(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let profile: Option<serde_json::Value> = row
        .get::<Option<&str>, _>("profile_json")
        .and_then(|s| serde_json::from_str(s).ok());
    let machine: Option<serde_json::Value> = row
        .get::<Option<&str>, _>("machine_json")
        .and_then(|s| serde_json::from_str(s).ok());
    let runtime: Option<serde_json::Value> = row
        .get::<Option<&str>, _>("runtime_json")
        .and_then(|s| serde_json::from_str(s).ok());
    let discovered_capabilities: serde_json::Value = row
        .try_get::<Option<&str>, _>("discovered_capabilities")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::json!([]));
    let meshagent_id: String = row.get::<String, _>("id");
    // `public_id` is the wire identifier edgeplaned uses for the poll loop
    // (`/agents/{public_id}/messages`) and that the edgeplane CLI passes via
    // `--to-agent-id`. Prefer the linked `agent.public_id` when this
    // meshagent points at a persistent agent identity (Step 5 of the
    // public_id plan); fall back to the meshagent's own UUID when no
    // linkage was supplied at enrollment so older rows keep working.
    let agent_public_id: Option<String> = row
        .try_get::<Option<String>, _>("agent_public_id")
        .ok()
        .flatten();
    let public_id = agent_public_id
        .clone()
        .unwrap_or_else(|| meshagent_id.clone());
    serde_json::json!({
        "id": &meshagent_id,
        "public_id": public_id,
        "agent_public_id": agent_public_id,
        "domain_id": row.get::<String, _>("domain_id"),
        "node_id": row.get::<Option<String>, _>("node_id"),
        "runtime_kind": row.get::<String, _>("runtime_kind"),
        "runtime_version": row.get::<String, _>("runtime_version"),
        "capabilities": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("capabilities")).unwrap_or(serde_json::json!([])),
        "labels": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("labels")).unwrap_or(serde_json::json!({})),
        "status": row.get::<String, _>("status"),
        "current_task_id": row.get::<Option<String>, _>("current_task_id"),
        "enrolled_at": row.get::<chrono::NaiveDateTime, _>("enrolled_at"),
        "last_heartbeat_at": row.get::<Option<chrono::NaiveDateTime>, _>("last_heartbeat_at"),
        // Daemon-driving fields — required by edgeplaned's controlplane-driven
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
    // meshmessage.id and .in_reply_to are `integer` (i32) in Postgres, and
    // body_json is nullable `text` — decoding as i64/non-Option panics via
    // `Row::get` (sqlx's `try_get().unwrap()`), crashing the request with an
    // empty reply on any row where these are exercised.
    let body_json: serde_json::Value = row
        .try_get::<Option<String>, _>("body_json")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "domain_id": row.get::<String, _>("domain_id"),
        "mission_id": row.get::<Option<String>, _>("mission_id"),
        "from_agent_id": row.get::<String, _>("from_agent_id"),
        "to_agent_id": row.get::<Option<String>, _>("to_agent_id"),
        "task_id": row.get::<Option<String>, _>("task_id"),
        "channel": row.get::<String, _>("channel"),
        "body_json": body_json,
        "in_reply_to": row.get::<Option<i32>, _>("in_reply_to"),
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
fn default_input_json() -> String {
    "{}".to_string()
}
fn default_first_claim() -> String {
    "first_claim".to_string()
}

#[derive(serde::Deserialize, Default)]
struct HeartbeatBody {
    claim_lease_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct CompleteBody {
    result_artifact_id: Option<String>,
    claim_lease_id: Option<String>,
    /// On-behalf-of ephemeral agent id — only honored for full-trust/admin
    /// callers (see complete_task's effective_id derivation), mirroring
    /// claim_task's own on-behalf-of write (work.rs:1055-1068). This is the
    /// real edgeplaned-bin/task_worker.rs call shape: it authenticates with
    /// its node's full-trust credential, never a per-agent token, and always
    /// sends `{"agent_id": ...}`.
    agent_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct FailBody {
    #[serde(default)]
    #[allow(dead_code)]
    error: String,
    claim_lease_id: Option<String>,
    /// See CompleteBody::agent_id.
    agent_id: Option<String>,
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
    claim_lease_id: String,
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
fn default_coordination() -> String {
    "coordination".to_string()
}
fn default_empty_obj() -> String {
    "{}".to_string()
}

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
    /// enrollment represents (e.g. `my-agent-work`). When set, the controlplane
    /// upserts the matching `agent` row and stores its `public_id` on this
    /// meshagent. edgeplaned then receives the public_id as the wire identifier
    /// and uses it to poll `/agents/{public_id}/messages`. See
    /// `docs/plans/2026-05-11-agent-public-id-edgeplaned-fix.md`.
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
fn default_human() -> String {
    "human".to_string()
}

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
fn default_neg_one() -> i32 {
    -1
}

#[derive(serde::Deserialize)]
struct AgentMessagesQuery {
    #[serde(default)]
    since_id: i64,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

pub(crate) const LEASE_TTL_SECS: i64 = 120;

/// Expire stale leases for a mission before listing tasks.
///
/// Two behavior changes from the pre-unification version (migration 0014):
///
/// 1. **Fencing fix (real bug, not speculative):** the old UPDATE cleared
///    `claimed_by_agent_id`/`lease_expires_at` on expiry but left the stale
///    `claim_lease_id` in place. A slow-but-alive agent A whose lease expired
///    and got reclaimed by agent B could still present A's old lease id to
///    `complete_task`/`fail_task`/etc. This was only masked by
///    `heartbeat_task`'s incidental `status != claimed/running` check, not by
///    intentional fencing. Now `claim_lease_id=NULL` is cleared here too, so a
///    stale token can never validate against a reclaimed (or re-readied) row.
/// 2. **Bounded retry (new — `attempt`/`max_attempts`, migration 0014):**
///    previously every expiry unconditionally reset the row to `status='ready'`
///    (infinitely reclaimable). Now `attempt` increments on every expiry, and
///    once `attempt >= max_attempts` the row goes to `status='failed'`
///    (`finalized_at` stamped) instead of back to `ready`. Default
///    `max_attempts=1` (the migration's column default) reproduces today's
///    exact single-shot-then-failed behavior is NOT preserved as-is: previously
///    a timed-out task looped back to `ready` forever; now, with the default
///    max_attempts=1, the FIRST expiry (attempt 0->1, 1>=1) already fails it
///    instead of re-readying it. This is an intentional behavior change (the
///    plan's point — bounded retry replaces unbounded silent reclaim) and is
///    called out explicitly in this PR's report, not shipped silently.
async fn expire_stale_leases(db: &sqlx::PgPool, mission_id: &str) {
    let now = Utc::now().naive_utc();
    let now_tz = Utc::now();
    let _ = sqlx::query(
        "UPDATE task SET \
           attempt = attempt + 1, \
           status = CASE WHEN attempt + 1 >= max_attempts THEN 'failed' ELSE 'ready' END, \
           finalized_at = CASE WHEN attempt + 1 >= max_attempts THEN $3 ELSE finalized_at END, \
           claimed_by_agent_id = NULL, lease_expires_at = NULL, claim_lease_id = NULL, \
           updated_at = $1 \
         WHERE mission_id=$2 AND kind='claimable' AND status IN ('claimed','running') \
           AND claim_policy != 'broadcast' \
           AND lease_expires_at IS NOT NULL AND lease_expires_at < $1",
    )
    .bind(now)
    .bind(mission_id)
    .bind(now_tz)
    .execute(db)
    .await;
}

/// DFS cycle detection when adding a new task with dependencies. `pub(crate)`
/// so `routes::mcp::submit_mesh_task` can reuse it — the migration 0014
/// column-population unification (REST `create_task` vs. MCP
/// `submit_mesh_task`) now has the MCP path accept `depends_on` too, and it
/// needs the same cycle guard `create_task` already had.
pub(crate) async fn detect_cycle(
    db: &sqlx::PgPool,
    mission_id: &str,
    new_id: &str,
    depends_on: &[String],
) -> Result<bool, sqlx::Error> {
    let rows =
        sqlx::query("SELECT id, depends_on FROM task WHERE mission_id=$1 AND kind='claimable'")
            .bind(mission_id)
            .fetch_all(db)
            .await?;
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for r in &rows {
        let id: String = r.get("id");
        // depends_on is nullable `text` (no NOT NULL) — non-Option `Row::get`
        // panics on NULL.
        let deps: Vec<String> = serde_json::from_str(
            r.try_get::<Option<&str>, _>("depends_on")
                .ok()
                .flatten()
                .unwrap_or("[]"),
        )
        .unwrap_or_default();
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
pub(crate) async fn unblock_dependents(
    db: &sqlx::PgPool,
    mission_id: &str,
    finished_id: &str,
) -> Vec<String> {
    let candidates = sqlx::query(
        "SELECT id, depends_on FROM task WHERE mission_id=$1 AND kind='claimable' AND status IN ('pending','blocked')",
    )
    .bind(mission_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let now = Utc::now().naive_utc();
    let mut ready_ids = Vec::new();

    for c in &candidates {
        let cid: String = c.get("id");
        let dep_ids: Vec<String> = serde_json::from_str(
            c.try_get::<Option<&str>, _>("depends_on")
                .ok()
                .flatten()
                .unwrap_or("[]"),
        )
        .unwrap_or_default();
        if !dep_ids.contains(&finished_id.to_string()) {
            continue;
        }
        // Check all deps are finished
        let dep_rows = sqlx::query("SELECT status FROM task WHERE id = ANY($1)")
            .bind(dep_ids.as_slice())
            .fetch_all(db)
            .await
            .unwrap_or_default();
        if dep_rows.len() == dep_ids.len()
            && dep_rows
                .iter()
                .all(|r| r.get::<String, _>("status") == "finished")
        {
            let _ = sqlx::query("UPDATE task SET status='ready', updated_at=$2 WHERE id=$1")
                .bind(&cid)
                .bind(now)
                .execute(db)
                .await;
            ready_ids.push(cid);
        }
    }
    ready_ids
}

/// Generate a `public_id` matching the convention used across the unified
/// `task` table (migration 0014: `'task-' || substr(replace(gen_random_uuid()
/// ::text,'-',''),1,8)`, and `routes/agents.rs::generate_public_id`'s pattern):
/// `task-{8 hex chars}`.
pub(crate) fn new_public_id() -> String {
    let hex = Uuid::new_v4().simple().to_string();
    format!("task-{}", &hex[..8])
}

/// Compute a new claimable task's initial status: `ready` if it has no
/// dependencies (or all of them are already `finished`), else `pending`.
/// `pub(crate)` — shared between `create_task` (REST) and
/// `routes::mcp::submit_mesh_task` (MCP) so the two column-population paths
/// (migration 0014's unification requirement) don't diverge on this logic.
pub(crate) async fn compute_initial_status(
    db: &sqlx::PgPool,
    depends_on: &[String],
) -> &'static str {
    if depends_on.is_empty() {
        return "ready";
    }
    let dep_rows = sqlx::query("SELECT status FROM task WHERE id = ANY($1)")
        .bind(depends_on)
        .fetch_all(db)
        .await
        .unwrap_or_default();
    if dep_rows.len() == depends_on.len()
        && dep_rows
            .iter()
            .all(|r| r.get::<String, _>("status") == "finished")
    {
        "ready"
    } else {
        "pending"
    }
}

// ── Task handlers ──────────────────────────────────────────────────────────────

async fn list_tasks(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Query(q): Query<TaskListQuery>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_by_mission(&state.db, &principal, &mission_id).await
    {
        return r;
    }
    expire_stale_leases(&state.db, &mission_id).await;

    // kind='claimable' filter: this route (`/work/missions/{id}/tasks`) is the
    // mesh/claimable-pool listing API — historically it only ever saw meshtask
    // rows. Post-unification the table also holds kind='assigned' rows (PM-style
    // status vocabulary, no claim_policy/lease semantics); without this filter
    // they'd leak into what daemon pollers (poll_ready_tasks) treat as claimable
    // work.
    let rows = if let Some(status) = &q.status {
        sqlx::query(
            "SELECT * FROM task WHERE mission_id=$1 AND kind='claimable' AND status=$2 ORDER BY priority DESC, created_at ASC",
        )
        .bind(&mission_id)
        .bind(status)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query(
            "SELECT * FROM task WHERE mission_id=$1 AND kind='claimable' ORDER BY priority DESC, created_at ASC",
        )
        .bind(&mission_id)
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
    Path(mission_id): Path<String>,
    Json(body): Json<TaskCreate>,
) -> impl IntoResponse {
    // Resolve domain_id from mission
    let mission_row = sqlx::query("SELECT id, domain_id FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await;
    let domain_id = match mission_row {
        Ok(Some(r)) => r.get::<Option<String>, _>("domain_id").unwrap_or_default(),
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("create_task fetch mission: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    // Validate depends_on tasks exist (claimable-only: dependency graph is a
    // claimable-pool concept, and depends_on/produces/consumes are
    // claimable-only columns post-unification).
    for dep_id in &body.depends_on {
        let exists: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM task WHERE id=$1 AND mission_id=$2 AND kind='claimable'",
        )
        .bind(dep_id)
        .bind(&mission_id)
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
        match detect_cycle(&state.db, &mission_id, &new_id, &body.depends_on).await {
            Ok(true) => return bad_request("Dependency cycle detected"),
            Ok(false) => {}
            Err(e) => {
                tracing::error!("detect_cycle: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }

    // Determine initial status: pending if has unfinished deps, else ready
    let initial_status = compute_initial_status(&state.db, &body.depends_on).await;

    let now = Utc::now().naive_utc();
    let depends_on_json =
        serde_json::to_string(&body.depends_on).unwrap_or_else(|_| "[]".to_string());
    let produces_json = serde_json::to_string(&body.produces).unwrap_or_else(|_| "{}".to_string());
    let consumes_json = serde_json::to_string(&body.consumes).unwrap_or_else(|_| "{}".to_string());
    let req_caps_json =
        serde_json::to_string(&body.required_capabilities).unwrap_or_else(|_| "[]".to_string());
    let public_id = new_public_id();

    let row = sqlx::query(
        "INSERT INTO task (id, public_id, mission_id, domain_id, parent_task_id, kind, title, description, \
         input_json, claim_policy, depends_on, produces, consumes, required_capabilities, \
         status, claimed_by_agent_id, result_artifact_id, priority, \
         lease_expires_at, claim_lease_id, version_counter, \
         created_by_subject, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,'claimable',$6,$7,$8,$9,$10,$11,$12,$13,$14,NULL,NULL,$15,NULL,NULL,0,$16,$17,$17) \
         RETURNING *",
    )
    .bind(&new_id)
    .bind(&public_id)
    .bind(&mission_id)
    .bind(&domain_id)
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
                broadcast_task_available(&domain_id, &mission_id, &tid).await;
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
    principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_by_mission(&state.db, &principal, &mission_id).await
    {
        return r;
    }
    let rows = sqlx::query(
        "SELECT id, title, status, depends_on FROM task WHERE mission_id=$1 AND kind='claimable'",
    )
    .bind(&mission_id)
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
                let deps: Vec<String> = serde_json::from_str(
                    r.try_get::<Option<&str>, _>("depends_on")
                        .ok()
                        .flatten()
                        .unwrap_or("[]"),
                )
                .unwrap_or_default();
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
    principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_by_task(&state.db, &principal, &task_id).await {
        return r;
    }
    match sqlx::query("SELECT * FROM task WHERE id=$1")
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
    principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let subject_id = principal
        .subject
        .strip_prefix("agent:")
        .unwrap_or(&principal.subject);
    let now = Utc::now().naive_utc();
    let now_tz = Utc::now();

    // Fenced CAS, same pattern as complete_task/fail_task — closes the
    // TOCTOU window the prior authz_task_owner-then-blind-UPDATE left open.
    // No broadcast carve-out needed: unlike the lease/freshness endpoints,
    // this predicate never checks lease_expires_at at all, so there's
    // nothing broadcast would need to bypass (the dual-review CRITICAL
    // finding on the other three endpoints doesn't recur here structurally
    // — confirmed by inspection, not by omission). No on-behalf-of
    // (effective_id) support either: the one real caller (`edgeplane daemon
    // task cancel`, a full-trust CLI session) sends no body at all, so
    // `subject_id` alone matches the plan; add on-behalf-of only if a real
    // caller ever needs it (YAGNI, not because there's a shape it can't yet
    // handle).
    //
    // finalized_at/finalized_by_subject: cancel is a terminal transition
    // like complete/fail, so it stamps both (the pre-existing code stamped
    // neither — closing that gap here, not just adding the new column).
    // Deliberately NOT `COALESCE(claimed_by_agent_id, $3)` like complete/
    // fail — an adversarial review (2026-08-20) caught that cancel's real
    // caller is an operator interrupting someone ELSE's live task, not a
    // claimer self-reporting its own work, so recording the *claimer*
    // would attribute the cancellation to its victim. `COALESCE(finalized_
    // by_subject, $3)` records the canceller for a live cancel, while still
    // preserving an already-recorded attribution rather than clobbering it
    // (e.g. cancelling an already-`failed` task — legal, since only
    // `finished`/`cancelled` are excluded — must not overwrite the agent
    // that actually failed it with the operator who cleaned it up after).
    let updated = sqlx::query(
        "UPDATE task SET status='cancelled', claimed_by_agent_id=NULL, \
         lease_expires_at=NULL, claim_lease_id=NULL, updated_at=$2, \
         finalized_at=$5, finalized_by_subject=COALESCE(finalized_by_subject, $3) \
         WHERE id=$1 \
           AND ( \
             (kind = 'claimable' AND status NOT IN ('finished','cancelled') \
              AND (claimed_by_agent_id = $3 OR $4)) \
             OR \
             (kind = 'assigned' AND status NOT IN ('done','finished','failed','cancelled') \
              AND (owner = $3 OR $4)) \
           ) \
         RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .bind(subject_id)
    .bind(is_bypass)
    .bind(now_tz)
    .fetch_optional(&state.db)
    .await;

    match updated {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => {
            let actor = crate::routes::task_transitions::task_actor(&principal);
            crate::routes::task_transitions::rest_transition_error(
                crate::routes::task_transitions::classify_fenced_rejection(
                    &state.db,
                    &actor,
                    &task_id,
                    None,
                    &["cancelled"],
                )
                .await,
            )
        }
        Err(e) => {
            tracing::error!("cancel_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn retry_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await
    {
        return resp;
    }

    let now = Utc::now().naive_utc();
    // Fenced CAS: preconditions (kind + status) are part of the WHERE clause,
    // not separate app-level checks. kind='claimable' ensures only tasks in
    // the claim pool can be retried; status IN ('failed','cancelled') are the
    // only eligible terminal states for retry entry back to 'ready'.
    let updated = sqlx::query(
        "UPDATE task SET status='ready', claimed_by_agent_id=NULL, result_artifact_id=NULL, \
         lease_expires_at=NULL, claim_lease_id=NULL, finalized_at=NULL, \
         finalized_by_subject=NULL, updated_at=$2 \
         WHERE id=$1 AND kind='claimable' AND status IN ('failed','cancelled') \
         RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .fetch_optional(&state.db)
    .await;

    match updated {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => classify_retry_rejection(&state.db, &task_id).await,
        Err(e) => {
            tracing::error!("retry_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// After the fenced retry UPDATE rejects (zero rows), classify why with a
/// fresh read — preserves retry_task's existing precondition-specific error
/// messages (kind vs. status) while ensuring the message reflects the
/// task's CURRENT state, not the pre-UPDATE snapshot the old check-then-act
/// code read (which could already be stale by the time the blind UPDATE
/// ran). No ownership/lease dimension here: retry_task performs no
/// per-caller ownership check by design (any domain member may retry a
/// failed/cancelled task — ruled by Merlin 2026-08-26, see
/// docs/superpowers/plans/2026-08-18-ep1-tower-fencing.md Roadmap,
/// "retry_task — severity correction"). This function only closes the
/// TOCTOU on the status/kind precondition, it does not add authorization.
async fn classify_retry_rejection(db: &sqlx::PgPool, task_id: &str) -> axum::response::Response {
    let row = match sqlx::query("SELECT kind, status FROM task WHERE id=$1")
        .bind(task_id)
        .fetch_optional(db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("classify_retry_rejection fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let kind: String = row.get("kind");
    if kind != "claimable" {
        return conflict("Task is not claimable (kind='assigned'); retry does not apply");
    }
    let status: String = row.get("status");
    conflict(&format!("Task cannot be retried from status: {status}"))
}

async fn claim_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    // Full-trust callers (session / node) and admins may supply an explicit
    // agent_id in the body to claim on behalf of another agent. Restricted
    // callers (agents, service accounts) are always attributed to themselves —
    // the body field is ignored so a compromised agent cannot spoof another's id.
    let self_id = principal
        .subject
        .strip_prefix("agent:")
        .unwrap_or(&principal.subject);
    let agent_id = if crate::auth::is_full_trust(&principal) || principal.is_admin {
        body.as_ref()
            .and_then(|b| b.get("agent_id"))
            .and_then(|v| v.as_str())
            .unwrap_or(self_id)
            .to_string()
    } else {
        self_id.to_string()
    };

    // First fetch the task to check claim_policy
    let task_row = match sqlx::query("SELECT * FROM task WHERE id=$1")
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

    let domain_id: String = task_row.get("domain_id");
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    // Kind-gating: an assigned task is never claimable, full stop.
    let kind: String = task_row.get("kind");
    if kind != "claimable" {
        return conflict("Task is not claimable (kind='assigned')");
    }

    let claim_policy: Option<String> = task_row.get("claim_policy");
    let claim_policy = claim_policy.unwrap_or_default();
    let status: String = task_row.get("status");

    if status != "ready" {
        return (
            StatusCode::LOCKED,
            Json(serde_json::json!({"detail": "Task not available for claiming"})),
        )
            .into_response();
    }

    let now = Utc::now().naive_utc();
    let lease_expires = now + chrono::Duration::seconds(LEASE_TTL_SECS);
    let lease_id = Uuid::new_v4().to_string();

    // Broadcast: no locking needed, just update status to running.
    // Intentionally unfenced — broadcast tasks are meant to be claimable by
    // multiple agents simultaneously, so there is no single "owner" for a
    // CAS to protect. This is a deliberate, stated exception to the fencing
    // pattern the rest of this file converges on, not an oversight. See
    // spec §1 "Broadcast claims and full-trust/admin bypass".
    if claim_policy == "broadcast" {
        let row = sqlx::query(
            "UPDATE task SET status='running', claimed_by_agent_id=$2, \
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

    let locked =
        sqlx::query("SELECT * FROM task WHERE id=$1 AND status='ready' FOR UPDATE SKIP LOCKED")
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
        "UPDATE task SET status='claimed', claimed_by_agent_id=$2, claim_lease_id=$3, \
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
    principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<HeartbeatBody>>,
) -> impl IntoResponse {
    let body = body.map(|b| b.0).unwrap_or_default();

    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::Heartbeat {
            claim_lease_id: body.claim_lease_id.as_deref(),
        },
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Task { task, .. }) => {
            Json(task).into_response()
        }
        // Exhaustive, not a wildcard: if `execute_task_transition` ever
        // grows a new variant, this arm must fail to compile until someone
        // decides what it means for Heartbeat, instead of silently panicking
        // at runtime the way `Ok(_) => unreachable!()` would.
        Ok(crate::routes::task_transitions::TransitionOutcome::Progress(_))
        | Ok(crate::routes::task_transitions::TransitionOutcome::WaitingReview { .. }) => {
            unreachable!("Heartbeat always yields TransitionOutcome::Task")
        }
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}

async fn append_progress(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    Json(body): Json<ProgressCreate>,
) -> impl IntoResponse {
    if body.claim_lease_id.is_empty() {
        return bad_request("claim_lease_id is required");
    }

    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::AppendProgress {
            claim_lease_id: &body.claim_lease_id,
            event_type: &body.event_type,
            phase: body.phase.as_deref(),
            step: body.step.as_deref(),
            summary: &body.summary,
            payload_json: &body.payload_json,
            agent_run_id: body.agent_run_id.as_deref(),
        },
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Progress(event)) => {
            Json(event).into_response()
        }
        // Exhaustive, not a wildcard — see the matching comment on
        // heartbeat_task's arm above.
        Ok(crate::routes::task_transitions::TransitionOutcome::Task { .. })
        | Ok(crate::routes::task_transitions::TransitionOutcome::WaitingReview { .. }) => {
            unreachable!("AppendProgress always yields TransitionOutcome::Progress")
        }
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}

async fn complete_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<CompleteBody>>,
) -> impl IntoResponse {
    let body = body.map(|b| b.0).unwrap_or_default();

    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let result_artifact_id: Option<i32> = body
        .result_artifact_id
        .as_deref()
        .and_then(|s| s.parse::<i32>().ok());

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::Complete {
            claim_lease_id: body.claim_lease_id.as_deref(),
            agent_id: body.agent_id.as_deref(),
            result_artifact_id,
        },
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Task {
            task,
            unblocked_task_ids,
        }) => {
            let mut val = task;
            val["unblocked_tasks"] = serde_json::json!(unblocked_task_ids);
            Json(val).into_response()
        }
        Ok(crate::routes::task_transitions::TransitionOutcome::WaitingReview {
            pending_gate_ids,
            ..
        }) => Json(serde_json::json!({
            "status": "waiting_review",
            "pending_gates": pending_gate_ids,
            "task_id": task_id,
        }))
        .into_response(),
        // Exhaustive, not a wildcard — see the matching comment on
        // heartbeat_task's arm above.
        Ok(crate::routes::task_transitions::TransitionOutcome::Progress(_)) => {
            unreachable!("Complete only yields Task or WaitingReview")
        }
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}

async fn fail_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<FailBody>>,
) -> impl IntoResponse {
    let body = body.map(|b| b.0).unwrap_or_default();

    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::Fail {
            claim_lease_id: body.claim_lease_id.as_deref(),
            agent_id: body.agent_id.as_deref(),
        },
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Task { task, .. }) => {
            Json(task).into_response()
        }
        // Exhaustive, not a wildcard — see the matching comment on
        // heartbeat_task's arm above.
        Ok(crate::routes::task_transitions::TransitionOutcome::Progress(_))
        | Ok(crate::routes::task_transitions::TransitionOutcome::WaitingReview { .. }) => {
            unreachable!("Fail always yields TransitionOutcome::Task")
        }
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}

async fn block_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::Block {
            claim_lease_id: None,
        },
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Task { task, .. }) => {
            Json(task).into_response()
        }
        // Exhaustive, not a wildcard — see the matching comment on
        // heartbeat_task's arm above.
        Ok(crate::routes::task_transitions::TransitionOutcome::Progress(_))
        | Ok(crate::routes::task_transitions::TransitionOutcome::WaitingReview { .. }) => {
            unreachable!("Block always yields TransitionOutcome::Task")
        }
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}

/// Mark a `ready` task as dispatched — terminal status `finished`, no claim
/// needed. Exists specifically for the triage routing pattern: the triage
/// layer creates a child meshtask under the routed mission (which carries
/// the work), and the intake task itself needs to transition to terminal
/// without the claim-then-complete dance that `complete_task` requires.
///
/// Authorization: admin OR the task's `created_by_subject`. (Different from
/// `complete_task`, which is for the agent that claimed the task — here the
/// caller is the triage layer, not a task executor.)
///
/// Idempotent on already-finished tasks: returns the existing row.
async fn dispatch_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query("SELECT status, created_by_subject, domain_id FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_optional(&state.db)
        .await;
    let (status, created_by, domain_id): (String, String, String) = match row {
        Ok(Some(r)) => (
            r.get("status"),
            r.get("created_by_subject"),
            r.get("domain_id"),
        ),
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("dispatch_task lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    if created_by != principal.subject && !principal.is_admin {
        return StatusCode::FORBIDDEN.into_response();
    }
    if status == "finished" {
        // Idempotent: re-fetch and return.
        let r = sqlx::query("SELECT * FROM task WHERE id=$1")
            .bind(&task_id)
            .fetch_one(&state.db)
            .await;
        return match r {
            Ok(row) => Json(row_to_task(&row)).into_response(),
            Err(e) => {
                tracing::error!("dispatch_task re-fetch: {e}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        };
    }
    if status != "ready" {
        return bad_request(&format!(
            "dispatch_task requires status='ready' (got '{status}'); use complete_task for claimed/running tasks"
        ));
    }

    let now = Utc::now().naive_utc();
    match sqlx::query("UPDATE task SET status='finished', updated_at=$2 WHERE id=$1 RETURNING *")
        .bind(&task_id)
        .bind(now)
        .fetch_one(&state.db)
        .await
    {
        Ok(r) => Json(row_to_task(&r)).into_response(),
        Err(e) => {
            tracing::error!("dispatch_task: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn unblock_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let subject_id = principal
        .subject
        .strip_prefix("agent:")
        .unwrap_or(&principal.subject);
    let now = Utc::now().naive_utc();

    // Fenced CAS — was a blind UPDATE with zero status precondition (any
    // domain member with ownership could unblock a task in ANY status, not
    // just 'blocked'). kind='claimable' only, same rationale as Task 5:
    // 'ready' is claimable-pool-only vocabulary (retry_task precedent,
    // work.rs ~974). Ownership: claimed_by_agent_id = $3 OR $4, no lease
    // path — this endpoint has never accepted a lease param, matching
    // block_task's symmetric predicate.
    //
    // Deliberately does NOT clear claimed_by_agent_id, unlike retry_task's
    // failed/cancelled->ready transition (which does). Traced this before
    // implementing rather than pattern-matching retry_task's precedent by
    // analogy (exactly the failure mode Tasks 4/5's bugs came from):
    // claim_task's non-broadcast path never consults claimed_by_agent_id
    // at all (only checks status='ready', fences via version_counter), so
    // clearing it buys no protection against a different agent claiming
    // the row the instant it's ready again — that race exists identically
    // either way. The two transitions aren't actually analogous:
    // retry_task's source states (failed/cancelled) are a deliberate fresh
    // start for anyone; unblock's predicate (only the original blocker or
    // a bypass caller may call it) encodes a "resume your own paused work"
    // semantic instead, where preserving the attribution is correct, not
    // stale. already_done_statuses is &[] (not &["ready"]): 'ready' isn't
    // exclusively produced by this endpoint (claim/retry/creation all land
    // there too), so treating "row is ready" as "already unblocked" would
    // misclassify a task that was never blocked in the first place; the
    // real criterion (does this transition destroy ownership evidence) is
    // satisfied some other way here — it doesn't destroy any, so a retry
    // resolves correctly via owns_directly alone.
    let updated = sqlx::query(
        "UPDATE task SET status='ready', updated_at=$2 \
         WHERE id=$1 AND kind='claimable' AND status='blocked' \
           AND (claimed_by_agent_id = $3 OR $4) \
         RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .bind(subject_id)
    .bind(is_bypass)
    .fetch_optional(&state.db)
    .await;

    match updated {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => {
            let actor = crate::routes::task_transitions::task_actor(&principal);
            crate::routes::task_transitions::rest_transition_error(
                crate::routes::task_transitions::classify_fenced_rejection(
                    &state.db,
                    &actor,
                    &task_id,
                    None,
                    &[],
                )
                .await,
            )
        }
        Err(e) => {
            tracing::error!("unblock_task: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_task_progress(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    Query(q): Query<ProgressQuery>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_by_task(&state.db, &principal, &task_id).await {
        return r;
    }
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM task WHERE id=$1")
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
                    // id is `integer` (i32) in Postgres — decoding as i64 panics via
                    // `Row::get` (sqlx's `try_get().unwrap()`), crashing the whole request
                    // with an empty reply. summary/payload_json are nullable `text` but were
                    // decoded as non-Option String/&str, same panic risk on a NULL row.
                    let payload_json = r
                        .try_get::<Option<String>, _>("payload_json")
                        .ok()
                        .flatten()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                        .unwrap_or(serde_json::json!({}));
                    serde_json::json!({
                        "id": r.get::<i32, _>("id"),
                        "task_id": r.get::<String, _>("task_id"),
                        "agent_id": r.get::<String, _>("agent_id"),
                        "seq": r.get::<i32, _>("seq"),
                        "event_type": r.get::<String, _>("event_type"),
                        "phase": r.get::<Option<String>, _>("phase"),
                        "step": r.get::<Option<String>, _>("step"),
                        "summary": r.try_get::<Option<String>, _>("summary").ok().flatten(),
                        "payload_json": payload_json,
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
    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let subject_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
    let gate_id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    // Fences the ownership check (Change 10: only the task's claimer, or
    // full-trust/admin, may attach a gate) AND a "still gate-attachable"
    // status check into the INSERT itself, closing the check-then-insert
    // TOCTOU the old separate authz_task_owner precheck left open: a caller
    // who owned the task at check-time but has since lost ownership
    // (reclaimed, completed, cancelled) could otherwise still attach a
    // pending gate. "Gate-attachable" mirrors complete_task's own
    // non-terminal predicate (task_transitions.rs's Complete arm) — a gate
    // only makes sense before the task reaches a status complete_task
    // itself would treat as terminal. See
    // docs/superpowers/plans/2026-08-18-ep1-tower-fencing.md Roadmap,
    // "create_gate is check-then-insert with no fencing on the insert
    // itself".
    let row = sqlx::query(
        "INSERT INTO reviewgate (id, owner_subject, mesh_task_id, run_id, gate_type, \
         required_approvals, status, approval_request_id, ai_pending_action_id, policy_rule_id, \
         created_at, resolved_at) \
         SELECT $1,$2,$3,$4,$5,$6,'pending',$7,NULL,NULL,$8,NULL \
         WHERE EXISTS ( \
           SELECT 1 FROM task \
           WHERE task.id = $3 \
             AND ( \
               (task.kind = 'claimable' AND task.status IN ('claimed','running','waiting_review') \
                AND (task.claimed_by_agent_id = $9 OR $10)) \
               OR \
               (task.kind = 'assigned' AND task.status NOT IN ('done','finished','failed','cancelled') \
                AND (task.owner = $9 OR $10)) \
             ) \
         ) \
         RETURNING *",
    )
    .bind(&gate_id)
    .bind(&principal.subject)
    .bind(&task_id)
    .bind(&body.run_id)
    .bind(&body.gate_type)
    .bind(&body.required_approvals)
    .bind(&body.approval_request_id)
    .bind(now)
    .bind(subject_id)
    .bind(is_bypass)
    .fetch_optional(&state.db)
    .await;

    match row {
        Ok(Some(r)) => (StatusCode::CREATED, Json(row_to_gate(&r))).into_response(),
        Ok(None) => classify_create_gate_rejection(&state.db, &principal, &task_id).await,
        Err(e) => {
            tracing::error!("create_gate: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// After the fenced INSERT rejects (zero rows), classify why with a fresh
/// read: reuses `authz_task_owner` unchanged for "task missing" (404) /
/// "caller isn't the claimer" (403) — those two conditions are exactly what
/// it already checks. A caller who reaches here AND passes
/// `authz_task_owner` must have failed the status half of the fence (the
/// only remaining reason the INSERT's WHERE EXISTS could be false), i.e.
/// the task is no longer in a gate-attachable status.
async fn classify_create_gate_rejection(
    db: &sqlx::PgPool,
    principal: &Principal,
    task_id: &str,
) -> axum::response::Response {
    if let Err(resp) = crate::routes::authz::authz_task_owner(db, principal, task_id, None).await
    {
        return resp;
    }
    conflict("Task is not in a gate-attachable status")
}

async fn list_gates(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_by_task(&state.db, &principal, &task_id).await {
        return r;
    }
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM task WHERE id=$1")
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

    let domain_id = match crate::routes::authz::domain_id_for_gate(&state.db, &gate_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let now = Utc::now().naive_utc();
    let now_tz = Utc::now();
    let subject_id = principal
        .subject
        .strip_prefix("agent:")
        .unwrap_or(&principal.subject);

    // Second pass (independent rust-reviewer, Task 7): the first version of
    // this function fenced the reviewgate UPDATE and the task-transition
    // UPDATE individually but ran them as two separate autocommitted
    // statements, with the any_rejected/all_resolved aggregate computed in a
    // THIRD statement in between — a gate created by a concurrent
    // create_gate call (which has no task-status precondition at all,
    // verified live) between that aggregate SELECT and the task UPDATE
    // could be missed entirely, letting a task finish with a still-pending
    // approval gate. This is exactly the race the spec's "recomputes
    // remaining-gate state in the same transaction so a second gate created
    // concurrently isn't missed" language calls out. Fixed: both statements
    // now run inside one explicit transaction (the `claim_task` FOR UPDATE
    // SKIP LOCKED path above is this file's existing precedent for
    // `state.db.begin()`), and the aggregate is folded into the task
    // UPDATE's own CTE — the same technique complete_task's pending-gate
    // check already uses for an identical cross-table-read-inside-a-fence
    // problem — instead of a separate, race-prone SELECT.
    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("resolve_gate begin tx: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Fenced: the ownership check and the pending-status precondition move
    // into the UPDATE's WHERE clause, same converge-into-WHERE pattern as
    // every other endpoint in this plan, applied here to the reviewgate row
    // instead of the task row (resolve_gate's authorization is gate
    // ownership, not a task lease — see this fn's own doc note above, "does
    // NOT use classify_fenced_rejection"). Closes a real TOCTOU the prior
    // code had: it SELECTed the gate, checked owner+status in application
    // code, then did a blind `UPDATE ... WHERE id=$1` with no status guard
    // at all — two callers who both observed status='pending' from their own
    // separate SELECT could both pass the app-level check, and the second
    // caller's UPDATE would silently overwrite the first's decision.
    // Independently flagged by a gpt-5.6-terra review, verified against this
    // exact code before fixing (SDD ledger, Task 7).
    let updated_gate = sqlx::query(
        "UPDATE reviewgate SET status=$2, resolved_at=$3 \
         WHERE id=$1 AND mesh_task_id=$4 AND status='pending' AND (owner_subject=$5 OR $6) \
         RETURNING *",
    )
    .bind(&gate_id)
    .bind(&body.decision)
    .bind(now)
    .bind(&task_id)
    .bind(&principal.subject)
    .bind(principal.is_admin)
    .fetch_optional(&mut *tx)
    .await;

    let gate_row = match updated_gate {
        Ok(Some(r)) => r,
        Ok(None) => {
            let _ = tx.rollback().await;
            // Zero rows: re-fetch (unfenced, pool connection — the tx that
            // would have seen the pending row just rolled back) to classify
            // why the same way classify_fenced_rejection does for the task
            // table — 404 if the gate doesn't exist for this task, 409 if it
            // exists but isn't pending (a real, already-applied decision —
            // including a decision that just won a race this same request
            // lost), 403 only for a genuine owner mismatch.
            let existing = sqlx::query(
                "SELECT owner_subject, status FROM reviewgate WHERE id=$1 AND mesh_task_id=$2",
            )
            .bind(&gate_id)
            .bind(&task_id)
            .fetch_optional(&state.db)
            .await;
            return match existing {
                Ok(Some(r)) => {
                    let status: String = r.get("status");
                    if status != "pending" {
                        conflict(&format!("Gate is already {status}"))
                    } else {
                        (
                            StatusCode::FORBIDDEN,
                            Json(serde_json::json!({"detail": "Not authorized to resolve this gate"})),
                        )
                            .into_response()
                    }
                }
                Ok(None) => not_found("Gate not found"),
                Err(e) => {
                    tracing::error!("resolve_gate classify: {e}");
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            };
        }
        Err(e) => {
            tracing::error!("resolve_gate update: {e}");
            let _ = tx.rollback().await;
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let gate_val = row_to_gate(&gate_row);

    // Fenced task transition, atomic with the gate UPDATE above (same tx).
    // The any_rejected/all_resolved aggregate is computed by the CTE against
    // whatever reviewgate rows exist for this task AT THE MOMENT this
    // statement runs — including the row this same request just resolved,
    // visible to itself within the open transaction — rather than from a
    // separately-fetched snapshot that a concurrent create_gate could race.
    // `bool_and` over zero rows is NULL (fails the `(agg.any_rejected OR
    // agg.all_resolved)` guard rather than defaulting to "finish the task"
    // the way the old code's `all(...)`-over-empty-Vec did) — unreachable in
    // practice here since the row just resolved above is always visible,
    // but a correct guard rather than an accidental one.
    //
    // finalized_by_subject: COALESCE prefers the task's actual claimer
    // (still present on the row — the pending-gate CTE in complete_task
    // never clears claimed_by_agent_id while waiting_review), falling back
    // to any attribution a prior cycle already recorded, falling back to
    // the gate resolver's own (agent-prefix-stripped, matching every
    // sibling endpoint's `subject_id` convention) identity. This mirrors
    // complete_task's/fail_task's "record the claimer" rationale, not
    // cancel_task's "record the interrupter" one — resolving a gate
    // finalizes the claimer's own submitted work, it doesn't interrupt
    // someone else's. Roadmap item: "resolve_gate inherits the attribution
    // + idempotent-retry pattern from Tasks 2/3's post-dual-review fix."
    //
    // finalized_at: every other terminal-transition endpoint in this crate
    // (tasks.rs, mcp.rs, complete_task, fail_task, cancel_task) stamps it;
    // resolve_gate was a gap, closed here rather than left inconsistent
    // while this code is already being touched. (dispatch_task is a
    // separate, still-open gap of the same shape — out of this plan's
    // scope, flagged in the roadmap rather than fixed here.)
    let updated_task = sqlx::query(
        "WITH agg AS ( \
           SELECT bool_or(status='rejected') AS any_rejected, \
                  bool_and(status IN ('approved','expired')) AS all_resolved \
           FROM reviewgate WHERE mesh_task_id=$1 \
         ) \
         UPDATE task SET \
           status = CASE WHEN agg.any_rejected THEN 'failed' ELSE 'finished' END, \
           finalized_by_subject = COALESCE(task.claimed_by_agent_id, task.finalized_by_subject, $3), \
           finalized_at=$4, lease_expires_at=NULL, claim_lease_id=NULL, \
           claimed_by_agent_id=NULL, updated_at=$2 \
         FROM agg \
         WHERE task.id=$1 AND task.status='waiting_review' \
           AND (agg.any_rejected OR agg.all_resolved) \
         RETURNING task.status, task.mission_id, task.domain_id",
    )
    .bind(&task_id)
    .bind(now)
    .bind(subject_id)
    .bind(now_tz)
    .fetch_optional(&mut *tx)
    .await;

    let transitioned = match updated_task {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("resolve_gate task transition: {e}");
            let _ = tx.rollback().await;
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Err(e) = tx.commit().await {
        tracing::error!("resolve_gate commit: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Only a real transition to 'finished' unblocks dependents — a 'failed'
    // transition, a still-waiting_review no-op (some gates still pending),
    // or a stale recompute that lost the CAS to an earlier resolution (this
    // row's own read of `agg` is unaffected by that — see the residual-race
    // note in the SDD ledger) never should.
    if let Some(r) = &transitioned
        && r.get::<String, _>("status") == "finished"
    {
        let mission_id: String = r.get("mission_id");
        let domain_id: String = r.get("domain_id");
        let unblocked = unblock_dependents(&state.db, &mission_id, &task_id).await;
        for tid in &unblocked {
            broadcast_task_available(&domain_id, &mission_id, tid).await;
        }
    }

    Json(gate_val).into_response()
}

// ── Agent notify WebSocket ────────────────────────────────────────────────────

async fn agent_notify_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(agent_id): Path<String>,
) -> Response {
    let domain_id = match crate::routes::authz::domain_id_for_agent(&state.db, &agent_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    ws.on_upgrade(move |socket| handle_agent_notify(socket, state, agent_id))
}

async fn handle_agent_notify(mut socket: WebSocket, state: Arc<AppState>, agent_id: String) {
    // Look up the agent's domain so we can subscribe to the right channel.
    let domain_id =
        match sqlx::query_scalar::<_, String>("SELECT domain_id FROM meshagent WHERE id=$1")
            .bind(&agent_id)
            .fetch_optional(&state.db)
            .await
        {
            Ok(Some(m)) => m,
            _ => return,
        };

    // Subscribe to the domain's broadcast channel, creating it on demand.
    let mut rx = {
        let mut reg = notify_registry().lock().await;
        let tx = reg
            .entry(domain_id.clone())
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

/// Mint and persist a fresh agent JWT. Returns the compact token string.
/// Errors propagate via `?` — callers decide whether to fail-hard or log.
async fn issue_agent_token(
    state: &AppState,
    agent_id: &str,
    domain_id: &str,
) -> anyhow::Result<String> {
    const TTL_HOURS: i64 = 12;
    let (token, jti) =
        crate::jwt::sign_agent_jwt(agent_id, domain_id, &state.jwt_encoding_key, TTL_HOURS)?;
    let expires_at = (Utc::now() + chrono::Duration::hours(TTL_HOURS)).naive_utc();
    sqlx::query(
        "INSERT INTO agenttoken (jti, agent_id, domain_id, revoked, expires_at, created_at) \
         VALUES ($1,$2,$3,false,$4,$5)",
    )
    .bind(&jti)
    .bind(agent_id)
    .bind(domain_id)
    .bind(expires_at)
    .bind(Utc::now().naive_utc())
    .execute(&state.db)
    .await?;
    Ok(token)
}

/// POST /work/agents/{agent_id}/token — mint a new agent JWT.
/// Requires full-trust (session/node) or admin. Denies agent principals to
/// prevent peer-impersonation.
async fn mint_agent_token(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    if !(crate::auth::is_full_trust(&principal) || principal.is_admin) {
        return (
            StatusCode::FORBIDDEN,
            Json(
                serde_json::json!({"detail": "full-trust principal required to mint agent tokens"}),
            ),
        )
            .into_response();
    }
    let domain_id = match crate::routes::authz::domain_id_for_agent(&state.db, &agent_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    match issue_agent_token(&state, &agent_id, &domain_id).await {
        Ok(tok) => (
            StatusCode::OK,
            Json(serde_json::json!({"agent_token": tok, "expires_in": 12 * 3600})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("mint_agent_token {agent_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn enroll_agent(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Json(body): Json<AgentEnroll>,
) -> impl IntoResponse {
    // Verify domain exists
    let domain_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM domain WHERE id=$1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if domain_exists.is_none() {
        return not_found("Domain not found");
    }

    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
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
    let labels_json = serde_json::to_string(&body.labels).unwrap_or_else(|_| "{}".to_string());
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
            match crate::routes::agents::upsert_agent_by_name(&state.db, n.trim(), &caps_json).await
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
        "INSERT INTO meshagent (id, domain_id, node_id, runtime_kind, runtime_version, \
         capabilities, labels, status, current_task_id, enrolled_by_subject, enrolled_at, \
         last_heartbeat_at, runtime_node_id, profile_json, machine_json, runtime_json, \
         supervision_mode, agent_public_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,'online',NULL,$8,$9,NULL,$10,$11,$12,$13,NULL,$14) RETURNING *",
    )
    .bind(&agent_id)
    .bind(&domain_id)
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
            let mut agent_json = row_to_agent(&r);
            // This meshagent row may grant a node domain scope it didn't have
            // before — invalidate both possible cache keys (the resolver
            // matches on `runtime_node_id OR node_id`) so the node can act in
            // this domain immediately rather than waiting out the TTL.
            if let Some(ref n) = body.node_id {
                crate::auth::invalidate_node_scope_cache(&state, n);
            }
            if let Some(ref n) = body.runtime_node_id
                && body.node_id.as_deref() != Some(n.as_str())
            {
                crate::auth::invalidate_node_scope_cache(&state, n);
            }
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
            // Best-effort token mint — log on failure, don't fail the enroll.
            match issue_agent_token(&state, &agent_id, &domain_id).await {
                Ok(tok) => {
                    agent_json["agent_token"] = serde_json::Value::String(tok);
                }
                Err(e) => {
                    tracing::error!("enroll_agent token mint for {agent_id}: {e}");
                }
            }
            (StatusCode::CREATED, Json(agent_json)).into_response()
        }
        Err(e) => {
            tracing::error!("enroll_agent: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_domain_agents(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return r;
    }
    let domain_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM domain WHERE id=$1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if domain_exists.is_none() {
        return not_found("Domain not found");
    }

    match sqlx::query("SELECT * FROM meshagent WHERE domain_id=$1 ORDER BY enrolled_at ASC")
        .bind(&domain_id)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_agent).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_domain_agents: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn agent_heartbeat(
    State(state): State<Arc<AppState>>,
    principal: Principal,
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

    let domain_id = match crate::routes::authz::domain_id_for_agent(&state.db, &agent_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    // Change 11a: non-full-trust callers may only heartbeat their own agent.
    if !crate::auth::is_full_trust(&principal) && !principal.is_admin {
        let self_id = principal
            .subject
            .strip_prefix("agent:")
            .unwrap_or(&principal.subject);
        if self_id != agent_id.as_str() {
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    let now = Utc::now().naive_utc();
    match sqlx::query("UPDATE meshagent SET last_heartbeat_at=$2 WHERE id=$1 RETURNING *")
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

/// Delete a meshagent row. Authorized for admin OR the subject that enrolled
/// the agent (meshagent.enrolled_by_subject). FK behavior on `agentrun` is
/// `ON DELETE SET NULL`, so audit rows survive.
///
/// Distinct from `revoke_node_agent` (DELETE /runtime/nodes/{node_id}/agents/{agent_id})
/// which requires the agent to be assigned to a registered runtime node owned by
/// the caller. This endpoint exists for the ephemeral-subagent model where the
/// spawner needs to clean up its own meshagent rows independent of node assignment.
async fn delete_agent(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let row = sqlx::query(
        "SELECT enrolled_by_subject, node_id, runtime_node_id, domain_id FROM meshagent WHERE id=$1",
    )
    .bind(&agent_id)
    .fetch_optional(&state.db)
    .await;
    let (enrolled_by, node_id, runtime_node_id, domain_id): (
        String,
        Option<String>,
        Option<String>,
        String,
    ) = match row {
        Ok(Some(r)) => (
            r.get("enrolled_by_subject"),
            r.get::<Option<String>, _>("node_id"),
            r.get::<Option<String>, _>("runtime_node_id"),
            r.get("domain_id"),
        ),
        Ok(None) => return not_found("Agent not found"),
        Err(e) => {
            tracing::error!("delete_agent lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    if enrolled_by != principal.subject && !principal.is_admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    if let Err(e) = sqlx::query("DELETE FROM meshagent WHERE id=$1")
        .bind(&agent_id)
        .execute(&state.db)
        .await
    {
        tracing::error!("delete_agent delete: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // This may have been the last meshagent row linking either node id to
    // `domain_id` — invalidate both possible node-scope cache keys (the
    // resolver matches on `runtime_node_id OR node_id`) so a node doesn't
    // keep cached access to a domain it no longer operates.
    if let Some(ref n) = node_id {
        crate::auth::invalidate_node_scope_cache(&state, n);
    }
    if let Some(ref n) = runtime_node_id
        && node_id.as_deref() != Some(n.as_str())
    {
        crate::auth::invalidate_node_scope_cache(&state, n);
    }

    // Change 13: revoke outstanding agent tokens immediately on delete so the
    // per-agent JWT is invalidated rather than remaining valid up to 12 h TTL.
    if let Err(e) = sqlx::query("UPDATE agenttoken SET revoked = true WHERE agent_id = $1")
        .bind(&agent_id)
        .execute(&state.db)
        .await
    {
        // Non-fatal: the agent row is already gone; log and continue.
        tracing::warn!("delete_agent token revoke: {e}");
    }

    if let Some(node) = node_id {
        broadcast_assignment_changed(
            &node,
            serde_json::json!({
                "type": "agent.deleted",
                "agent_id": agent_id,
                "domain_id": domain_id,
            }),
        )
        .await;
    }

    StatusCode::NO_CONTENT.into_response()
}

async fn set_agent_status(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(agent_id): Path<String>,
    Query(q): Query<AgentStatusQuery>,
) -> impl IntoResponse {
    let valid = ["online", "busy", "idle", "offline", "errored"];
    if !valid.contains(&q.status.as_str()) {
        return bad_request(&format!(
            "Invalid status: {}. Must be one of: online, busy, idle, offline, errored",
            q.status
        ));
    }

    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM meshagent WHERE id=$1")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if exists.is_none() {
        return not_found("Agent not found");
    }

    let domain_id = match crate::routes::authz::domain_id_for_agent(&state.db, &agent_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    // Change 11b: non-full-trust callers may only set their own agent's status.
    if !crate::auth::is_full_trust(&principal) && !principal.is_admin {
        let self_id = principal
            .subject
            .strip_prefix("agent:")
            .unwrap_or(&principal.subject);
        if self_id != agent_id.as_str() {
            return StatusCode::FORBIDDEN.into_response();
        }
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
    principal: Principal,
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

    let domain_id = match crate::routes::authz::domain_id_for_agent(&state.db, &agent_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    // Change 11c: non-full-trust callers may only update their own agent's profile.
    if !crate::auth::is_full_trust(&principal) && !principal.is_admin {
        let self_id = principal
            .subject
            .strip_prefix("agent:")
            .unwrap_or(&principal.subject);
        if self_id != agent_id.as_str() {
            return StatusCode::FORBIDDEN.into_response();
        }
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
    principal: Principal,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_by_agent(&state.db, &principal, &agent_id).await {
        return r;
    }
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
    principal: Principal,
    Path(agent_id): Path<String>,
    Query(q): Query<AgentMessagesQuery>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_by_agent(&state.db, &principal, &agent_id).await {
        return r;
    }
    // Look up the agent's domain so we can also surface domain broadcasts.
    let domain_id: String = match sqlx::query("SELECT domain_id FROM meshagent WHERE id=$1")
        .bind(&agent_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r.get("domain_id"),
        Ok(None) => return not_found("Agent not found"),
        Err(e) => {
            tracing::error!("get_agent_messages lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Direct messages addressed to this agent + domain-scoped broadcasts.
    // since_id paging in the caller dedupes broadcasts across polls.
    match sqlx::query(
        "SELECT * FROM meshmessage \
         WHERE id > $1 \
           AND (to_agent_id = $2 \
                OR (to_agent_id IS NULL AND domain_id = $3)) \
         ORDER BY id ASC",
    )
    .bind(q.since_id)
    .bind(&agent_id)
    .bind(&domain_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            // Mark only direct messages as read — broadcasts have N recipients
            // and would need a per-recipient read table to track properly.
            let direct_ids: Vec<i32> = rows
                .iter()
                .filter(|r| r.get::<Option<String>, _>("to_agent_id").is_some())
                .map(|r| r.get::<i32, _>("id"))
                .collect();
            if !direct_ids.is_empty() {
                let now = Utc::now().naive_utc();
                let _ = sqlx::query(
                    "UPDATE meshmessage SET read_at=$2 WHERE id = ANY($1) AND read_at IS NULL",
                )
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

// ── Domain message handlers ───────────────────────────────────────────────────

async fn list_domain_messages(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Query(q): Query<MessageListQuery>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return r;
    }
    let domain_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM domain WHERE id=$1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if domain_exists.is_none() {
        return not_found("Domain not found");
    }

    let since_id = q.since_id.unwrap_or(0);

    let rows = if let Some(channel) = &q.channel {
        sqlx::query(
            "SELECT * FROM meshmessage WHERE domain_id=$1 AND channel=$2 AND id > $3 \
             ORDER BY id ASC",
        )
        .bind(&domain_id)
        .bind(channel)
        .bind(since_id)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query("SELECT * FROM meshmessage WHERE domain_id=$1 AND id > $2 ORDER BY id ASC")
            .bind(&domain_id)
            .bind(since_id)
            .fetch_all(&state.db)
            .await
    };

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_message).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_domain_messages: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn send_domain_message(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Json(body): Json<MessageCreate>,
) -> impl IntoResponse {
    let domain_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM domain WHERE id=$1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if domain_exists.is_none() {
        return not_found("Domain not found");
    }

    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let body_json_str = if let Some(ref v) = body.body {
        serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
    } else {
        body.body_json.clone()
    };

    let now = Utc::now().naive_utc();

    let row = sqlx::query(
        "INSERT INTO meshmessage (domain_id, mission_id, from_agent_id, to_agent_id, task_id, \
         channel, body_json, in_reply_to, created_at, read_at) \
         VALUES ($1,NULL,$2,$3,$4,$5,$6,$7,$8,NULL) RETURNING id, created_at",
    )
    .bind(&domain_id)
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
                "id": r.get::<i32, _>("id"),
                "created_at": r.get::<chrono::NaiveDateTime, _>("created_at"),
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("send_domain_message: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn domain_roster(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return r;
    }
    let domain_exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM domain WHERE id=$1")
        .bind(&domain_id)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);
    if domain_exists.is_none() {
        return not_found("Domain not found");
    }

    // Return agents grouped by status
    let agents = sqlx::query("SELECT * FROM meshagent WHERE domain_id=$1 ORDER BY enrolled_at ASC")
        .bind(&domain_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let mut roster: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for row in &agents {
        let status: String = row.get("status");
        roster.entry(status).or_default().push(row_to_agent(row));
    }

    Json(serde_json::json!({
        "domain_id": domain_id,
        "agents": agents.iter().map(row_to_agent).collect::<Vec<_>>(),
        "by_status": roster,
        "total": agents.len(),
    }))
    .into_response()
}

// ── Mission message handlers ───────────────────────────────────────────────────

async fn list_mission_messages(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Query(q): Query<MessageListQuery>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_by_mission(&state.db, &principal, &mission_id).await
    {
        return r;
    }
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
        sqlx::query("SELECT * FROM meshmessage WHERE mission_id=$1 AND id > $2 ORDER BY id ASC")
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
    let domain_id = match crate::routes::authz::domain_id_for_mission(&state.db, &mission_id).await
    {
        Ok(d) => d,
        Err(resp) => return resp,
    };

    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let body_json_str = if let Some(ref v) = body.body {
        serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
    } else {
        body.body_json.clone()
    };

    let now = Utc::now().naive_utc();

    let row = sqlx::query(
        "INSERT INTO meshmessage (domain_id, mission_id, from_agent_id, to_agent_id, task_id, \
         channel, body_json, in_reply_to, created_at, read_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,NULL) RETURNING id, created_at",
    )
    .bind(&domain_id)
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
                "id": r.get::<i32, _>("id"),
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

// ── WebSocket streams ──────────────────────────────────────────────────────────

async fn mission_stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
) -> Response {
    let domain_id = match crate::routes::authz::domain_id_for_mission(&state.db, &mission_id).await
    {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    ws.on_upgrade(move |socket| poll_ledger_stream(socket, state, "mission_id".into(), mission_id))
}

async fn domain_stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
) -> Response {
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    ws.on_upgrade(move |socket| poll_ledger_stream(socket, state, "domain_id".into(), domain_id))
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
            if id > last_id {
                last_id = id;
            }
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
            if socket
                .send(Message::Text(evt.to_string().into()))
                .await
                .is_err()
            {
                return;
            }
        }

        ticks_since_ping += 1;
        if ticks_since_ping >= 15 {
            ticks_since_ping = 0;
            let ping = serde_json::json!({"type": "ping"});
            if socket
                .send(Message::Text(ping.to_string().into()))
                .await
                .is_err()
            {
                return;
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

// ── Global SSE feed: GET /sse ──────────────────────────────────────────────────
// Streams meshprogressevent rows for all tasks/agents. Heartbeat every 30s.
// Polls every 2 seconds for rows with id > last_seen.

async fn global_sse(State(state): State<Arc<AppState>>, principal: Principal) -> Response {
    if !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "admin required"})),
        )
            .into_response();
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);
    let db = state.db.clone();

    tokio::spawn(async move {
        let mut last_id: i32 = 0;
        let mut ticks_since_heartbeat: u32 = 0;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let rows = sqlx::query(
                "SELECT id, task_id, agent_id, seq, event_type, phase, step, summary, occurred_at \
                 FROM meshprogressevent WHERE id > $1 ORDER BY id ASC LIMIT 100",
            )
            .bind(last_id)
            .fetch_all(&db)
            .await
            .unwrap_or_default();

            for row in &rows {
                let id: i32 = row.get("id");
                if id > last_id {
                    last_id = id;
                }
                let data = serde_json::json!({
                    "id": id,
                    "task_id": row.get::<String, _>("task_id"),
                    "agent_id": row.get::<String, _>("agent_id"),
                    "seq": row.get::<i32, _>("seq"),
                    "event_type": row.get::<String, _>("event_type"),
                    "phase": row.get::<Option<String>, _>("phase"),
                    "step": row.get::<Option<String>, _>("step"),
                    // summary is nullable `text` (rows written via the MCP
                    // progress_mesh_task path never populate it) — decoding as
                    // non-Option panics via Row::get, same class as the
                    // meshmessage/meshprogressevent fixes in #113.
                    "summary": row.get::<Option<String>, _>("summary"),
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
