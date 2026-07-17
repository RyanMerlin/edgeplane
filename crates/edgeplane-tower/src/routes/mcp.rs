use axum::{
    Json, Router,
    extract::State,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    // Public surface (intentional, no `Principal` extraction):
    //   * GET /mcp/tools   — static tool catalogue (no state mutation).
    //   * GET /mcp/health  — health check for monitoring.
    // Authenticated:
    //   * POST /mcp/call   — dispatches real work scoped to the caller; the
    //     handler extracts `Principal` and per-tool authorisation logic
    //     reads `principal.subject` for owner filtering.
    Router::new()
        .route("/mcp/tools", get(list_tools))
        .route("/mcp/health", get(mcp_health))
        .route("/mcp/call", post(call_tool))
}

async fn mcp_health() -> impl IntoResponse {
    Json(json!({"ok": true, "version": "edgeplane-tower"}))
}

fn tool_def(name: &str, description: &str, schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": schema})
}

async fn list_tools() -> impl IntoResponse {
    // ADR 0006: runtime-only keep set (17) + extract-first tools (7, no REST route yet) + northstar (1).
    // Total: 25. Management tools are served via REST API or the edgeplane CLI.
    let tools = vec![
        // ── Core mesh work (14) ────────────────────────────────────────────────
        tool_def(
            "submit_mesh_task",
            "Create a task in a mission (mesh work model)",
            json!({"type":"object","properties":{"mission_id":{"type":"string"},"title":{"type":"string"},"description":{"type":"string"},"kind":{"type":"string"},"input_json":{"type":"string"},"priority":{"type":"integer"},"domain_id":{"type":"string"}}}),
        ),
        tool_def(
            "list_mesh_tasks",
            "List tasks in a mission (mesh work model)",
            json!({"type":"object","properties":{"mission_id":{"type":"string"},"status":{"type":"string"},"limit":{"type":"integer"}}}),
        ),
        tool_def(
            "claim_mesh_task",
            "Claim a mesh task for an agent; returns claim_lease_id",
            json!({"type":"object","properties":{"task_id":{"type":"string"},"agent_id":{"type":"string"},"lease_seconds":{"type":"integer"}}}),
        ),
        tool_def(
            "heartbeat_mesh_task",
            "Renew a mesh task lease to prevent expiry",
            json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"}}}),
        ),
        tool_def(
            "progress_mesh_task",
            "Post a typed progress event for a mesh task",
            json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"},"event_type":{"type":"string"},"payload_json":{"type":"string"}}}),
        ),
        tool_def(
            "complete_mesh_task",
            "Mark a mesh task as complete",
            json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"},"output_json":{"type":"string"}}}),
        ),
        tool_def(
            "fail_mesh_task",
            "Mark a mesh task as failed",
            json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"},"error":{"type":"string"}}}),
        ),
        tool_def(
            "block_mesh_task",
            "Mark a mesh task as blocked",
            json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"},"reason":{"type":"string"}}}),
        ),
        tool_def(
            "load_mission_workspace",
            "Load/sync a mission workspace and acquire a lease",
            json!({"type":"object","properties":{"mission_id":{"type":"string"},"workspace_label":{"type":"string"},"agent_id":{"type":"string"},"lease_seconds":{"type":"integer"}},"required":["mission_id"]}),
        ),
        tool_def(
            "heartbeat_workspace_lease",
            "Extend a workspace lease heartbeat",
            json!({"type":"object","properties":{"lease_id":{"type":"string"}},"required":["lease_id"]}),
        ),
        tool_def(
            "commit_mission_workspace",
            "Commit workspace changes with optimistic conflict checks",
            json!({"type":"object","properties":{"lease_id":{"type":"string"},"change_set":{"type":"array"},"validation_mode":{"type":"string"}},"required":["lease_id","change_set"]}),
        ),
        tool_def(
            "release_mission_workspace",
            "Release an active workspace lease",
            json!({"type":"object","properties":{"lease_id":{"type":"string"},"reason":{"type":"string"}},"required":["lease_id"]}),
        ),
        tool_def(
            "send_mesh_message",
            "Send a message in a mission or domain channel",
            json!({"type":"object","properties":{"mission_id":{"type":"string"},"domain_id":{"type":"string"},"content":{"type":"string"},"sender_agent_id":{"type":"string"},"recipient_agent_id":{"type":"string"}}}),
        ),
        tool_def(
            "list_mesh_messages",
            "List messages for an agent inbox",
            json!({"type":"object","properties":{"agent_id":{"type":"string"},"mission_id":{"type":"string"},"limit":{"type":"integer"}}}),
        ),
        // ── Borderline keep (3) ────────────────────────────────────────────────
        tool_def(
            "get_overlap_suggestions",
            "Get overlap suggestions for a task",
            json!({"type":"object","properties":{"task_id":{"type":"string"},"limit":{"type":"integer"}}}),
        ),
        tool_def(
            "fetch_workspace_artifact",
            "Fetch artifact bytes or signed download URL while a lease is active",
            json!({"type":"object","properties":{"lease_id":{"type":"string"},"artifact_id":{"type":"integer"},"mode":{"type":"string"},"expires_seconds":{"type":"integer"}},"required":["lease_id","artifact_id"]}),
        ),
        tool_def(
            "get_mesh_task",
            "Get a single mesh task by ID",
            json!({"type":"object","properties":{"task_id":{"type":"string"}}}),
        ),
        // ── Extract-first (7): kept here until REST routes exist ───────────────
        tool_def(
            "get_artifact_download_url",
            "Get a short-lived download URL for an S3-backed artifact",
            json!({"type":"object","properties":{"artifact_id":{"type":"integer"},"expires_seconds":{"type":"integer"}}}),
        ),
        tool_def(
            "publish_pending_ledger_events",
            "Publish pending domain-scoped ledger events to Git",
            json!({"type":"object","properties":{"domain_id":{"type":"string"}}}),
        ),
        tool_def(
            "provision_domain_persistence",
            "Create/update connection, binding, and domain policy routes in one call",
            json!({"type":"object","properties":{"domain_id":{"type":"string"}}}),
        ),
        tool_def(
            "resolve_publish_plan",
            "Resolve publish route (binding/repo/branch/path) for an entity",
            json!({"type":"object","properties":{"entity_type":{"type":"string"},"entity_id":{"type":"string"},"domain_id":{"type":"string"}}}),
        ),
        tool_def(
            "get_publication_status",
            "List recent publication records",
            json!({"type":"object","properties":{"domain_id":{"type":"string"},"limit":{"type":"integer"}}}),
        ),
        // ── Domain context (1) ─────────────────────────────────────────────────
        tool_def(
            "get_domain_northstar",
            "Load the Northstar narrative for a domain — describes the domain's purpose, scope, and direction",
            json!({
                "type": "object",
                "properties": {
                    "domain_id": {"type": "string", "description": "The domain id"}
                },
                "required": ["domain_id"]
            }),
        ),
    ];
    Json(tools)
}

/// Return the names of all tools advertised in `list_tools()`.
/// Used by the parity test to ensure catalogue ↔ dispatch stay in sync.
pub fn advertised_tool_names() -> Vec<&'static str> {
    vec![
        "submit_mesh_task",
        "list_mesh_tasks",
        "claim_mesh_task",
        "heartbeat_mesh_task",
        "progress_mesh_task",
        "complete_mesh_task",
        "fail_mesh_task",
        "block_mesh_task",
        "load_mission_workspace",
        "heartbeat_workspace_lease",
        "commit_mission_workspace",
        "release_mission_workspace",
        "send_mesh_message",
        "list_mesh_messages",
        "get_overlap_suggestions",
        "fetch_workspace_artifact",
        "get_mesh_task",
        "get_artifact_download_url",
        "publish_pending_ledger_events",
        "provision_domain_persistence",
        "resolve_publish_plan",
        "get_publication_status",
        "get_domain_northstar",
    ]
}

/// Return the names of all tools with a dispatch arm in `dispatch()`.
/// Used by the parity test to ensure catalogue ↔ dispatch stay in sync.
pub fn dispatch_handled_names() -> Vec<&'static str> {
    vec![
        "submit_mesh_task",
        "list_mesh_tasks",
        "get_mesh_task",
        "claim_mesh_task",
        "heartbeat_mesh_task",
        "progress_mesh_task",
        "complete_mesh_task",
        "fail_mesh_task",
        "block_mesh_task",
        "send_mesh_message",
        "list_mesh_messages",
        "get_overlap_suggestions",
        "fetch_workspace_artifact",
        "load_mission_workspace",
        "heartbeat_workspace_lease",
        "commit_mission_workspace",
        "release_mission_workspace",
        "get_artifact_download_url",
        "publish_pending_ledger_events",
        "provision_domain_persistence",
        "resolve_publish_plan",
        "get_publication_status",
        "get_domain_northstar",
    ]
}

#[derive(Deserialize)]
struct McpCallRequest {
    tool: String,
    args: Option<Value>,
}

fn ok_result(result: Value) -> Value {
    json!({"ok": true, "result": result})
}

fn err_result(error: &str) -> Value {
    json!({"ok": false, "error": error, "result": {}})
}

#[allow(dead_code)]
fn not_impl() -> Value {
    err_result("not_implemented_in_rust_server")
}

fn uri_encode_path(s: &str) -> String {
    s.split('/').map(uri_encode).collect::<Vec<_>>().join("/")
}

fn uri_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

fn sigv4_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    use hmac::Mac;
    fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(key).unwrap();
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

async fn call_tool(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<McpCallRequest>,
) -> impl IntoResponse {
    let args = payload.args.unwrap_or(json!({}));
    let result = dispatch(&state, &principal, &payload.tool, &args).await;
    Json(result)
}

async fn mcp_authz_domain(state: &AppState, p: &Principal, domain_id: &str) -> Result<(), Value> {
    match crate::routes::authz::authz_domain(&state.db, p, domain_id).await {
        Ok(()) => Ok(()),
        Err(resp) if resp.status() == axum::http::StatusCode::INTERNAL_SERVER_ERROR => {
            Err(json!({ "ok": false, "error": "database_error" }))
        }
        Err(_) => Err(json!({ "ok": false, "error": "forbidden", "detail": "not authorized for domain" })),
    }
}

async fn dispatch(state: &AppState, principal: &Principal, tool: &str, args: &Value) -> Value {
    let now = Utc::now().naive_utc();

    match tool {
        // ── Overlap ───────────────────────────────────────────────────────────
        "get_overlap_suggestions" => {
            let task_id = match int_arg(args, "task_id") {
                Some(v) => v as i32,
                None => return err_result("task_id is required"),
            };
            // Change 4: resolve domain via task→mission→domain and authz before the SELECT.
            // overlapsuggestion.task_id is integer FK to task.id (workspace task model).
            let domain_id_result = sqlx::query_scalar::<_, Option<String>>(
                "SELECT m.domain_id FROM overlapsuggestion o \
                 JOIN task t ON t.id = o.task_id \
                 JOIN mission m ON m.id = t.mission_id \
                 WHERE o.task_id = $1 LIMIT 1"
            )
            .bind(task_id)
            .fetch_optional(&state.db)
            .await;
            let domain_id = match domain_id_result {
                Err(e) => {
                    tracing::error!("mcp get_overlap_suggestions domain resolve (task_id={task_id}): {e}");
                    return err_result("task not found");
                }
                Ok(row) => match row.flatten() {
                    Some(d) => d,
                    None => return err_result("task not found"),
                },
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let limit = int_arg(args, "limit").unwrap_or(10).min(50);
            match sqlx::query(
                "SELECT id, task_id, candidate_task_id, similarity_score, evidence, suggested_action \
                 FROM overlapsuggestion WHERE task_id=$1 ORDER BY similarity_score DESC LIMIT $2"
            )
            .bind(task_id).bind(limit).fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "task_id": r.get::<i32,_>("task_id"),
                    "candidate_task_id": r.get::<i32,_>("candidate_task_id"),
                    "similarity_score": r.get::<f64,_>("similarity_score"),
                    "evidence": r.get::<String,_>("evidence"),
                    "suggested_action": r.get::<String,_>("suggested_action"),
                })).collect())),
                Err(e) => { tracing::error!("mcp get_overlap_suggestions: {e}"); err_result("database_error") }
            }
        }

        // ── Mesh tasks ────────────────────────────────────────────────────────
        "submit_mesh_task" => {
            let mission_id = str_arg(args, "mission_id");
            let title = str_arg(args, "title");
            if mission_id.is_empty() || title.is_empty() {
                return err_result("mission_id and title are required");
            }
            // Resolve the canonical domain from the mission (closes client-supplied-domain mismatch).
            let domain_id = match crate::routes::authz::domain_id_for_mission(&state.db, &mission_id).await {
                Ok(d) => d,
                Err(_) => return err_result("mission not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let description = str_arg(args, "description");
            let input_json = args.get("input_json").cloned().unwrap_or(json!({}));
            let priority = int_arg(args, "priority").unwrap_or(0) as i32;
            let id = uuid::Uuid::new_v4().to_string();
            match sqlx::query(
                "INSERT INTO meshtask (id, mission_id, domain_id, title, description, input_json, \
                 priority, status, claim_policy, version_counter, created_by_subject, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,'ready','first_claim',0,$8,$9,$9)",
            )
            .bind(&id)
            .bind(&mission_id)
            .bind(&domain_id)
            .bind(&title)
            .bind(&description)
            .bind(input_json.to_string())
            .bind(priority)
            .bind(&principal.subject)
            .bind(now)
            .execute(&state.db)
            .await
            {
                Ok(_) => ok_result(
                    json!({"task_id": id, "mission_id": mission_id, "domain_id": domain_id, "title": title, "status": "ready"}),
                ),
                Err(e) => {
                    tracing::error!("mcp submit_mesh_task: {e}");
                    err_result("database_error")
                }
            }
        }

        "list_mesh_tasks" => {
            // Change 5: mission_id is now required; resolve domain and authz before the SELECT.
            let mission_id = str_arg(args, "mission_id");
            if mission_id.is_empty() {
                return err_result("mission_id is required");
            }
            let domain_id = match crate::routes::authz::domain_id_for_mission(&state.db, &mission_id).await {
                Ok(d) => d,
                Err(_) => return err_result("mission not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let status_filter = args.get("status").and_then(|v| v.as_str());
            let limit = int_arg(args, "limit").unwrap_or(50).min(200);
            match sqlx::query(
                "SELECT id, mission_id, domain_id, title, description, status, priority, \
                 claimed_by_agent_id, created_at, updated_at \
                 FROM meshtask \
                 WHERE mission_id=$1 \
                   AND ($2::text IS NULL OR status=$2) \
                 ORDER BY priority DESC, created_at ASC LIMIT $3"
            )
            .bind(&mission_id).bind(status_filter).bind(limit)
            .fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<String,_>("id"),
                    "mission_id": r.get::<String,_>("mission_id"),
                    "domain_id": r.get::<String,_>("domain_id"),
                    "title": r.get::<String,_>("title"),
                    "status": r.get::<String,_>("status"),
                    "priority": r.get::<i32,_>("priority"),
                    "claimed_by_agent_id": r.get::<Option<String>,_>("claimed_by_agent_id"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_mesh_tasks: {e}"); err_result("database_error") }
            }
        }

        "get_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() {
                return err_result("task_id is required");
            }
            // Change 6: resolve domain and authz before the SELECT.
            let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
                Ok(d) => d,
                Err(_) => return err_result("mesh_task_not_found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            match sqlx::query(
                "SELECT id, mission_id, domain_id, title, description, status, priority, \
                 input_json, claimed_by_agent_id, claim_lease_id, lease_expires_at, \
                 created_at, updated_at FROM meshtask WHERE id=$1",
            )
            .bind(&task_id)
            .fetch_optional(&state.db)
            .await
            {
                Ok(Some(r)) => {
                    // Drop claim_lease_id for non-owners to avoid live-lease exposure.
                    let claimed_by: Option<String> = r.get("claimed_by_agent_id");
                    let lease_id: Option<String> = r.get("claim_lease_id");
                    let self_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
                    let is_owner = crate::auth::is_full_trust(principal)
                        || principal.is_admin
                        || claimed_by.as_deref() == Some(self_id);
                    ok_result(json!({
                        "id": r.get::<String,_>("id"),
                        "mission_id": r.get::<String,_>("mission_id"),
                        "domain_id": r.get::<String,_>("domain_id"),
                        "title": r.get::<String,_>("title"),
                        "description": r.get::<String,_>("description"),
                        "status": r.get::<String,_>("status"),
                        "priority": r.get::<i32,_>("priority"),
                        "claimed_by_agent_id": claimed_by,
                        "claim_lease_id": if is_owner { lease_id } else { None },
                        "lease_expires_at": r.get::<Option<chrono::NaiveDateTime>,_>("lease_expires_at"),
                    }))
                }
                Ok(None) => err_result("mesh_task_not_found"),
                Err(e) => {
                    tracing::error!("mcp get_mesh_task: {e}");
                    err_result("database_error")
                }
            }
        }

        "claim_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() {
                return err_result("task_id is required");
            }
            // Full-trust callers may supply an explicit agent_id; restricted
            // callers (agents, SA) are always attributed to themselves.
            let self_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
            let agent_id = if crate::auth::is_full_trust(principal) || principal.is_admin {
                str_arg(args, "agent_id")
            } else {
                self_id.to_string()
            };
            if agent_id.is_empty() {
                return err_result("agent_id is required (or authenticate as an agent)");
            }
            // resolve domain and guard
            let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
                Ok(d) => d,
                Err(_) => return err_result("task not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let lease_seconds = int_arg(args, "lease_seconds").unwrap_or(300);
            let lease_id = uuid::Uuid::new_v4().to_string();
            let expires_at = now + chrono::Duration::seconds(lease_seconds);
            match sqlx::query(
                "UPDATE meshtask SET status='claimed', claimed_by_agent_id=$2, claim_lease_id=$3, \
                 lease_expires_at=$4, version_counter=version_counter+1, updated_at=NOW() \
                 WHERE id=$1 AND status='ready' RETURNING id",
            )
            .bind(&task_id)
            .bind(&agent_id)
            .bind(&lease_id)
            .bind(expires_at)
            .fetch_optional(&state.db)
            .await
            {
                Ok(Some(_)) => ok_result(
                    json!({"task_id": task_id, "claim_lease_id": lease_id, "lease_expires_at": expires_at}),
                ),
                Ok(None) => {
                    json!({"ok": false, "error": "conflict", "detail": "task not found or already claimed"})
                }
                Err(e) => {
                    tracing::error!("mcp claim_mesh_task: {e}");
                    err_result("database_error")
                }
            }
        }

        "heartbeat_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            let claim_lease_id = str_arg(args, "claim_lease_id");
            if task_id.is_empty() || claim_lease_id.is_empty() {
                return err_result("task_id and claim_lease_id are required");
            }
            let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
                Ok(d) => d,
                Err(_) => return err_result("task not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let lease_opt = if claim_lease_id.is_empty() { None } else { Some(claim_lease_id.as_str()) };
            if crate::routes::authz::authz_task_owner(&state.db, principal, &task_id, lease_opt).await.is_err() {
                return err_result("not the task's claimer");
            }
            let expires_at = now + chrono::Duration::seconds(300);
            match sqlx::query(
                "UPDATE meshtask SET lease_expires_at=$3, updated_at=NOW() \
                 WHERE id=$1 AND claim_lease_id=$2 RETURNING id",
            )
            .bind(&task_id)
            .bind(&claim_lease_id)
            .bind(expires_at)
            .fetch_optional(&state.db)
            .await
            {
                Ok(Some(_)) => {
                    ok_result(json!({"task_id": task_id, "lease_expires_at": expires_at}))
                }
                Ok(None) => err_result("invalid_task_or_lease"),
                Err(e) => {
                    tracing::error!("mcp heartbeat_mesh_task: {e}");
                    err_result("database_error")
                }
            }
        }

        "progress_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            let event_type = str_arg(args, "event_type");
            // Full-trust callers may supply an explicit agent_id; restricted
            // callers (agents, SA) are always attributed to themselves.
            let self_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
            let agent_id = if crate::auth::is_full_trust(principal) || principal.is_admin {
                str_arg(args, "agent_id")
            } else {
                self_id.to_string()
            };
            if task_id.is_empty() || event_type.is_empty() {
                return err_result("task_id and event_type are required");
            }
            let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
                Ok(d) => d,
                Err(_) => return err_result("task not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            // Change 7: ownership check (mirrors heartbeat_mesh_task); use caller-presented lease.
            let claim_lease_str = str_arg(args, "claim_lease_id");
            let claim_lease_opt = if claim_lease_str.is_empty() { None } else { Some(claim_lease_str.as_str()) };
            if crate::routes::authz::authz_task_owner(&state.db, principal, &task_id, claim_lease_opt).await.is_err() {
                return err_result("not the task's claimer");
            }
            let payload_json = args.get("payload_json").cloned().unwrap_or(json!({}));
            let phase = args.get("phase").and_then(|v| v.as_str());
            let step = args.get("step").and_then(|v| v.as_str());
            // seq has no DB default (see meshprogressevent in migrations/0001) — mirror the
            // REST post_progress handler (routes/work.rs) and compute the next value ourselves.
            // seq is `integer` (i32) in Postgres; must match exactly or sqlx's runtime decode
            // fails silently into unwrap_or(0) (see routes/work.rs for the same fix).
            let seq: i32 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq), -1) + 1 FROM meshprogressevent WHERE task_id=$1",
            )
            .bind(&task_id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);
            match sqlx::query(
                "INSERT INTO meshprogressevent (task_id, agent_id, seq, event_type, phase, step, payload_json, occurred_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,NOW()) RETURNING id"
            )
            .bind(&task_id).bind(&agent_id).bind(seq).bind(&event_type).bind(phase).bind(step)
            .bind(payload_json.to_string())
            .fetch_one(&state.db).await
            {
                Ok(r) => ok_result(json!({"event_id": r.get::<i32,_>("id"), "task_id": task_id, "event_type": event_type})),
                Err(e) => { tracing::error!("mcp progress_mesh_task: {e}"); err_result("database_error") }
            }
        }

        "complete_mesh_task" | "fail_mesh_task" | "block_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() {
                return err_result("task_id is required");
            }
            let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
                Ok(d) => d,
                Err(_) => return err_result("task not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let lease_str = str_arg(args, "claim_lease_id");
            let lease_opt = if lease_str.is_empty() { None } else { Some(lease_str.as_str()) };
            if crate::routes::authz::authz_task_owner(&state.db, principal, &task_id, lease_opt).await.is_err() {
                return err_result("not the task's claimer");
            }
            let new_status = match tool {
                "complete_mesh_task" => "finished",
                "fail_mesh_task" => "failed",
                "block_mesh_task" => "blocked",
                _ => return err_result("unknown_tool"),
            };
            match sqlx::query(
                "UPDATE meshtask SET status=$2, updated_at=NOW(), \
                 claim_lease_id=CASE WHEN $2 IN ('finished','failed','cancelled') THEN NULL ELSE claim_lease_id END, \
                 claimed_by_agent_id=CASE WHEN $2 IN ('finished','failed','cancelled') THEN NULL ELSE claimed_by_agent_id END \
                 WHERE id=$1 RETURNING id"
            )
            .bind(&task_id).bind(new_status).fetch_optional(&state.db).await
            {
                Ok(Some(_)) => ok_result(json!({"task_id": task_id, "status": new_status})),
                Ok(None) => err_result("mesh_task_not_found"),
                Err(e) => { tracing::error!("mcp {tool}: {e}"); err_result("database_error") }
            }
        }

        // ── Mesh messages ─────────────────────────────────────────────────────
        "send_mesh_message" => {
            let domain_id = str_arg(args, "domain_id");
            let body = args.get("content").cloned().unwrap_or(json!({}));
            if domain_id.is_empty() {
                return err_result("domain_id is required");
            }
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            // Change 12: anti-spoof sender attribution (mirrors claim_mesh_task pattern).
            // Full-trust/admin may supply an explicit sender; restricted callers are
            // always attributed to their own identity, ignoring caller-supplied sender_agent_id.
            let from_agent_id = if crate::auth::is_full_trust(principal) || principal.is_admin {
                let supplied = str_arg(args, "sender_agent_id");
                if supplied.is_empty() {
                    principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject).to_string()
                } else {
                    supplied
                }
            } else {
                principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject).to_string()
            };
            if from_agent_id.is_empty() {
                return err_result("sender_agent_id is required (or authenticate as an agent)");
            }
            let to_agent_id = args.get("recipient_agent_id").and_then(|v| v.as_str());
            let mission_id = args.get("mission_id").and_then(|v| v.as_str());
            let channel = str_arg_or(args, "channel", "coordination");
            let body_json = if body.is_string() {
                json!({"text": body.as_str().unwrap_or("")})
            } else {
                body
            };
            match sqlx::query(
                "INSERT INTO meshmessage (domain_id, mission_id, from_agent_id, to_agent_id, channel, body_json, created_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,NOW()) RETURNING id"
            )
            .bind(&domain_id).bind(mission_id).bind(&from_agent_id).bind(to_agent_id)
            .bind(&channel).bind(body_json.to_string())
            .fetch_one(&state.db).await
            {
                Ok(r) => ok_result(json!({"message_id": r.get::<i32,_>("id"), "domain_id": domain_id})),
                Err(e) => { tracing::error!("mcp send_mesh_message: {e}"); err_result("database_error") }
            }
        }

        "list_mesh_messages" => {
            let agent_id = str_arg(args, "agent_id");
            let limit = int_arg(args, "limit").unwrap_or(20).min(100);
            if agent_id.is_empty() {
                return err_result("agent_id is required");
            }
            // Change 1: resolve the agent's domain, authz the caller for it, and
            // restrict direct-message reads to the agent's own identity.
            let domain_id = match crate::routes::authz::domain_id_for_agent(&state.db, &agent_id).await {
                Ok(d) => d,
                Err(_) => return err_result("agent not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            // Non-full-trust callers may only read their own agent's messages.
            if !crate::auth::is_full_trust(principal) && !principal.is_admin {
                let self_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
                if self_id != agent_id.as_str() {
                    return json!({ "ok": false, "error": "forbidden", "detail": "may only read own agent's messages" });
                }
            }
            match sqlx::query(
                "SELECT id, domain_id, from_agent_id, to_agent_id, channel, body_json, created_at, read_at \
                 FROM meshmessage \
                 WHERE (to_agent_id=$1 OR (to_agent_id IS NULL AND domain_id=$2)) \
                 ORDER BY created_at DESC LIMIT $3"
            )
            .bind(&agent_id).bind(&domain_id).bind(limit).fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "domain_id": r.get::<String,_>("domain_id"),
                    "from_agent_id": r.get::<String,_>("from_agent_id"),
                    "to_agent_id": r.get::<Option<String>,_>("to_agent_id"),
                    "channel": r.get::<String,_>("channel"),
                    // body_json is nullable `text` — decoding as non-Option panics via
                    // Row::get, same class as the row_to_message fix in #113 (this MCP
                    // tool handler is a separate code path #113 didn't reach).
                    "body_json": r.try_get::<Option<String>,_>("body_json").ok().flatten().unwrap_or_default(),
                    "read_at": r.get::<Option<chrono::NaiveDateTime>,_>("read_at"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_mesh_messages: {e}"); err_result("database_error") }
            }
        }

        // ── Publication ───────────────────────────────────────────────────────
        "resolve_publish_plan" => {
            let domain_id = str_arg(args, "domain_id");
            let entity_kind = str_arg(args, "entity_kind");
            let event_kind = str_arg(args, "event_kind");
            if domain_id.is_empty() || entity_kind.is_empty() {
                return err_result("domain_id and entity_kind are required");
            }
            // Change 3: authz before the SELECT — leaks publishing infra config.
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let row = sqlx::query(
                "SELECT r.id AS route_id, r.format, r.branch, r.rel_path_template, \
                 b.id AS binding_id, b.name AS binding_name, \
                 c.id AS conn_id, c.provider, c.host, c.repo_path \
                 FROM domainpersistenceroute r \
                 JOIN repobinding b ON b.id = r.binding_id \
                 JOIN repoconnection c ON c.id = b.connection_id \
                 WHERE r.domain_id=$1 AND r.entity_kind=$2 AND r.active=true \
                 AND (r.event_kind=$3 OR r.event_kind='') \
                 ORDER BY r.event_kind DESC LIMIT 1",
            )
            .bind(&domain_id)
            .bind(&entity_kind)
            .bind(&event_kind)
            .fetch_optional(&state.db)
            .await;
            match row {
                Ok(Some(r)) => ok_result(json!({
                    "binding_id": r.get::<i32,_>("binding_id"),
                    "binding_name": r.get::<String,_>("binding_name"),
                    "connection_id": r.get::<i32,_>("conn_id"),
                    "provider": r.get::<String,_>("provider"),
                    "host": r.get::<Option<String>,_>("host"),
                    "repo_path": r.get::<String,_>("repo_path"),
                    "branch": r.get::<Option<String>,_>("branch"),
                    "rel_path": r.get::<Option<String>,_>("rel_path_template"),
                    "format": r.get::<Option<String>,_>("format"),
                })),
                Ok(None) => err_result("no_publish_plan_found"),
                Err(e) => {
                    tracing::error!("mcp resolve_publish_plan: {e}");
                    err_result("database_error")
                }
            }
        }

        "get_publication_status" => {
            let domain_id = str_arg(args, "domain_id");
            let limit = int_arg(args, "limit").unwrap_or(20).min(200);
            let rows = if domain_id.is_empty() {
                sqlx::query("SELECT * FROM publicationrecord WHERE owner_subject=$1 ORDER BY created_at DESC LIMIT $2")
                    .bind(&principal.subject).bind(limit).fetch_all(&state.db).await
            } else {
                sqlx::query("SELECT * FROM publicationrecord WHERE owner_subject=$1 AND domain_id=$2 ORDER BY created_at DESC LIMIT $3")
                    .bind(&principal.subject).bind(&domain_id).bind(limit).fetch_all(&state.db).await
            };
            match rows {
                Ok(rows) => ok_result(json!(
                    rows.iter()
                        .map(|r| json!({
                            "id": r.get::<i32,_>("id"),
                            "owner_subject": r.get::<String,_>("owner_subject"),
                            "domain_id": r.get::<Option<String>,_>("domain_id"),
                            "entity_kind": r.get::<String,_>("entity_kind"),
                            "entity_id": r.get::<String,_>("entity_id"),
                            "event_kind": r.get::<Option<String>,_>("event_kind"),
                            "binding_id": r.get::<Option<i32>,_>("binding_id"),
                            "status": r.get::<String,_>("status"),
                            "error": r.get::<Option<String>,_>("error"),
                            "commit_sha": r.get::<Option<String>,_>("commit_sha"),
                            "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
                            "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
                        }))
                        .collect::<Vec<_>>()
                )),
                Err(e) => {
                    tracing::error!("mcp get_publication_status: {e}");
                    err_result("database_error")
                }
            }
        }

        // ── Provision persistence ─────────────────────────────────────────────
        "provision_domain_persistence" => {
            let domain_id = str_arg(args, "domain_id");
            if domain_id.is_empty() {
                return err_result("domain_id is required");
            }
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }

            let conn_input = args
                .get("connection")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let bind_input = args
                .get("binding")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let routes_input = args
                .get("routes")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let conn_name = conn_input
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let repo_path = conn_input
                .get("repo_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if conn_name.is_empty() || repo_path.is_empty() {
                return err_result("connection.name and connection.repo_path are required");
            }
            let provider = conn_input
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("github_app")
                .to_string();
            let host = conn_input
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("github.com")
                .to_string();
            let default_branch = conn_input
                .get("default_branch")
                .and_then(|v| v.as_str())
                .unwrap_or("main")
                .to_string();
            let credential_ref = conn_input
                .get("credential_ref")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let options_json = conn_input
                .get("options")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "{}".into());

            // Upsert RepoConnection
            let conn_row = sqlx::query(
                "INSERT INTO repoconnection (owner_subject, name, provider, host, repo_path, default_branch, credential_ref, options_json, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9) \
                 ON CONFLICT (owner_subject, name) DO UPDATE SET \
                 provider=$3, host=$4, repo_path=$5, default_branch=$6, credential_ref=$7, options_json=$8, updated_at=$9 \
                 RETURNING *"
            )
            .bind(&principal.subject).bind(&conn_name).bind(&provider).bind(&host)
            .bind(&repo_path).bind(&default_branch).bind(&credential_ref).bind(&options_json).bind(now)
            .fetch_one(&state.db).await;
            let conn_row = match conn_row {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("mcp provision conn: {e}");
                    return err_result("database_error");
                }
            };
            let conn_id: i32 = conn_row.get("id");

            // Upsert RepoBinding
            let bind_name = bind_input
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if bind_name.is_empty() {
                return err_result("binding.name is required");
            }
            let branch_override = bind_input
                .get("branch_override")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let base_path = bind_input
                .get("base_path")
                .and_then(|v| v.as_str())
                .unwrap_or("domains")
                .trim_matches('/')
                .to_string();
            let bind_active = bind_input
                .get("active")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let bind_row = sqlx::query(
                "INSERT INTO repobinding (owner_subject, name, connection_id, branch_override, base_path, active, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$7) \
                 ON CONFLICT (owner_subject, name) DO UPDATE SET \
                 connection_id=$3, branch_override=$4, base_path=$5, active=$6, updated_at=$7 \
                 RETURNING *"
            )
            .bind(&principal.subject).bind(&bind_name).bind(conn_id)
            .bind(&branch_override).bind(&base_path).bind(bind_active).bind(now)
            .fetch_one(&state.db).await;
            let bind_row = match bind_row {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("mcp provision binding: {e}");
                    return err_result("database_error");
                }
            };
            let bind_id: i32 = bind_row.get("id");

            // Upsert DomainPersistencePolicy
            let fallback_mode = str_arg_or(args, "fallback_mode", "fail_closed");
            let require_approval = args
                .get("require_approval")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let _ = sqlx::query(
                "INSERT INTO domainpersistencepolicy (domain_id, default_binding_id, fallback_mode, require_approval, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$5) \
                 ON CONFLICT (domain_id) DO UPDATE SET \
                 default_binding_id=$2, fallback_mode=$3, require_approval=$4, updated_at=$5"
            )
            .bind(&domain_id).bind(bind_id).bind(&fallback_mode).bind(require_approval).bind(now)
            .execute(&state.db).await;

            // Replace routes
            let _ = sqlx::query("DELETE FROM domainpersistenceroute WHERE domain_id=$1")
                .bind(&domain_id)
                .execute(&state.db)
                .await;

            for route in &routes_input {
                let target_name = route
                    .get("binding_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&bind_name);
                let target_id: Option<i32> = if target_name == bind_name {
                    Some(bind_id)
                } else {
                    sqlx::query_scalar(
                        "SELECT id FROM repobinding WHERE owner_subject=$1 AND name=$2",
                    )
                    .bind(&principal.subject)
                    .bind(target_name)
                    .fetch_optional(&state.db)
                    .await
                    .unwrap_or(None)
                };
                let Some(tid) = target_id else {
                    continue;
                };
                let entity_kind = route
                    .get("entity_kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if entity_kind.is_empty() {
                    continue;
                }
                let event_kind = route
                    .get("event_kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let route_branch = route
                    .get("branch_override")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let path_tpl = route
                    .get("path_template")
                    .and_then(|v| v.as_str())
                    .unwrap_or("domains/{domain_id}/{entity_kind}/{entity_id}.json");
                let format = route
                    .get("format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("json_v1");
                let active = route
                    .get("active")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let _ = sqlx::query(
                    "INSERT INTO domainpersistenceroute \
                     (domain_id, entity_kind, event_kind, binding_id, branch_override, path_template, format, active, created_at, updated_at) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9)"
                )
                .bind(&domain_id).bind(entity_kind).bind(event_kind).bind(tid)
                .bind(route_branch).bind(path_tpl).bind(format).bind(active).bind(now)
                .execute(&state.db).await;
            }

            let routes = sqlx::query(
                "SELECT * FROM domainpersistenceroute WHERE domain_id=$1 AND active=true ORDER BY id ASC"
            )
            .bind(&domain_id).fetch_all(&state.db).await.unwrap_or_default();

            ok_result(json!({
                "ok": true,
                "domain_id": domain_id,
                "connection": {
                    "id": conn_row.get::<i32,_>("id"),
                    "owner_subject": conn_row.get::<String,_>("owner_subject"),
                    "name": conn_row.get::<String,_>("name"),
                    "provider": conn_row.get::<String,_>("provider"),
                    "host": conn_row.get::<String,_>("host"),
                    "repo_path": conn_row.get::<String,_>("repo_path"),
                    "default_branch": conn_row.get::<String,_>("default_branch"),
                    "credential_ref": conn_row.get::<String,_>("credential_ref"),
                    "created_at": conn_row.get::<chrono::NaiveDateTime,_>("created_at"),
                    "updated_at": conn_row.get::<chrono::NaiveDateTime,_>("updated_at"),
                },
                "binding": {
                    "id": bind_row.get::<i32,_>("id"),
                    "owner_subject": bind_row.get::<String,_>("owner_subject"),
                    "name": bind_row.get::<String,_>("name"),
                    "connection_id": bind_row.get::<i32,_>("connection_id"),
                    "branch_override": bind_row.get::<String,_>("branch_override"),
                    "base_path": bind_row.get::<String,_>("base_path"),
                    "active": bind_row.get::<bool,_>("active"),
                    "created_at": bind_row.get::<chrono::NaiveDateTime,_>("created_at"),
                    "updated_at": bind_row.get::<chrono::NaiveDateTime,_>("updated_at"),
                },
                "routes": routes.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "domain_id": r.get::<String,_>("domain_id"),
                    "entity_kind": r.get::<String,_>("entity_kind"),
                    "event_kind": r.get::<String,_>("event_kind"),
                    "binding_id": r.get::<i32,_>("binding_id"),
                    "branch_override": r.get::<String,_>("branch_override"),
                    "path_template": r.get::<String,_>("path_template"),
                    "format": r.get::<String,_>("format"),
                    "active": r.get::<bool,_>("active"),
                })).collect::<Vec<_>>(),
            }))
        }

        // ── Git ledger publish ────────────────────────────────────────────────
        "publish_pending_ledger_events" => {
            let domain_id = str_arg(args, "domain_id");
            if domain_id.is_empty() {
                return err_result("domain_id is required");
            }
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }

            // Fetch pending events
            let events = sqlx::query(
                "SELECT * FROM ledgerevent WHERE domain_id=$1 AND state='pending' \
                 ORDER BY created_at ASC LIMIT 500",
            )
            .bind(&domain_id)
            .fetch_all(&state.db)
            .await;
            let events = match events {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!("mcp publish_ledger fetch: {e}");
                    return err_result("database_error");
                }
            };
            if events.is_empty() {
                return ok_result(
                    json!({"published_count": 0, "commit_sha": "", "branch": "", "repo_url": ""}),
                );
            }

            // Get routing: binding + connection
            let route = sqlx::query(
                "SELECT r.path_template, r.format, r.event_kind, \
                 b.branch_override, b.base_path, \
                 c.host, c.repo_path, c.default_branch, c.credential_ref, c.provider \
                 FROM domainpersistenceroute r \
                 JOIN repobinding b ON b.id = r.binding_id \
                 JOIN repoconnection c ON c.id = b.connection_id \
                 WHERE r.domain_id=$1 AND r.active=true \
                 ORDER BY r.id ASC LIMIT 1",
            )
            .bind(&domain_id)
            .fetch_optional(&state.db)
            .await;
            let route = match route {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("no publish route configured for domain"),
                Err(e) => {
                    tracing::error!("mcp publish_ledger route: {e}");
                    return err_result("database_error");
                }
            };

            let host: String = route.get("host");
            let repo_path: String = route.get("repo_path");
            let default_branch: String = route.get("default_branch");
            let credential_ref: String = route.get("credential_ref");
            let branch: String = route
                .try_get("branch_override")
                .ok()
                .filter(|s: &String| !s.is_empty())
                .unwrap_or_else(|| default_branch.clone());
            let path_tpl: String = route.get("path_template");

            // Resolve credential: "env:VAR_NAME" → token from env
            let token = if let Some(var_name) = credential_ref.strip_prefix("env:") {
                std::env::var(var_name).unwrap_or_default()
            } else {
                std::env::var("GIT_PUBLISH_TOKEN").unwrap_or_default()
            };

            let repo_url = if token.is_empty() {
                format!("https://{host}/{repo_path}")
            } else {
                format!("https://x-access-token:{token}@{host}/{repo_path}")
            };

            // Clone to tempdir and write files
            let tmpdir = match tempfile::TempDir::new() {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("mcp publish_ledger tempdir: {e}");
                    return err_result("internal_error");
                }
            };
            let repo_dir = tmpdir.path().to_string_lossy().to_string();

            let clone_out = std::process::Command::new("git")
                .args([
                    "clone",
                    "--depth=1",
                    "--branch",
                    &branch,
                    &repo_url,
                    &repo_dir,
                ])
                .output();
            if let Err(e) = clone_out {
                tracing::error!("mcp publish_ledger clone: {e}");
                return err_result("git_clone_failed");
            }

            // Write entity files
            for event in &events {
                let entity_type: String = event.get("entity_type");
                let entity_id: String = event.get("entity_id");
                let payload: String = event.try_get("payload_json").unwrap_or_default();
                let rel = path_tpl
                    .replace("{domain_id}", &domain_id)
                    .replace("{entity_kind}", &entity_type)
                    .replace("{entity_id}", &entity_id);
                let full_path = std::path::Path::new(&repo_dir).join(&rel);
                if let Some(parent) = full_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&full_path, payload.as_bytes());
                let _ = std::process::Command::new("git")
                    .args(["-C", &repo_dir, "add", &rel])
                    .output();
            }

            // Commit and push
            let commit_msg = format!(
                "edgeplane-tower: publish {} ledger events for {}",
                events.len(),
                domain_id
            );
            let _ = std::process::Command::new("git")
                .args([
                    "-C",
                    &repo_dir,
                    "config",
                    "user.email",
                    "edgeplane-tower@localhost",
                ])
                .output();
            let _ = std::process::Command::new("git")
                .args(["-C", &repo_dir, "config", "user.name", "edgeplane-tower"])
                .output();
            let commit_out = std::process::Command::new("git")
                .args([
                    "-C",
                    &repo_dir,
                    "commit",
                    "--allow-empty",
                    "-m",
                    &commit_msg,
                ])
                .output();
            let commit_sha = if let Ok(_out) = commit_out {
                // Extract SHA from "git rev-parse HEAD"
                std::process::Command::new("git")
                    .args(["-C", &repo_dir, "rev-parse", "HEAD"])
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let push_out = std::process::Command::new("git")
                .args(["-C", &repo_dir, "push", "origin", &branch])
                .output();
            if let Err(e) = push_out {
                tracing::error!("mcp publish_ledger push: {e}");
            }

            // Update ledger events state
            let published_count = events.len() as i64;
            for event in &events {
                let eid: i32 = event.get("id");
                let entity_type: String = event.get("entity_type");
                let entity_id: String = event.get("entity_id");
                let rel = path_tpl
                    .replace("{domain_id}", &domain_id)
                    .replace("{entity_kind}", &entity_type)
                    .replace("{entity_id}", &entity_id);
                let _ = sqlx::query(
                    "UPDATE ledgerevent SET state='published', git_commit=$2, git_path=$3, published_at=$4, updated_at=$4 WHERE id=$1"
                )
                .bind(eid).bind(&commit_sha).bind(&rel).bind(now)
                .execute(&state.db).await;
            }

            let clean_repo_url = format!("https://{host}/{repo_path}");
            ok_result(
                json!({"published_count": published_count, "commit_sha": commit_sha, "branch": branch, "repo_url": clean_repo_url}),
            )
        }

        // ── get_artifact_download_url — SigV4 presigned S3 URL ──────────────────
        "get_artifact_download_url" => {
            let artifact_id = int_arg(args, "artifact_id").unwrap_or(0) as i32;
            let expires: u64 = int_arg(args, "expires_seconds")
                .unwrap_or(60)
                .clamp(1, 3600) as u64;
            if artifact_id <= 0 {
                return err_result("artifact_id is required");
            }

            // Look up artifact
            let artifact = sqlx::query("SELECT * FROM artifact WHERE id=$1")
                .bind(artifact_id)
                .fetch_optional(&state.db)
                .await;
            let artifact = match artifact {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Artifact not found"),
                Err(e) => {
                    tracing::error!("get_artifact_download_url artifact: {e}");
                    return err_result("database_error");
                }
            };
            // ── Domain authz: resolve mission → domain, then check membership ──
            let art_mission_id: String = artifact.try_get("mission_id").unwrap_or_default();
            let domain_id = match crate::routes::authz::domain_id_for_mission(&state.db, &art_mission_id).await {
                Ok(d) => d,
                Err(_) => return err_result("Artifact mission not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let storage_backend: String = artifact.try_get("storage_backend").unwrap_or_default();
            let uri: String = artifact.try_get("uri").unwrap_or_default();
            if storage_backend != "s3" || !uri.starts_with("s3://") {
                return err_result("Artifact does not have retrievable S3-backed content");
            }

            // Parse s3://bucket/key
            let rest = match uri.strip_prefix("s3://") {
                Some(r) => r,
                None => return err_result("uri must be an s3:// URI"),
            };
            let (bucket, key) = match rest.split_once('/') {
                Some((b, k)) if !b.is_empty() && !k.is_empty() => (b.to_string(), k.to_string()),
                _ => return err_result("invalid s3 URI"),
            };

            // Read config from env
            let endpoint = std::env::var("EP_OBJECT_STORAGE_ENDPOINT").unwrap_or_default();
            let region =
                std::env::var("EP_OBJECT_STORAGE_REGION").unwrap_or_else(|_| "us-east-1".into());
            let access_key = std::env::var("EP_OBJECT_STORAGE_ACCESS_KEY")
                .or_else(|_| std::env::var("EP_OBJECT_STORAGE_KEY"))
                .unwrap_or_default();
            let secret_key = std::env::var("EP_OBJECT_STORAGE_ACCESS_SECRET")
                .or_else(|_| std::env::var("EP_OBJECT_STORAGE_SECRET"))
                .unwrap_or_default();

            if access_key.is_empty() || secret_key.is_empty() {
                return err_result("object storage not configured (missing access key or secret)");
            }

            // Build SigV4 presigned URL
            let now = chrono::Utc::now();
            let date_stamp = now.format("%Y%m%d").to_string();
            let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

            // Determine host and path style
            // Custom endpoint → path-style (MinIO); no endpoint → virtual-hosted (AWS)
            let (host, canonical_path, url_path) = if endpoint.is_empty() {
                // AWS virtual-hosted: bucket in hostname
                let h = format!("{bucket}.s3.{region}.amazonaws.com");
                let cp = format!("/{}", uri_encode_path(&key));
                let up = format!("/{key}");
                (h, cp, up)
            } else {
                let stripped = endpoint
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_end_matches('/');
                let h = stripped.to_string();
                let cp = format!("/{}/{}", uri_encode_path(&bucket), uri_encode_path(&key));
                let up = format!("/{bucket}/{key}");
                (h, cp, up)
            };

            let scope = format!("{date_stamp}/{region}/s3/aws4_request");
            let algorithm = "AWS4-HMAC-SHA256";
            let credential = format!("{access_key}/{scope}");

            // Canonical query string — params sorted lexicographically
            let signed_headers = "host";
            let mut qparams = [
                ("X-Amz-Algorithm", algorithm.to_string()),
                ("X-Amz-Credential", credential.clone()),
                ("X-Amz-Date", amz_date.clone()),
                ("X-Amz-Expires", expires.to_string()),
                ("X-Amz-SignedHeaders", signed_headers.to_string()),
            ];
            qparams.sort_by(|a, b| a.0.cmp(b.0));
            let canonical_qs: String = qparams
                .iter()
                .map(|(k, v)| format!("{}={}", uri_encode(k), uri_encode(v)))
                .collect::<Vec<_>>()
                .join("&");

            let canonical_headers = format!("host:{host}\n");
            let payload_hash = "UNSIGNED-PAYLOAD";
            let canonical_request = format!(
                "GET\n{canonical_path}\n{canonical_qs}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
            );

            use sha2::Digest;
            let cr_hash = hex::encode(sha2::Sha256::digest(canonical_request.as_bytes()));
            let string_to_sign = format!("{algorithm}\n{amz_date}\n{scope}\n{cr_hash}");

            let signing_key = sigv4_signing_key(&secret_key, &date_stamp, &region, "s3");
            use hmac::Mac;
            let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&signing_key).unwrap();
            mac.update(string_to_sign.as_bytes());
            let signature = hex::encode(mac.finalize().into_bytes());

            let scheme = if std::env::var("EP_OBJECT_STORAGE_SECURE")
                .map(|v| v.to_lowercase() == "false" || v == "0")
                .unwrap_or(false)
            {
                "http"
            } else {
                "https"
            };
            let presigned_url =
                format!("{scheme}://{host}{url_path}?{canonical_qs}&X-Amz-Signature={signature}");

            ok_result(json!({"url": presigned_url, "expires_seconds": expires}))
        }

        // ── Workspace leases ──────────────────────────────────────────────────
        "load_mission_workspace" => {
            let mission_id = str_arg(args, "mission_id");
            if mission_id.is_empty() {
                return err_result("mission_id is required");
            }
            let lease_seconds = int_arg(args, "lease_seconds")
                .unwrap_or(900)
                .clamp(60, 3600) as i32;
            let workspace_label = str_arg(args, "workspace_label");
            let agent_id = str_arg(args, "agent_id");

            // Verify mission exists and get its domain_id
            let mission = sqlx::query("SELECT id, domain_id FROM mission WHERE id=$1")
                .bind(&mission_id)
                .fetch_optional(&state.db)
                .await;
            let mission = match mission {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Mission not found"),
                Err(e) => {
                    tracing::error!("mcp load_mission_workspace mission: {e}");
                    return err_result("database_error");
                }
            };
            let domain_id: Option<String> = mission.get("domain_id");
            let domain_id = match domain_id {
                Some(m) if !m.is_empty() => m,
                _ => return err_result("Mission is not linked to a domain"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }

            // Build workspace snapshot from DB
            let snapshot = build_workspace_snapshot(&state.db, &domain_id, &mission_id).await;

            // Create lease
            let lease_id = format!(
                "wl-{}",
                &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
            );
            let expires_at = now + chrono::Duration::seconds(lease_seconds as i64);
            let snapshot_index =
                serde_json::to_string(&snapshot.get("index").cloned().unwrap_or(json!({})))
                    .unwrap_or_default();

            let lease = sqlx::query(
                "INSERT INTO workspacelease \
                 (id, domain_id, mission_id, actor_subject, agent_id, workspace_label, \
                  status, base_snapshot_json, lease_seconds, last_heartbeat_at, expires_at, \
                  release_reason, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,'active',$7,$8,$9,$10,'',$9,$9) RETURNING *",
            )
            .bind(&lease_id)
            .bind(&domain_id)
            .bind(&mission_id)
            .bind(&principal.subject)
            .bind(&agent_id)
            .bind(&workspace_label)
            .bind(&snapshot_index)
            .bind(lease_seconds)
            .bind(now)
            .bind(expires_at)
            .fetch_one(&state.db)
            .await;

            match lease {
                Ok(r) => ok_result(json!({
                    "lease": lease_row_to_json(&r),
                    "workspace_snapshot": snapshot,
                })),
                Err(e) => {
                    tracing::error!("mcp load_mission_workspace insert: {e}");
                    err_result("database_error")
                }
            }
        }

        "heartbeat_workspace_lease" => {
            let lease_id = str_arg(args, "lease_id");
            if lease_id.is_empty() {
                return err_result("lease_id is required");
            }

            let lease = sqlx::query("SELECT * FROM workspacelease WHERE id=$1")
                .bind(&lease_id)
                .fetch_optional(&state.db)
                .await;
            let lease = match lease {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Workspace lease not found"),
                Err(e) => {
                    tracing::error!("mcp heartbeat lease: {e}");
                    return err_result("database_error");
                }
            };

            let owner: String = lease.get("actor_subject");
            if owner != principal.subject && !principal.is_admin {
                return err_result("forbidden");
            }
            let status: String = lease.get("status");
            if status != "active" {
                return err_result("Workspace lease is not active");
            }
            let lease_seconds: i32 = lease.try_get("lease_seconds").unwrap_or(900);
            let new_expires = now + chrono::Duration::seconds(lease_seconds as i64);

            match sqlx::query(
                "UPDATE workspacelease SET last_heartbeat_at=$2, expires_at=$3, updated_at=$2 WHERE id=$1 RETURNING *"
            )
            .bind(&lease_id).bind(now).bind(new_expires)
            .fetch_one(&state.db).await {
                Ok(r) => ok_result(json!({"lease": {
                    "id": r.get::<String,_>("id"),
                    "status": r.get::<String,_>("status"),
                    "last_heartbeat_at": r.get::<chrono::NaiveDateTime,_>("last_heartbeat_at"),
                    "expires_at": r.get::<chrono::NaiveDateTime,_>("expires_at"),
                }})),
                Err(e) => { tracing::error!("mcp heartbeat update: {e}"); err_result("database_error") }
            }
        }

        "fetch_workspace_artifact" => {
            let lease_id = str_arg(args, "lease_id");
            let artifact_id = int_arg(args, "artifact_id").unwrap_or(0) as i32;
            let mode = str_arg_or(args, "mode", "content");
            if lease_id.is_empty() {
                return err_result("lease_id is required");
            }
            if artifact_id <= 0 {
                return err_result("artifact_id is required");
            }

            let lease = sqlx::query("SELECT * FROM workspacelease WHERE id=$1")
                .bind(&lease_id)
                .fetch_optional(&state.db)
                .await;
            let lease = match lease {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Workspace lease not found"),
                Err(e) => {
                    tracing::error!("mcp fetch_workspace_artifact lease: {e}");
                    return err_result("database_error");
                }
            };
            let owner: String = lease.get("actor_subject");
            if owner != principal.subject && !principal.is_admin {
                return err_result("forbidden");
            }
            let lease_status: String = lease.get("status");
            if lease_status != "active" {
                return err_result("Workspace lease is not active");
            }
            let lease_mission: String = lease.get("mission_id");

            let artifact = sqlx::query("SELECT * FROM artifact WHERE id=$1")
                .bind(artifact_id)
                .fetch_optional(&state.db)
                .await;
            let artifact = match artifact {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Artifact not found"),
                Err(e) => {
                    tracing::error!("mcp fetch_workspace_artifact artifact: {e}");
                    return err_result("database_error");
                }
            };
            let art_mission: String = artifact.get("mission_id");
            if art_mission != lease_mission {
                return err_result("Artifact is outside lease mission scope");
            }

            let storage_backend: String = artifact.try_get("storage_backend").unwrap_or_default();
            let content_b64: Option<String> = artifact.try_get("content_b64").ok().flatten();
            let _uri: String = artifact.try_get("uri").unwrap_or_default();
            let mime_type: String = artifact.try_get("mime_type").unwrap_or_default();

            if mode == "content" {
                if storage_backend == "s3" {
                    return err_result("S3 artifact content fetch requires Python API");
                }
                match content_b64 {
                    Some(b64) if !b64.is_empty() => {
                        use base64::Engine;
                        let size = base64::engine::general_purpose::STANDARD
                            .decode(b64.as_bytes())
                            .map(|b| b.len())
                            .unwrap_or(0);
                        ok_result(json!({
                            "artifact_id": artifact_id,
                            "mode": "content",
                            "mime_type": mime_type,
                            "size_bytes": size,
                            "content_b64": b64,
                        }))
                    }
                    _ => err_result("Artifact does not have retrievable inline content"),
                }
            } else {
                // download_url mode
                if storage_backend != "s3" {
                    return err_result(
                        "Artifact is not S3-backed — use content mode or Python API for download URL",
                    );
                }
                err_result("S3 presigned URL generation requires Python API")
            }
        }

        "commit_mission_workspace" => {
            let lease_id = str_arg(args, "lease_id");
            if lease_id.is_empty() {
                return err_result("lease_id is required");
            }
            let changes = match args.get("change_set").and_then(|v| v.as_array()) {
                Some(c) if !c.is_empty() => c.clone(),
                _ => return err_result("change_set must be a non-empty array"),
            };

            let lease = sqlx::query("SELECT * FROM workspacelease WHERE id=$1")
                .bind(&lease_id)
                .fetch_optional(&state.db)
                .await;
            let lease = match lease {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Workspace lease not found"),
                Err(e) => {
                    tracing::error!("mcp commit_workspace lease: {e}");
                    return err_result("database_error");
                }
            };
            let owner: String = lease.get("actor_subject");
            if owner != principal.subject && !principal.is_admin {
                return err_result("forbidden");
            }
            let lease_status: String = lease.get("status");
            if lease_status != "active" {
                return err_result("Workspace lease is not active");
            }
            let mission_id: String = lease.get("mission_id");
            let domain_id: String = lease.get("domain_id");

            let mut applied_count = 0i64;
            let mut applied: Vec<serde_json::Value> = vec![];

            for change in &changes {
                let entity_type = change
                    .get("entity_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let entity_id = change
                    .get("entity_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                match entity_type {
                    "doc" => {
                        let doc_id: i32 = match entity_id.parse() {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let doc = sqlx::query("SELECT * FROM doc WHERE id=$1 AND mission_id=$2")
                            .bind(doc_id)
                            .bind(&mission_id)
                            .fetch_optional(&state.db)
                            .await;
                        let doc = match doc {
                            Ok(Some(r)) => r,
                            _ => continue,
                        };
                        let mut sets: Vec<String> = vec![];
                        let mut new_title: Option<String> = None;
                        let mut new_body: Option<String> = None;
                        let mut new_doc_type: Option<String> = None;
                        let mut new_status: Option<String> = None;
                        if let Some(v) = change.get("content").and_then(|v| v.as_str()) {
                            new_body = Some(v.to_string());
                            sets.push("body=$IDX".to_string());
                        }
                        if let Some(v) = change.get("title").and_then(|v| v.as_str()) {
                            new_title = Some(v.to_string());
                            sets.push("title=$IDX".to_string());
                        }
                        if let Some(v) = change.get("doc_type").and_then(|v| v.as_str()) {
                            new_doc_type = Some(v.to_string());
                            sets.push("doc_type=$IDX".to_string());
                        }
                        if let Some(v) = change.get("status").and_then(|v| v.as_str()) {
                            new_status = Some(v.to_string());
                            sets.push("status=$IDX".to_string());
                        }
                        if sets.is_empty() {
                            continue;
                        }
                        let cur_version: i32 = doc.try_get("version").unwrap_or(1);
                        let sql = format!(
                            "UPDATE doc SET version={}, updated_at=$1, {} WHERE id=$2",
                            cur_version + 1,
                            sets.iter()
                                .enumerate()
                                .map(|(i, s)| s.replace("$IDX", &format!("${}", i + 3)))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        let mut q = sqlx::query(&sql).bind(now).bind(doc_id);
                        if let Some(v) = &new_body {
                            q = q.bind(v);
                        }
                        if let Some(v) = &new_title {
                            q = q.bind(v);
                        }
                        if let Some(v) = &new_doc_type {
                            q = q.bind(v);
                        }
                        if let Some(v) = &new_status {
                            q = q.bind(v);
                        }
                        let _ = q.execute(&state.db).await;
                        applied.push(json!({"entity_type": "doc", "entity_id": doc_id, "version": cur_version + 1}));
                        applied_count += 1;
                    }
                    "artifact" => {
                        let art_id: i32 = match entity_id.parse() {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        let art =
                            sqlx::query("SELECT * FROM artifact WHERE id=$1 AND mission_id=$2")
                                .bind(art_id)
                                .bind(&mission_id)
                                .fetch_optional(&state.db)
                                .await;
                        let art = match art {
                            Ok(Some(r)) => r,
                            _ => continue,
                        };
                        let cur_version: i32 = art.try_get("version").unwrap_or(1);
                        // Only field updates — skip S3 content_b64 upload
                        let fields = change.get("fields").and_then(|v| v.as_object());
                        let mut parts: Vec<String> = vec![];
                        let mut vals: Vec<String> = vec![];
                        for key in [
                            "name",
                            "artifact_type",
                            "uri",
                            "storage_backend",
                            "content_sha256",
                            "mime_type",
                            "status",
                            "provenance",
                        ] {
                            if let Some(v) =
                                fields.and_then(|f| f.get(key)).and_then(|v| v.as_str())
                            {
                                parts.push(format!("{}=$IDX", key));
                                vals.push(v.to_string());
                            }
                        }
                        let sql = format!(
                            "UPDATE artifact SET version={}, updated_at=$1, {} WHERE id=$2",
                            cur_version + 1,
                            parts
                                .iter()
                                .enumerate()
                                .map(|(i, s)| s.replace("$IDX", &format!("${}", i + 3)))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        if !parts.is_empty() {
                            let mut q = sqlx::query(&sql).bind(now).bind(art_id);
                            for v in &vals {
                                q = q.bind(v);
                            }
                            let _ = q.execute(&state.db).await;
                        }
                        applied.push(json!({"entity_type": "artifact", "entity_id": art_id, "version": cur_version + 1}));
                        applied_count += 1;
                    }
                    _ => {}
                }

                // Enqueue ledger event
                let event_id = format!(
                    "le-{}",
                    &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
                );
                let _ = sqlx::query(
                    "INSERT INTO ledgerevent (event_id, domain_id, mission_id, entity_type, entity_id, \
                     action, payload_json, state, created_by_subject, created_at, updated_at) \
                     VALUES ($1,$2,$3,$4,$5,'workspace_commit','{}'::text,'pending',$6,$7,$7)"
                )
                .bind(&event_id).bind(&domain_id).bind(&mission_id)
                .bind(entity_type).bind(entity_id).bind(&principal.subject).bind(now)
                .execute(&state.db).await;
            }

            let snapshot = build_workspace_snapshot(&state.db, &domain_id, &mission_id).await;
            ok_result(
                json!({"applied_count": applied_count, "applied": applied, "workspace_snapshot": snapshot}),
            )
        }

        "release_mission_workspace" => {
            let lease_id = str_arg(args, "lease_id");
            let reason = str_arg(args, "reason");
            if lease_id.is_empty() {
                return err_result("lease_id is required");
            }

            let lease = sqlx::query("SELECT * FROM workspacelease WHERE id=$1")
                .bind(&lease_id)
                .fetch_optional(&state.db)
                .await;
            let lease = match lease {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Workspace lease not found"),
                Err(e) => {
                    tracing::error!("mcp release_workspace lease: {e}");
                    return err_result("database_error");
                }
            };
            let owner: String = lease.get("actor_subject");
            if owner != principal.subject && !principal.is_admin {
                return err_result("forbidden");
            }
            let current_status: String = lease.get("status");
            if current_status == "released" || current_status == "expired" {
                return ok_result(json!({"lease": lease_row_to_json(&lease)}));
            }

            match sqlx::query(
                "UPDATE workspacelease SET status='released', release_reason=$2, released_at=$3, updated_at=$3 WHERE id=$1 RETURNING *"
            )
            .bind(&lease_id).bind(&reason).bind(now)
            .fetch_one(&state.db).await {
                Ok(r) => ok_result(json!({"lease": {
                    "id": r.get::<String,_>("id"),
                    "status": r.get::<String,_>("status"),
                    "release_reason": r.get::<String,_>("release_reason"),
                    "released_at": r.get::<Option<chrono::NaiveDateTime>,_>("released_at"),
                }})),
                Err(e) => { tracing::error!("mcp release_workspace update: {e}"); err_result("database_error") }
            }
        }

        // ── Domain context ────────────────────────────────────────────────────
        "get_domain_northstar" => {
            let domain_id = str_arg(args, "domain_id");
            if domain_id.is_empty() {
                return err_result("domain_id is required");
            }
            // Change 2: authz before the SELECT — northstar_md is sensitive strategy content.
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let row =
                sqlx::query("SELECT northstar_md, northstar_version FROM domain WHERE id = $1")
                    .bind(&domain_id)
                    .fetch_optional(&state.db)
                    .await;
            match row {
                Ok(Some(r)) => {
                    let content: String = r.get("northstar_md");
                    let version: i32 = r.get("northstar_version");
                    ok_result(
                        json!({ "domain_id": domain_id, "content": content, "version": version }),
                    )
                }
                Ok(None) => err_result(&format!("domain '{}' not found", domain_id)),
                Err(e) => {
                    tracing::error!("mcp get_domain_northstar: {e}");
                    err_result("database_error")
                }
            }
        }

        _ => err_result("unknown_tool"),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn str_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn str_arg_or(args: &Value, key: &str, default: &str) -> String {
    let v = args
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if v.is_empty() { default.to_string() } else { v }
}

fn int_arg(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

fn lease_row_to_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<String,_>("id"),
        "domain_id": r.get::<String,_>("domain_id"),
        "mission_id": r.get::<String,_>("mission_id"),
        "actor_subject": r.get::<String,_>("actor_subject"),
        "agent_id": r.get::<String,_>("agent_id"),
        "workspace_label": r.get::<String,_>("workspace_label"),
        "status": r.get::<String,_>("status"),
        "lease_seconds": r.get::<i32,_>("lease_seconds"),
        "last_heartbeat_at": r.get::<chrono::NaiveDateTime,_>("last_heartbeat_at"),
        "expires_at": r.get::<chrono::NaiveDateTime,_>("expires_at"),
    })
}

async fn build_workspace_snapshot(db: &sqlx::PgPool, domain_id: &str, mission_id: &str) -> Value {
    let tasks = sqlx::query(
        "SELECT id, title, description, status, priority, claimed_by_agent_id, claim_policy, updated_at \
         FROM meshtask WHERE mission_id=$1 AND status NOT IN ('finished','cancelled') ORDER BY updated_at DESC LIMIT 200"
    )
    .bind(mission_id).fetch_all(db).await.unwrap_or_default();

    let docs = sqlx::query(
        "SELECT id, title, doc_type, status, version, updated_at FROM doc WHERE mission_id=$1 ORDER BY updated_at DESC LIMIT 100"
    )
    .bind(mission_id).fetch_all(db).await.unwrap_or_default();

    let artifacts = sqlx::query(
        "SELECT id, name, artifact_type, uri, storage_backend, mime_type, size_bytes, status, version, updated_at \
         FROM artifact WHERE mission_id=$1 ORDER BY updated_at DESC LIMIT 100"
    )
    .bind(mission_id).fetch_all(db).await.unwrap_or_default();

    // Build version index for conflict detection (stored in base_snapshot_json)
    let mut index = serde_json::Map::new();
    for r in &docs {
        let id: i32 = r.get("id");
        let ver: i32 = r.try_get("version").unwrap_or(1);
        index.insert(format!("doc:{id}"), json!(ver));
    }
    for r in &artifacts {
        let id: i32 = r.get("id");
        let ver: i32 = r.try_get("version").unwrap_or(1);
        index.insert(format!("artifact:{id}"), json!(ver));
    }

    json!({
        "domain_id": domain_id,
        "mission_id": mission_id,
        "tasks": tasks.iter().map(|r| json!({
            "id": r.get::<String,_>("id"),
            "title": r.get::<String,_>("title"),
            "description": r.try_get::<String,_>("description").unwrap_or_default(),
            "status": r.get::<String,_>("status"),
            "priority": r.get::<i32,_>("priority"),
            "claimed_by_agent_id": r.try_get::<String,_>("claimed_by_agent_id").unwrap_or_default(),
            "claim_policy": r.get::<String,_>("claim_policy"),
            "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
        })).collect::<Vec<_>>(),
        "docs": docs.iter().map(|r| json!({
            "id": r.get::<i32,_>("id"),
            "title": r.get::<String,_>("title"),
            "doc_type": r.get::<String,_>("doc_type"),
            "status": r.get::<String,_>("status"),
            "version": r.try_get::<i32,_>("version").unwrap_or(1),
            "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
        })).collect::<Vec<_>>(),
        "artifacts": artifacts.iter().map(|r| json!({
            "id": r.get::<i32,_>("id"),
            "name": r.get::<String,_>("name"),
            "artifact_type": r.try_get::<String,_>("artifact_type").unwrap_or_default(),
            "storage_backend": r.try_get::<String,_>("storage_backend").unwrap_or_default(),
            "mime_type": r.try_get::<String,_>("mime_type").unwrap_or_default(),
            "size_bytes": r.try_get::<i32,_>("size_bytes").unwrap_or(0),
            "status": r.try_get::<String,_>("status").unwrap_or_default(),
            "version": r.try_get::<i32,_>("version").unwrap_or(1),
            "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
        })).collect::<Vec<_>>(),
        "index": index,
    })
}
