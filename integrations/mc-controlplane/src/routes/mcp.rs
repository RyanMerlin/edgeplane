use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/mcp/tools", get(list_tools))
        .route("/mcp/health", get(mcp_health))
        .route("/mcp/call", post(call_tool))
}

async fn mcp_health() -> impl IntoResponse {
    Json(json!({"ok": true, "version": "mc-controlplane"}))
}

fn tool_def(name: &str, description: &str, schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": schema})
}

async fn list_tools() -> impl IntoResponse {
    let tools = vec![
        tool_def("search_klusters", "Search klusters by name or tag", json!({"type":"object","properties":{"query":{"type":"string"}}})),
        tool_def("search_tasks", "Search tasks by semantic similarity", json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer"}}})),
        tool_def("read_doc", "Read a doc by id", json!({"type":"object","properties":{"doc_id":{"type":"integer"}}})),
        tool_def("create_doc", "Create a doc in a kluster", json!({"type":"object","properties":{"kluster_id":{"type":"string"},"title":{"type":"string"},"body":{"type":"string"},"doc_type":{"type":"string"},"status":{"type":"string"},"provenance":{"type":"string"}}})),
        tool_def("create_artifact", "Create an artifact in a kluster", json!({"type":"object","properties":{"kluster_id":{"type":"string"},"name":{"type":"string"},"artifact_type":{"type":"string"},"uri":{"type":"string"},"content_b64":{"type":"string"},"storage_backend":{"type":"string"},"storage_class":{"type":"string"},"content_sha256":{"type":"string"},"size_bytes":{"type":"integer"},"mime_type":{"type":"string"},"external_pointer":{"type":"boolean"},"external_uri":{"type":"string"},"status":{"type":"string"},"provenance":{"type":"string"}}})),
        tool_def("get_artifact_download_url", "Get a short-lived download URL for an S3-backed artifact", json!({"type":"object","properties":{"artifact_id":{"type":"integer"},"expires_seconds":{"type":"integer"}}})),
        tool_def("load_kluster_workspace", "Load/sync a kluster workspace and acquire a lease", json!({"type":"object","properties":{"kluster_id":{"type":"string"},"workspace_label":{"type":"string"},"agent_id":{"type":"string"},"lease_seconds":{"type":"integer"}},"required":["kluster_id"]})),
        tool_def("heartbeat_workspace_lease", "Extend a workspace lease heartbeat", json!({"type":"object","properties":{"lease_id":{"type":"string"}},"required":["lease_id"]})),
        tool_def("fetch_workspace_artifact", "Fetch artifact bytes or signed download URL while a lease is active", json!({"type":"object","properties":{"lease_id":{"type":"string"},"artifact_id":{"type":"integer"},"mode":{"type":"string"},"expires_seconds":{"type":"integer"}},"required":["lease_id","artifact_id"]})),
        tool_def("commit_kluster_workspace", "Commit workspace changes with optimistic conflict checks", json!({"type":"object","properties":{"lease_id":{"type":"string"},"change_set":{"type":"array"},"validation_mode":{"type":"string"}},"required":["lease_id","change_set"]})),
        tool_def("release_kluster_workspace", "Release an active workspace lease", json!({"type":"object","properties":{"lease_id":{"type":"string"},"reason":{"type":"string"}},"required":["lease_id"]})),
        tool_def("create_task", "Create a task in a kluster", json!({"type":"object","properties":{"kluster_id":{"type":"string"},"title":{"type":"string"},"description":{"type":"string"},"owner":{"type":"string"}}})),
        tool_def("claim_task", "Atomically claim a proposed task. Sets status to in_progress and assigns ownership to the caller.", json!({"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"]})),
        tool_def("update_task", "Update a task by id. Pass expected_status to guard against concurrent overwrites.", json!({"type":"object","properties":{"task_id":{"type":"string"},"title":{"type":"string"},"description":{"type":"string"},"status":{"type":"string"},"owner":{"type":"string"},"contributors":{"type":"string"},"dependencies":{"type":"string"},"definition_of_done":{"type":"string"},"related_artifacts":{"type":"string"},"expected_status":{"type":"string"}}})),
        tool_def("delete_task", "Delete a task by id", json!({"type":"object","properties":{"task_id":{"type":"string"}}})),
        tool_def("list_tasks", "List tasks for a kluster", json!({"type":"object","properties":{"kluster_id":{"type":"string"}}})),
        tool_def("list_pending_ledger_events", "List pending ledger events (default mission-scoped)", json!({"type":"object","properties":{"mission_id":{"type":"string"},"limit":{"type":"integer"}}})),
        tool_def("publish_pending_ledger_events", "Publish pending mission-scoped ledger events to Git", json!({"type":"object","properties":{"mission_id":{"type":"string"}}})),
        tool_def("list_repo_bindings", "List configured repository bindings for current principal", json!({"type":"object","properties":{}})),
        tool_def("provision_mission_persistence", "Create/update connection, binding, and mission policy routes in one call", json!({"type":"object","properties":{"mission_id":{"type":"string"}}})),
        tool_def("resolve_publish_plan", "Resolve publish route (binding/repo/branch/path) for an entity", json!({"type":"object","properties":{"entity_type":{"type":"string"},"entity_id":{"type":"string"},"mission_id":{"type":"string"}}})),
        tool_def("get_publication_status", "List recent publication records", json!({"type":"object","properties":{"mission_id":{"type":"string"},"limit":{"type":"integer"}}})),
        tool_def("get_entity_history", "Get ledger event history for an entity", json!({"type":"object","properties":{"entity_type":{"type":"string"},"entity_id":{"type":"string"},"mission_id":{"type":"string"}}})),
        tool_def("create_mission", "Create a mission", json!({"type":"object","properties":{"name":{"type":"string"},"description":{"type":"string"}}})),
        tool_def("list_missions", "List missions readable by current principal", json!({"type":"object","properties":{"limit":{"type":"integer"}}})),
        tool_def("create_kluster", "Create a kluster in a mission", json!({"type":"object","properties":{"mission_id":{"type":"string"},"name":{"type":"string"},"description":{"type":"string"}}})),
        tool_def("update_kluster", "Update a kluster by id", json!({"type":"object","properties":{"kluster_id":{"type":"string"},"name":{"type":"string"},"description":{"type":"string"},"status":{"type":"string"}}})),
        tool_def("delete_kluster", "Delete a kluster by id", json!({"type":"object","properties":{"kluster_id":{"type":"string"}}})),
        tool_def("update_mission", "Update a mission by id", json!({"type":"object","properties":{"mission_id":{"type":"string"},"name":{"type":"string"},"description":{"type":"string"},"status":{"type":"string"}}})),
        tool_def("delete_mission", "Delete a mission by id", json!({"type":"object","properties":{"mission_id":{"type":"string"}}})),
        tool_def("update_doc", "Update a doc by id", json!({"type":"object","properties":{"doc_id":{"type":"integer"},"title":{"type":"string"},"body":{"type":"string"},"status":{"type":"string"}}})),
        tool_def("update_artifact", "Update an artifact by id", json!({"type":"object","properties":{"artifact_id":{"type":"integer"},"name":{"type":"string"},"status":{"type":"string"}}})),
        tool_def("get_overlap_suggestions", "Get overlap suggestions for a task", json!({"type":"object","properties":{"task_id":{"type":"string"},"limit":{"type":"integer"}}})),
        tool_def("register_agent", "Register a new agent", json!({"type":"object","properties":{"name":{"type":"string"},"capabilities":{"type":"string"}}})),
        tool_def("list_agents", "List registered agents", json!({"type":"object","properties":{"mission_id":{"type":"string"}}})),
        tool_def("get_agent", "Get agent by id", json!({"type":"object","properties":{"agent_id":{"type":"string"}}})),
        tool_def("update_agent_status", "Update agent status (online/offline)", json!({"type":"object","properties":{"agent_id":{"type":"string"},"status":{"type":"string"}}})),
        tool_def("start_agent_session", "Start an agent session (brings agent online)", json!({"type":"object","properties":{"agent_id":{"type":"string"},"context":{"type":"string"},"claude_session_id":{"type":"string"}}})),
        tool_def("end_agent_session", "End an agent session (brings agent offline)", json!({"type":"object","properties":{"agent_id":{"type":"string"},"session_id":{"type":"string"}}})),
        tool_def("release_task", "Release a claimed task back to available", json!({"type":"object","properties":{"task_id":{"type":"string"}}})),
        tool_def("list_task_assignments", "List task assignments", json!({"type":"object","properties":{"agent_id":{"type":"string"},"kluster_id":{"type":"string"}}})),
        tool_def("resolve_skill_snapshot", "Resolve effective mission+kluster skill snapshot", json!({"type":"object","properties":{"mission_id":{"type":"string"},"kluster_id":{"type":"string"}}})),
        tool_def("download_skill_snapshot", "Download a resolved skill snapshot tarball", json!({"type":"object","properties":{"mission_id":{"type":"string"},"kluster_id":{"type":"string"}}})),
        tool_def("get_skill_sync_status", "Read last recorded local skill sync state", json!({"type":"object","properties":{"mission_id":{"type":"string"},"kluster_id":{"type":"string"}}})),
        tool_def("ack_skill_sync", "Record local sync/apply status for mission/kluster skill snapshot", json!({"type":"object","properties":{"mission_id":{"type":"string"},"kluster_id":{"type":"string"},"snapshot_id":{"type":"string"},"sha256":{"type":"string"}}})),
        tool_def("promote_local_skill_overlay", "Mark local overlay as promoted for current mission/kluster state", json!({"type":"object","properties":{"mission_id":{"type":"string"},"kluster_id":{"type":"string"}}})),
        tool_def("list_profiles", "List profiles for the current principal", json!({"type":"object","properties":{}})),
        tool_def("get_profile", "Get profile metadata by name", json!({"type":"object","properties":{"name":{"type":"string"}}})),
        tool_def("publish_profile", "Create or replace a profile bundle", json!({"type":"object","properties":{"name":{"type":"string"},"description":{"type":"string"},"tarball_b64":{"type":"string"}}})),
        tool_def("download_profile", "Download profile bundle by name", json!({"type":"object","properties":{"name":{"type":"string"}}})),
        tool_def("activate_profile", "Set profile as default", json!({"type":"object","properties":{"name":{"type":"string"}}})),
        tool_def("delete_profile", "Delete a profile by name", json!({"type":"object","properties":{"name":{"type":"string"}}})),
        tool_def("profile_status", "Read current profile sha and optional expected-sha match", json!({"type":"object","properties":{"name":{"type":"string"},"expected_sha256":{"type":"string"}}})),
        tool_def("pin_profile_version", "Assert profile sha matches a pinned version", json!({"type":"object","properties":{"name":{"type":"string"},"sha256":{"type":"string"}}})),
        tool_def("register_remote_target", "Register a remote SSH or K8s host as a named target for agent launches", json!({"type":"object","properties":{"name":{"type":"string"},"host":{"type":"string"},"kind":{"type":"string"}}})),
        tool_def("list_remote_targets", "List registered remote targets for the current user", json!({"type":"object","properties":{}})),
        tool_def("delete_remote_target", "Delete a registered remote target by ID", json!({"type":"object","properties":{"target_id":{"type":"string"}}})),
        tool_def("create_remote_launch", "Launch an agent process on a registered remote target", json!({"type":"object","properties":{"target_id":{"type":"string"},"mission_id":{"type":"string"},"kluster_id":{"type":"string"},"runtime_kind":{"type":"string"},"prompt":{"type":"string"}}})),
        tool_def("list_remote_launches", "List remote agent launches, optionally filtered by status", json!({"type":"object","properties":{"status":{"type":"string"},"limit":{"type":"integer"}}})),
        tool_def("get_remote_launch", "Get status, heartbeat age, and log tail for a specific remote launch", json!({"type":"object","properties":{"launch_id":{"type":"string"}}})),
        tool_def("kill_remote_launch", "Kill a running remote launch: revokes its session token and marks it failed", json!({"type":"object","properties":{"launch_id":{"type":"string"}}})),
        tool_def("submit_mesh_task", "Create a task in a kluster (mesh work model)", json!({"type":"object","properties":{"kluster_id":{"type":"string"},"title":{"type":"string"},"description":{"type":"string"},"kind":{"type":"string"},"input_json":{"type":"string"},"priority":{"type":"integer"},"mission_id":{"type":"string"}}})),
        tool_def("list_mesh_tasks", "List tasks in a kluster (mesh work model)", json!({"type":"object","properties":{"kluster_id":{"type":"string"},"status":{"type":"string"},"limit":{"type":"integer"}}})),
        tool_def("get_mesh_task", "Get a single mesh task by ID", json!({"type":"object","properties":{"task_id":{"type":"string"}}})),
        tool_def("claim_mesh_task", "Claim a mesh task for an agent; returns claim_lease_id", json!({"type":"object","properties":{"task_id":{"type":"string"},"agent_id":{"type":"string"},"lease_seconds":{"type":"integer"}}})),
        tool_def("heartbeat_mesh_task", "Renew a mesh task lease to prevent expiry", json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"}}})),
        tool_def("progress_mesh_task", "Post a typed progress event for a mesh task", json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"},"event_type":{"type":"string"},"payload_json":{"type":"string"}}})),
        tool_def("complete_mesh_task", "Mark a mesh task as complete", json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"},"output_json":{"type":"string"}}})),
        tool_def("fail_mesh_task", "Mark a mesh task as failed", json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"},"error":{"type":"string"}}})),
        tool_def("block_mesh_task", "Mark a mesh task as blocked", json!({"type":"object","properties":{"task_id":{"type":"string"},"claim_lease_id":{"type":"string"},"reason":{"type":"string"}}})),
        tool_def("unblock_mesh_task", "Unblock a mesh task (set back to ready)", json!({"type":"object","properties":{"task_id":{"type":"string"}}})),
        tool_def("cancel_mesh_task", "Cancel a mesh task", json!({"type":"object","properties":{"task_id":{"type":"string"}}})),
        tool_def("retry_mesh_task", "Retry a failed or cancelled mesh task", json!({"type":"object","properties":{"task_id":{"type":"string"}}})),
        tool_def("enroll_mesh_agent", "Enroll an agent in a mission (mesh work model)", json!({"type":"object","properties":{"mission_id":{"type":"string"},"agent_id":{"type":"string"},"capabilities_json":{"type":"string"}}})),
        tool_def("list_mesh_agents", "List agents enrolled in a mission", json!({"type":"object","properties":{"mission_id":{"type":"string"}}})),
        tool_def("send_mesh_message", "Send a message in a kluster or mission channel", json!({"type":"object","properties":{"kluster_id":{"type":"string"},"mission_id":{"type":"string"},"content":{"type":"string"},"sender_agent_id":{"type":"string"},"recipient_agent_id":{"type":"string"}}})),
        tool_def("list_mesh_messages", "List messages for an agent inbox", json!({"type":"object","properties":{"agent_id":{"type":"string"},"kluster_id":{"type":"string"},"limit":{"type":"integer"}}})),
        tool_def("list_mission_packs", "List mission packs owned by the current principal", json!({"type":"object","properties":{"limit":{"type":"integer"}}})),
        tool_def("export_mission_pack", "Export a mission into a portable mission pack tarball", json!({"type":"object","properties":{"mission_id":{"type":"string"},"name":{"type":"string"}}})),
        tool_def("install_mission_pack", "Install a mission pack, creating mission + klusters + skills + budgets", json!({"type":"object","properties":{"pack_id":{"type":"string"},"name_override":{"type":"string"}}})),
    ];
    Json(tools)
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
    s.split('/').map(|seg| uri_encode(seg)).collect::<Vec<_>>().join("/")
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

async fn dispatch(
    state: &AppState,
    principal: &Principal,
    tool: &str,
    args: &Value,
) -> Value {
    let now = Utc::now().naive_utc();

    match tool {
        // ── Tasks ─────────────────────────────────────────────────────────────

        "create_task" => {
            let kluster_id = str_arg(args, "kluster_id");
            let title = str_arg(args, "title");
            if kluster_id.is_empty() || title.is_empty() {
                return err_result("kluster_id and title are required");
            }
            let description = str_arg(args, "description");
            let owner = str_arg_or(args, "owner", &principal.subject);
            match sqlx::query(
                "INSERT INTO task (kluster_id, title, description, owner, status, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,'proposed',$5,$5) RETURNING id",
            )
            .bind(&kluster_id)
            .bind(&title)
            .bind(&description)
            .bind(&owner)
            .bind(now)
            .fetch_one(&state.db)
            .await
            {
                Ok(row) => {
                    let id: i32 = row.get("id");
                    ok_result(json!({"id": id, "kluster_id": kluster_id, "title": title, "description": description, "owner": owner, "status": "proposed"}))
                }
                Err(e) => {
                    tracing::error!("mcp create_task: {e}");
                    err_result("database_error")
                }
            }
        }

        "list_tasks" => {
            let kluster_id = str_arg(args, "kluster_id");
            if kluster_id.is_empty() {
                return err_result("kluster_id is required");
            }
            match sqlx::query(
                "SELECT id, kluster_id, title, description, status, owner, created_at, updated_at \
                 FROM task WHERE kluster_id=$1 ORDER BY created_at DESC LIMIT 200",
            )
            .bind(&kluster_id)
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| task_row_to_json(r)).collect())),
                Err(e) => { tracing::error!("mcp list_tasks: {e}"); err_result("database_error") }
            }
        }

        "update_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() { return err_result("task_id is required"); }

            let row = sqlx::query("SELECT id, status FROM task WHERE id=$1 OR public_id=$1")
                .bind(&task_id)
                .fetch_optional(&state.db)
                .await;
            let row = match row {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("task_not_found"),
                Err(e) => { tracing::error!("mcp update_task lookup: {e}"); return err_result("database_error"); }
            };
            let db_id: i32 = row.get("id");
            let current_status: String = row.get("status");

            if let Some(exp) = args.get("expected_status").and_then(|v| v.as_str()) {
                if exp != current_status {
                    return json!({"ok": false, "error": "conflict", "current_status": current_status, "expected_status": exp});
                }
            }

            let title = args.get("title").and_then(|v| v.as_str());
            let description = args.get("description").and_then(|v| v.as_str());
            let status = args.get("status").and_then(|v| v.as_str());
            let owner = args.get("owner").and_then(|v| v.as_str());

            let mut parts: Vec<String> = vec!["updated_at = NOW()".to_string()];
            if title.is_some() { parts.push("title = $2".to_string()); }
            if description.is_some() { parts.push(format!("description = ${}", parts.len() + 2)); }
            if status.is_some() { parts.push(format!("status = ${}", parts.len() + 2)); }
            if owner.is_some() { parts.push(format!("owner = ${}", parts.len() + 2)); }


            let sql = format!("UPDATE task SET {} WHERE id=$1", parts.join(", "));
            let mut q = sqlx::query(&sql).bind(db_id);
            if let Some(v) = title { q = q.bind(v); }
            if let Some(v) = description { q = q.bind(v); }
            if let Some(v) = status { q = q.bind(v); }
            if let Some(v) = owner { q = q.bind(v); }

            match q.execute(&state.db).await {
                Ok(_) => ok_result(json!({"id": db_id, "updated": true})),
                Err(e) => { tracing::error!("mcp update_task: {e}"); err_result("database_error") }
            }
        }

        "delete_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() { return err_result("task_id is required"); }
            match sqlx::query("DELETE FROM task WHERE id=$1 OR public_id=$1")
                .bind(&task_id)
                .execute(&state.db)
                .await
            {
                Ok(r) if r.rows_affected() == 0 => err_result("task_not_found"),
                Ok(_) => ok_result(json!({"deleted": true})),
                Err(e) => { tracing::error!("mcp delete_task: {e}"); err_result("database_error") }
            }
        }

        "claim_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() { return err_result("task_id is required"); }
            match sqlx::query(
                "UPDATE task SET status='in_progress', owner=$2, updated_at=NOW() \
                 WHERE (id=$1::integer OR public_id=$1) AND status='proposed' RETURNING id",
            )
            .bind(&task_id)
            .bind(&principal.subject)
            .fetch_optional(&state.db)
            .await
            {
                Ok(Some(row)) => ok_result(json!({"id": row.get::<i32,_>("id"), "status": "in_progress", "owner": principal.subject})),
                Ok(None) => json!({"ok": false, "error": "conflict", "detail": "task not found or already claimed"}),
                Err(e) => { tracing::error!("mcp claim_task: {e}"); err_result("database_error") }
            }
        }

        // ── Missions ──────────────────────────────────────────────────────────

        "create_mission" => {
            let name = str_arg(args, "name");
            if name.is_empty() { return err_result("name is required"); }
            let description = str_arg(args, "description");
            let id = format!("m_{}", &uuid::Uuid::new_v4().to_string().replace("-", "")[..12]);
            match sqlx::query(
                "INSERT INTO mission (id, name, description, status, owner_subject, created_at, updated_at) \
                 VALUES ($1,$2,$3,'active',$4,$5,$5)"
            )
            .bind(&id).bind(&name).bind(&description).bind(&principal.subject).bind(now)
            .execute(&state.db).await
            {
                Ok(_) => ok_result(json!({"id": id, "name": name, "description": description, "status": "active"})),
                Err(e) => { tracing::error!("mcp create_mission: {e}"); err_result("database_error") }
            }
        }

        "list_missions" => {
            let limit = int_arg(args, "limit").unwrap_or(50).min(200);
            match sqlx::query(
                "SELECT m.id, m.name, m.description, m.status, m.created_at, m.updated_at \
                 FROM mission m \
                 LEFT JOIN missionrolemembership mrm ON mrm.mission_id = m.id AND mrm.subject = $1 \
                 WHERE m.owner_subject = $1 OR mrm.subject IS NOT NULL \
                 ORDER BY m.updated_at DESC LIMIT $2",
            )
            .bind(&principal.subject)
            .bind(limit as i64)
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<String,_>("id"),
                    "name": r.get::<String,_>("name"),
                    "description": r.get::<String,_>("description"),
                    "status": r.get::<String,_>("status"),
                    "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
                    "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_missions: {e}"); err_result("database_error") }
            }
        }

        "update_mission" => {
            let mission_id = str_arg(args, "mission_id");
            if mission_id.is_empty() { return err_result("mission_id is required"); }
            let name = args.get("name").and_then(|v| v.as_str());
            let description = args.get("description").and_then(|v| v.as_str());
            let status = args.get("status").and_then(|v| v.as_str());
            let mut parts = vec!["updated_at = NOW()".to_string()];
            if name.is_some() { parts.push("name = $2".to_string()); }
            if description.is_some() { parts.push(format!("description = ${}", parts.len() + 2)); }
            if status.is_some() { parts.push(format!("status = ${}", parts.len() + 2)); }
            let sql = format!("UPDATE mission SET {} WHERE id=$1", parts.join(", "));
            let mut q = sqlx::query(&sql).bind(&mission_id);
            if let Some(v) = name { q = q.bind(v); }
            if let Some(v) = description { q = q.bind(v); }
            if let Some(v) = status { q = q.bind(v); }
            match q.execute(&state.db).await {
                Ok(r) if r.rows_affected() == 0 => err_result("mission_not_found"),
                Ok(_) => ok_result(json!({"id": mission_id, "updated": true})),
                Err(e) => { tracing::error!("mcp update_mission: {e}"); err_result("database_error") }
            }
        }

        "delete_mission" => {
            let mission_id = str_arg(args, "mission_id");
            if mission_id.is_empty() { return err_result("mission_id is required"); }
            match sqlx::query("DELETE FROM mission WHERE id=$1")
                .bind(&mission_id).execute(&state.db).await
            {
                Ok(r) if r.rows_affected() == 0 => err_result("mission_not_found"),
                Ok(_) => ok_result(json!({"deleted": true})),
                Err(e) => { tracing::error!("mcp delete_mission: {e}"); err_result("database_error") }
            }
        }

        // ── Klusters ──────────────────────────────────────────────────────────

        "create_kluster" => {
            let mission_id = str_arg(args, "mission_id");
            let name = str_arg(args, "name");
            if mission_id.is_empty() || name.is_empty() { return err_result("mission_id and name are required"); }
            let description = str_arg(args, "description");
            let id = format!("k_{}", &uuid::Uuid::new_v4().to_string().replace("-", "")[..12]);
            match sqlx::query(
                "INSERT INTO kluster (id, mission_id, name, description, status, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,'active',$5,$5)"
            )
            .bind(&id).bind(&mission_id).bind(&name).bind(&description).bind(now)
            .execute(&state.db).await
            {
                Ok(_) => ok_result(json!({"id": id, "mission_id": mission_id, "name": name, "description": description, "status": "active"})),
                Err(e) => { tracing::error!("mcp create_kluster: {e}"); err_result("database_error") }
            }
        }

        "update_kluster" => {
            let kluster_id = str_arg(args, "kluster_id");
            if kluster_id.is_empty() { return err_result("kluster_id is required"); }
            let name = args.get("name").and_then(|v| v.as_str());
            let description = args.get("description").and_then(|v| v.as_str());
            let status = args.get("status").and_then(|v| v.as_str());
            let mut parts = vec!["updated_at = NOW()".to_string()];
            if name.is_some() { parts.push("name = $2".to_string()); }
            if description.is_some() { parts.push(format!("description = ${}", parts.len() + 2)); }
            if status.is_some() { parts.push(format!("status = ${}", parts.len() + 2)); }
            let sql = format!("UPDATE kluster SET {} WHERE id=$1", parts.join(", "));
            let mut q = sqlx::query(&sql).bind(&kluster_id);
            if let Some(v) = name { q = q.bind(v); }
            if let Some(v) = description { q = q.bind(v); }
            if let Some(v) = status { q = q.bind(v); }
            match q.execute(&state.db).await {
                Ok(r) if r.rows_affected() == 0 => err_result("kluster_not_found"),
                Ok(_) => ok_result(json!({"id": kluster_id, "updated": true})),
                Err(e) => { tracing::error!("mcp update_kluster: {e}"); err_result("database_error") }
            }
        }

        "delete_kluster" => {
            let kluster_id = str_arg(args, "kluster_id");
            if kluster_id.is_empty() { return err_result("kluster_id is required"); }
            match sqlx::query("DELETE FROM kluster WHERE id=$1")
                .bind(&kluster_id).execute(&state.db).await
            {
                Ok(r) if r.rows_affected() == 0 => err_result("kluster_not_found"),
                Ok(_) => ok_result(json!({"deleted": true})),
                Err(e) => { tracing::error!("mcp delete_kluster: {e}"); err_result("database_error") }
            }
        }

        // ── Docs ──────────────────────────────────────────────────────────────

        "read_doc" => {
            let doc_id = int_arg(args, "doc_id");
            let doc_id = match doc_id { Some(v) => v, None => return err_result("doc_id is required") };
            match sqlx::query(
                "SELECT id, kluster_id, title, body, doc_type, status, provenance, created_at, updated_at FROM doc WHERE id=$1"
            )
            .bind(doc_id as i32)
            .fetch_optional(&state.db)
            .await
            {
                Ok(Some(r)) => ok_result(json!({
                    "id": r.get::<i32,_>("id"),
                    "kluster_id": r.get::<String,_>("kluster_id"),
                    "title": r.get::<String,_>("title"),
                    "body": r.get::<String,_>("body"),
                    "doc_type": r.get::<String,_>("doc_type"),
                    "status": r.get::<String,_>("status"),
                    "provenance": r.get::<String,_>("provenance"),
                    "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
                    "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
                })),
                Ok(None) => err_result("doc_not_found"),
                Err(e) => { tracing::error!("mcp read_doc: {e}"); err_result("database_error") }
            }
        }

        "create_doc" => {
            let kluster_id = str_arg(args, "kluster_id");
            let title = str_arg(args, "title");
            if kluster_id.is_empty() || title.is_empty() { return err_result("kluster_id and title are required"); }
            let body = str_arg(args, "body");
            let doc_type = str_arg_or(args, "doc_type", "note");
            let status = str_arg_or(args, "status", "active");
            let provenance = str_arg(args, "provenance");
            match sqlx::query(
                "INSERT INTO doc (kluster_id, title, body, doc_type, status, provenance, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$7) RETURNING id"
            )
            .bind(&kluster_id).bind(&title).bind(&body).bind(&doc_type).bind(&status).bind(&provenance).bind(now)
            .fetch_one(&state.db).await
            {
                Ok(r) => ok_result(json!({"id": r.get::<i32,_>("id"), "kluster_id": kluster_id, "title": title, "status": status})),
                Err(e) => { tracing::error!("mcp create_doc: {e}"); err_result("database_error") }
            }
        }

        "update_doc" => {
            let doc_id = match int_arg(args, "doc_id") { Some(v) => v, None => return err_result("doc_id is required") };
            let title = args.get("title").and_then(|v| v.as_str());
            let body = args.get("body").and_then(|v| v.as_str());
            let status = args.get("status").and_then(|v| v.as_str());
            let mut parts = vec!["updated_at = NOW()".to_string()];
            if title.is_some() { parts.push("title = $2".to_string()); }
            if body.is_some() { parts.push(format!("body = ${}", parts.len() + 2)); }
            if status.is_some() { parts.push(format!("status = ${}", parts.len() + 2)); }
            let sql = format!("UPDATE doc SET {} WHERE id=$1", parts.join(", "));
            let mut q = sqlx::query(&sql).bind(doc_id as i32);
            if let Some(v) = title { q = q.bind(v); }
            if let Some(v) = body { q = q.bind(v); }
            if let Some(v) = status { q = q.bind(v); }
            match q.execute(&state.db).await {
                Ok(r) if r.rows_affected() == 0 => err_result("doc_not_found"),
                Ok(_) => ok_result(json!({"id": doc_id, "updated": true})),
                Err(e) => { tracing::error!("mcp update_doc: {e}"); err_result("database_error") }
            }
        }

        // ── Agents ────────────────────────────────────────────────────────────

        "register_agent" => {
            let name = str_arg(args, "name");
            if name.is_empty() { return err_result("name is required"); }
            let capabilities = args.get("capabilities").and_then(|v| v.as_str()).unwrap_or("[]");
            match sqlx::query(
                "INSERT INTO agent (name, capabilities, status, metadata, created_at, updated_at) \
                 VALUES ($1,$2,'offline','{}',NOW(),NOW()) RETURNING id"
            )
            .bind(&name).bind(capabilities)
            .fetch_one(&state.db).await
            {
                Ok(r) => ok_result(json!({"id": r.get::<i32,_>("id"), "name": name, "status": "offline"})),
                Err(e) => { tracing::error!("mcp register_agent: {e}"); err_result("database_error") }
            }
        }

        "list_agents" => {
            match sqlx::query("SELECT id, name, status, capabilities, created_at FROM agent ORDER BY created_at DESC LIMIT 100")
                .fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "name": r.get::<String,_>("name"),
                    "status": r.get::<String,_>("status"),
                    "capabilities": r.get::<String,_>("capabilities"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_agents: {e}"); err_result("database_error") }
            }
        }

        "get_agent" => {
            let agent_id = str_arg(args, "agent_id");
            if agent_id.is_empty() { return err_result("agent_id is required"); }
            let id: i32 = match agent_id.parse() { Ok(v) => v, Err(_) => return err_result("invalid agent_id") };
            match sqlx::query("SELECT id, name, status, capabilities, metadata, created_at, updated_at FROM agent WHERE id=$1")
                .bind(id).fetch_optional(&state.db).await
            {
                Ok(Some(r)) => ok_result(json!({
                    "id": r.get::<i32,_>("id"),
                    "name": r.get::<String,_>("name"),
                    "status": r.get::<String,_>("status"),
                    "capabilities": r.get::<String,_>("capabilities"),
                })),
                Ok(None) => err_result("agent_not_found"),
                Err(e) => { tracing::error!("mcp get_agent: {e}"); err_result("database_error") }
            }
        }

        "update_agent_status" => {
            let agent_id = str_arg(args, "agent_id");
            let status = str_arg(args, "status");
            if agent_id.is_empty() || status.is_empty() { return err_result("agent_id and status are required"); }
            let id: i32 = match agent_id.parse() { Ok(v) => v, Err(_) => return err_result("invalid agent_id") };
            match sqlx::query("UPDATE agent SET status=$2, updated_at=NOW() WHERE id=$1")
                .bind(id).bind(&status).execute(&state.db).await
            {
                Ok(r) if r.rows_affected() == 0 => err_result("agent_not_found"),
                Ok(_) => ok_result(json!({"id": id, "status": status})),
                Err(e) => { tracing::error!("mcp update_agent_status: {e}"); err_result("database_error") }
            }
        }

        // ── Klusters search (name-based fallback, no vectorstore) ─────────────

        "search_klusters" => {
            let query = str_arg(args, "query");
            match sqlx::query(
                "SELECT id, mission_id, name, description, status FROM kluster \
                 WHERE name ILIKE $1 OR description ILIKE $1 ORDER BY updated_at DESC LIMIT 20"
            )
            .bind(format!("%{}%", query))
            .fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<String,_>("id"),
                    "mission_id": r.get::<String,_>("mission_id"),
                    "name": r.get::<String,_>("name"),
                    "status": r.get::<String,_>("status"),
                })).collect())),
                Err(e) => { tracing::error!("mcp search_klusters: {e}"); err_result("database_error") }
            }
        }

        // search_tasks needs vectorstore — return SQL fallback
        "search_tasks" => {
            let query = str_arg(args, "query");
            let limit = int_arg(args, "limit").unwrap_or(10).min(50) as i64;
            match sqlx::query(
                "SELECT id, kluster_id, title, description, status, owner FROM task \
                 WHERE title ILIKE $1 OR description ILIKE $1 ORDER BY updated_at DESC LIMIT $2"
            )
            .bind(format!("%{}%", query))
            .bind(limit)
            .fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| task_row_to_json(r)).collect())),
                Err(e) => { tracing::error!("mcp search_tasks: {e}"); err_result("database_error") }
            }
        }

        // ── Artifacts ─────────────────────────────────────────────────────────

        "create_artifact" => {
            let kluster_id = str_arg(args, "kluster_id");
            let name = str_arg(args, "name");
            if kluster_id.is_empty() || name.is_empty() { return err_result("kluster_id and name are required"); }
            let artifact_type = str_arg_or(args, "artifact_type", "file");
            let uri = str_arg(args, "uri");
            let status = str_arg_or(args, "status", "active");
            let mime_type = str_arg(args, "mime_type");
            let size_bytes = int_arg(args, "size_bytes").unwrap_or(0);
            match sqlx::query(
                "INSERT INTO artifact (kluster_id, name, artifact_type, uri, status, mime_type, size_bytes, \
                 external_pointer, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,false,$8,$8) RETURNING id"
            )
            .bind(&kluster_id).bind(&name).bind(&artifact_type).bind(&uri).bind(&status)
            .bind(&mime_type).bind(size_bytes as i64).bind(now)
            .fetch_one(&state.db).await
            {
                Ok(r) => ok_result(json!({"id": r.get::<i32,_>("id"), "kluster_id": kluster_id, "name": name, "status": status})),
                Err(e) => { tracing::error!("mcp create_artifact: {e}"); err_result("database_error") }
            }
        }

        "update_artifact" => {
            let artifact_id = match int_arg(args, "artifact_id") { Some(v) => v as i32, None => return err_result("artifact_id is required") };
            let name = args.get("name").and_then(|v| v.as_str());
            let status = args.get("status").and_then(|v| v.as_str());
            let mut parts = vec!["updated_at = NOW()".to_string()];
            if name.is_some() { parts.push("name = $2".to_string()); }
            if status.is_some() { parts.push(format!("status = ${}", parts.len() + 2)); }
            let sql = format!("UPDATE artifact SET {} WHERE id=$1", parts.join(", "));
            let mut q = sqlx::query(&sql).bind(artifact_id);
            if let Some(v) = name { q = q.bind(v); }
            if let Some(v) = status { q = q.bind(v); }
            match q.execute(&state.db).await {
                Ok(r) if r.rows_affected() == 0 => err_result("artifact_not_found"),
                Ok(_) => ok_result(json!({"id": artifact_id, "updated": true})),
                Err(e) => { tracing::error!("mcp update_artifact: {e}"); err_result("database_error") }
            }
        }

        // ── Overlap / assignments ─────────────────────────────────────────────

        "get_overlap_suggestions" => {
            let task_id = match int_arg(args, "task_id") { Some(v) => v as i32, None => return err_result("task_id is required") };
            let limit = int_arg(args, "limit").unwrap_or(10).min(50) as i64;
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

        "list_task_assignments" => {
            let limit = int_arg(args, "limit").unwrap_or(50).min(200) as i64;
            let agent_id = args.get("agent_id").and_then(|v| v.as_i64());
            let kluster_id = args.get("kluster_id").and_then(|v| v.as_str());
            match sqlx::query(
                "SELECT ta.id, ta.task_id, ta.agent_id, ta.status, ta.created_at, ta.updated_at \
                 FROM taskassignment ta \
                 JOIN task t ON t.id = ta.task_id \
                 WHERE ($1::bigint IS NULL OR ta.agent_id = $1) \
                   AND ($2::text IS NULL OR t.kluster_id = $2) \
                 ORDER BY ta.updated_at DESC LIMIT $3"
            )
            .bind(agent_id).bind(kluster_id).bind(limit)
            .fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "task_id": r.get::<i32,_>("task_id"),
                    "agent_id": r.get::<i32,_>("agent_id"),
                    "status": r.get::<String,_>("status"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_task_assignments: {e}"); err_result("database_error") }
            }
        }

        "release_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() { return err_result("task_id is required"); }
            match sqlx::query(
                "UPDATE task SET status='proposed', owner='', updated_at=NOW() \
                 WHERE (id::text=$1 OR public_id=$1) AND status='in_progress'"
            )
            .bind(&task_id).execute(&state.db).await
            {
                Ok(r) if r.rows_affected() == 0 => err_result("task_not_found_or_not_in_progress"),
                Ok(_) => ok_result(json!({"released": true})),
                Err(e) => { tracing::error!("mcp release_task: {e}"); err_result("database_error") }
            }
        }

        // ── Agent sessions ────────────────────────────────────────────────────

        "start_agent_session" => {
            let agent_id = str_arg(args, "agent_id");
            if agent_id.is_empty() { return err_result("agent_id is required"); }
            let aid: i32 = match agent_id.parse() { Ok(v) => v, Err(_) => return err_result("invalid agent_id") };
            let context = str_arg(args, "context");
            let claude_session_id = args.get("claude_session_id").and_then(|v| v.as_str());
            match sqlx::query(
                "INSERT INTO agentsession (agent_id, context, claude_session_id, started_at) \
                 VALUES ($1,$2,$3,NOW()) RETURNING id"
            )
            .bind(aid).bind(&context).bind(claude_session_id)
            .fetch_one(&state.db).await
            {
                Ok(r) => {
                    let _ = sqlx::query("UPDATE agent SET status='online', updated_at=NOW() WHERE id=$1")
                        .bind(aid).execute(&state.db).await;
                    ok_result(json!({"session_id": r.get::<i32,_>("id"), "agent_id": aid, "status": "online"}))
                }
                Err(e) => { tracing::error!("mcp start_agent_session: {e}"); err_result("database_error") }
            }
        }

        "end_agent_session" => {
            let agent_id = str_arg(args, "agent_id");
            if agent_id.is_empty() { return err_result("agent_id is required"); }
            let aid: i32 = match agent_id.parse() { Ok(v) => v, Err(_) => return err_result("invalid agent_id") };
            let session_id = args.get("session_id").and_then(|v| v.as_i64());
            let q = if let Some(sid) = session_id {
                sqlx::query("UPDATE agentsession SET ended_at=NOW() WHERE id=$1 AND agent_id=$2")
                    .bind(sid as i32).bind(aid)
            } else {
                sqlx::query("UPDATE agentsession SET ended_at=NOW() WHERE agent_id=$1 AND ended_at IS NULL")
                    .bind(aid)
            };
            let _ = q.execute(&state.db).await;
            let _ = sqlx::query("UPDATE agent SET status='offline', updated_at=NOW() WHERE id=$1")
                .bind(aid).execute(&state.db).await;
            ok_result(json!({"agent_id": aid, "status": "offline"}))
        }

        // ── Repo bindings ─────────────────────────────────────────────────────

        "list_repo_bindings" => {
            match sqlx::query(
                "SELECT rb.id, rb.name, rb.owner_subject, rb.branch_override, rb.base_path, rb.active, \
                 rc.provider, rc.host, rc.repo_path, rc.default_branch \
                 FROM repobinding rb JOIN repoconnection rc ON rc.id = rb.connection_id \
                 WHERE rb.owner_subject=$1 AND rb.active=true ORDER BY rb.created_at DESC"
            )
            .bind(&principal.subject)
            .fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "name": r.get::<String,_>("name"),
                    "provider": r.get::<String,_>("provider"),
                    "host": r.get::<String,_>("host"),
                    "repo_path": r.get::<String,_>("repo_path"),
                    "branch_override": r.get::<String,_>("branch_override"),
                    "base_path": r.get::<String,_>("base_path"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_repo_bindings: {e}"); err_result("database_error") }
            }
        }

        // ── Profiles ──────────────────────────────────────────────────────────

        "list_profiles" => {
            match sqlx::query(
                "SELECT id, name, description, is_default, sha256, size_bytes, created_at, updated_at \
                 FROM userprofile WHERE owner_subject=$1 ORDER BY updated_at DESC"
            )
            .bind(&principal.subject).fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "name": r.get::<String,_>("name"),
                    "description": r.get::<String,_>("description"),
                    "is_default": r.get::<bool,_>("is_default"),
                    "sha256": r.get::<Option<String>,_>("sha256"),
                    "size_bytes": r.get::<i64,_>("size_bytes"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_profiles: {e}"); err_result("database_error") }
            }
        }

        "get_profile" => {
            let name = str_arg(args, "name");
            if name.is_empty() { return err_result("name is required"); }
            match sqlx::query(
                "SELECT id, name, description, is_default, sha256, size_bytes, created_at, updated_at \
                 FROM userprofile WHERE owner_subject=$1 AND name=$2"
            )
            .bind(&principal.subject).bind(&name).fetch_optional(&state.db).await
            {
                Ok(Some(r)) => ok_result(json!({
                    "id": r.get::<i32,_>("id"),
                    "name": r.get::<String,_>("name"),
                    "description": r.get::<String,_>("description"),
                    "is_default": r.get::<bool,_>("is_default"),
                    "sha256": r.get::<Option<String>,_>("sha256"),
                    "size_bytes": r.get::<i64,_>("size_bytes"),
                })),
                Ok(None) => err_result("profile_not_found"),
                Err(e) => { tracing::error!("mcp get_profile: {e}"); err_result("database_error") }
            }
        }

        "activate_profile" => {
            let name = str_arg(args, "name");
            if name.is_empty() { return err_result("name is required"); }
            let _ = sqlx::query("UPDATE userprofile SET is_default=false WHERE owner_subject=$1")
                .bind(&principal.subject).execute(&state.db).await;
            match sqlx::query(
                "UPDATE userprofile SET is_default=true, updated_at=NOW() WHERE owner_subject=$1 AND name=$2"
            )
            .bind(&principal.subject).bind(&name).execute(&state.db).await
            {
                Ok(r) if r.rows_affected() == 0 => err_result("profile_not_found"),
                Ok(_) => ok_result(json!({"name": name, "is_default": true})),
                Err(e) => { tracing::error!("mcp activate_profile: {e}"); err_result("database_error") }
            }
        }

        "delete_profile" => {
            let name = str_arg(args, "name");
            if name.is_empty() { return err_result("name is required"); }
            match sqlx::query("DELETE FROM userprofile WHERE owner_subject=$1 AND name=$2")
                .bind(&principal.subject).bind(&name).execute(&state.db).await
            {
                Ok(r) if r.rows_affected() == 0 => err_result("profile_not_found"),
                Ok(_) => ok_result(json!({"deleted": true})),
                Err(e) => { tracing::error!("mcp delete_profile: {e}"); err_result("database_error") }
            }
        }

        "profile_status" => {
            let name = str_arg(args, "name");
            if name.is_empty() { return err_result("name is required"); }
            let expected_sha = args.get("expected_sha256").and_then(|v| v.as_str());
            match sqlx::query("SELECT name, sha256, is_default FROM userprofile WHERE owner_subject=$1 AND name=$2")
                .bind(&principal.subject).bind(&name).fetch_optional(&state.db).await
            {
                Ok(Some(r)) => {
                    let sha: Option<String> = r.get("sha256");
                    let matches = expected_sha.map(|e| sha.as_deref() == Some(e));
                    ok_result(json!({"name": name, "sha256": sha, "is_default": r.get::<bool,_>("is_default"), "sha_matches": matches}))
                }
                Ok(None) => err_result("profile_not_found"),
                Err(e) => { tracing::error!("mcp profile_status: {e}"); err_result("database_error") }
            }
        }

        // ── Remote targets ────────────────────────────────────────────────────

        "register_remote_target" => {
            let name = str_arg(args, "name");
            let host = str_arg(args, "host");
            if name.is_empty() || host.is_empty() { return err_result("name and host are required"); }
            let transport = str_arg_or(args, "kind", "ssh");
            let id = uuid::Uuid::new_v4().to_string();
            match sqlx::query(
                "INSERT INTO remotetarget (id, owner_subject, name, host, transport, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$6)"
            )
            .bind(&id).bind(&principal.subject).bind(&name).bind(&host).bind(&transport).bind(now)
            .execute(&state.db).await
            {
                Ok(_) => ok_result(json!({"id": id, "name": name, "host": host, "transport": transport})),
                Err(e) if e.to_string().contains("unique") || e.to_string().contains("duplicate") =>
                    err_result("target_name_already_exists"),
                Err(e) => { tracing::error!("mcp register_remote_target: {e}"); err_result("database_error") }
            }
        }

        "list_remote_targets" => {
            match sqlx::query(
                "SELECT id, name, host, transport, created_at FROM remotetarget \
                 WHERE owner_subject=$1 ORDER BY created_at DESC LIMIT 100"
            )
            .bind(&principal.subject).fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<String,_>("id"),
                    "name": r.get::<String,_>("name"),
                    "host": r.get::<String,_>("host"),
                    "transport": r.get::<String,_>("transport"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_remote_targets: {e}"); err_result("database_error") }
            }
        }

        "delete_remote_target" => {
            let target_id = str_arg(args, "target_id");
            if target_id.is_empty() { return err_result("target_id is required"); }
            match sqlx::query("DELETE FROM remotetarget WHERE id=$1 AND owner_subject=$2")
                .bind(&target_id).bind(&principal.subject).execute(&state.db).await
            {
                Ok(r) if r.rows_affected() == 0 => err_result("target_not_found"),
                Ok(_) => ok_result(json!({"deleted": true})),
                Err(e) => { tracing::error!("mcp delete_remote_target: {e}"); err_result("database_error") }
            }
        }

        "list_remote_launches" => {
            let status_filter = args.get("status").and_then(|v| v.as_str());
            let limit = int_arg(args, "limit").unwrap_or(20).min(100) as i64;
            match sqlx::query(
                "SELECT id, transport, target_host, agent_kind, status, created_at, updated_at \
                 FROM remotelaunchrecord WHERE owner_subject=$1 \
                 AND ($2::text IS NULL OR status=$2) \
                 ORDER BY created_at DESC LIMIT $3"
            )
            .bind(&principal.subject).bind(status_filter).bind(limit)
            .fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<String,_>("id"),
                    "transport": r.get::<String,_>("transport"),
                    "target_host": r.get::<String,_>("target_host"),
                    "agent_kind": r.get::<String,_>("agent_kind"),
                    "status": r.get::<String,_>("status"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_remote_launches: {e}"); err_result("database_error") }
            }
        }

        "get_remote_launch" => {
            let launch_id = str_arg(args, "launch_id");
            if launch_id.is_empty() { return err_result("launch_id is required"); }
            match sqlx::query(
                "SELECT id, transport, target_host, agent_kind, status, error_message, log_tail, \
                 created_at, updated_at, last_heartbeat_at \
                 FROM remotelaunchrecord WHERE id=$1 AND owner_subject=$2"
            )
            .bind(&launch_id).bind(&principal.subject).fetch_optional(&state.db).await
            {
                Ok(Some(r)) => ok_result(json!({
                    "id": r.get::<String,_>("id"),
                    "transport": r.get::<String,_>("transport"),
                    "target_host": r.get::<String,_>("target_host"),
                    "agent_kind": r.get::<String,_>("agent_kind"),
                    "status": r.get::<String,_>("status"),
                    "error_message": r.get::<String,_>("error_message"),
                    "log_tail": r.get::<String,_>("log_tail"),
                    "last_heartbeat_at": r.get::<Option<chrono::NaiveDateTime>,_>("last_heartbeat_at"),
                })),
                Ok(None) => err_result("launch_not_found"),
                Err(e) => { tracing::error!("mcp get_remote_launch: {e}"); err_result("database_error") }
            }
        }

        // ── Mesh tasks ────────────────────────────────────────────────────────

        "submit_mesh_task" => {
            let kluster_id = str_arg(args, "kluster_id");
            let mission_id = str_arg(args, "mission_id");
            let title = str_arg(args, "title");
            if kluster_id.is_empty() || mission_id.is_empty() || title.is_empty() {
                return err_result("kluster_id, mission_id, and title are required");
            }
            let description = str_arg(args, "description");
            let input_json = args.get("input_json").cloned().unwrap_or(json!({}));
            let priority = int_arg(args, "priority").unwrap_or(0) as i32;
            let id = uuid::Uuid::new_v4().to_string();
            match sqlx::query(
                "INSERT INTO meshtask (id, kluster_id, mission_id, title, description, input_json, \
                 priority, status, created_by_subject, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,'ready',$8,$9,$9)"
            )
            .bind(&id).bind(&kluster_id).bind(&mission_id).bind(&title).bind(&description)
            .bind(input_json.to_string()).bind(priority).bind(&principal.subject).bind(now)
            .execute(&state.db).await
            {
                Ok(_) => ok_result(json!({"task_id": id, "kluster_id": kluster_id, "mission_id": mission_id, "title": title, "status": "ready"})),
                Err(e) => { tracing::error!("mcp submit_mesh_task: {e}"); err_result("database_error") }
            }
        }

        "list_mesh_tasks" => {
            let kluster_id = str_arg(args, "kluster_id");
            let status_filter = args.get("status").and_then(|v| v.as_str());
            let limit = int_arg(args, "limit").unwrap_or(50).min(200) as i64;
            match sqlx::query(
                "SELECT id, kluster_id, mission_id, title, description, status, priority, \
                 claimed_by_agent_id, created_at, updated_at \
                 FROM meshtask \
                 WHERE ($1::text = '' OR kluster_id=$1) \
                   AND ($2::text IS NULL OR status=$2) \
                 ORDER BY priority DESC, created_at ASC LIMIT $3"
            )
            .bind(&kluster_id).bind(status_filter).bind(limit)
            .fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<String,_>("id"),
                    "kluster_id": r.get::<String,_>("kluster_id"),
                    "mission_id": r.get::<String,_>("mission_id"),
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
            if task_id.is_empty() { return err_result("task_id is required"); }
            match sqlx::query(
                "SELECT id, kluster_id, mission_id, title, description, status, priority, \
                 input_json, claimed_by_agent_id, claim_lease_id, lease_expires_at, \
                 created_at, updated_at FROM meshtask WHERE id=$1"
            )
            .bind(&task_id).fetch_optional(&state.db).await
            {
                Ok(Some(r)) => ok_result(json!({
                    "id": r.get::<String,_>("id"),
                    "kluster_id": r.get::<String,_>("kluster_id"),
                    "mission_id": r.get::<String,_>("mission_id"),
                    "title": r.get::<String,_>("title"),
                    "description": r.get::<String,_>("description"),
                    "status": r.get::<String,_>("status"),
                    "priority": r.get::<i32,_>("priority"),
                    "claimed_by_agent_id": r.get::<Option<String>,_>("claimed_by_agent_id"),
                    "claim_lease_id": r.get::<Option<String>,_>("claim_lease_id"),
                    "lease_expires_at": r.get::<Option<chrono::NaiveDateTime>,_>("lease_expires_at"),
                })),
                Ok(None) => err_result("mesh_task_not_found"),
                Err(e) => { tracing::error!("mcp get_mesh_task: {e}"); err_result("database_error") }
            }
        }

        "claim_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            let agent_id = str_arg(args, "agent_id");
            if task_id.is_empty() || agent_id.is_empty() { return err_result("task_id and agent_id are required"); }
            let lease_seconds = int_arg(args, "lease_seconds").unwrap_or(300);
            let lease_id = uuid::Uuid::new_v4().to_string();
            let expires_at = now + chrono::Duration::seconds(lease_seconds);
            match sqlx::query(
                "UPDATE meshtask SET status='claimed', claimed_by_agent_id=$2, claim_lease_id=$3, \
                 lease_expires_at=$4, version_counter=version_counter+1, updated_at=NOW() \
                 WHERE id=$1 AND status='ready' RETURNING id"
            )
            .bind(&task_id).bind(&agent_id).bind(&lease_id).bind(expires_at)
            .fetch_optional(&state.db).await
            {
                Ok(Some(_)) => ok_result(json!({"task_id": task_id, "claim_lease_id": lease_id, "lease_expires_at": expires_at})),
                Ok(None) => json!({"ok": false, "error": "conflict", "detail": "task not found or already claimed"}),
                Err(e) => { tracing::error!("mcp claim_mesh_task: {e}"); err_result("database_error") }
            }
        }

        "heartbeat_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            let claim_lease_id = str_arg(args, "claim_lease_id");
            if task_id.is_empty() || claim_lease_id.is_empty() { return err_result("task_id and claim_lease_id are required"); }
            let expires_at = now + chrono::Duration::seconds(300);
            match sqlx::query(
                "UPDATE meshtask SET lease_expires_at=$3, updated_at=NOW() \
                 WHERE id=$1 AND claim_lease_id=$2 RETURNING id"
            )
            .bind(&task_id).bind(&claim_lease_id).bind(expires_at)
            .fetch_optional(&state.db).await
            {
                Ok(Some(_)) => ok_result(json!({"task_id": task_id, "lease_expires_at": expires_at})),
                Ok(None) => err_result("invalid_task_or_lease"),
                Err(e) => { tracing::error!("mcp heartbeat_mesh_task: {e}"); err_result("database_error") }
            }
        }

        "progress_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            let agent_id = str_arg(args, "agent_id");
            let event_type = str_arg(args, "event_type");
            if task_id.is_empty() || event_type.is_empty() { return err_result("task_id and event_type are required"); }
            let payload_json = args.get("payload_json").cloned().unwrap_or(json!({}));
            let phase = args.get("phase").and_then(|v| v.as_str());
            let step = args.get("step").and_then(|v| v.as_str());
            match sqlx::query(
                "INSERT INTO meshprogressevent (task_id, agent_id, event_type, phase, step, payload_json, occurred_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,NOW()) RETURNING id"
            )
            .bind(&task_id).bind(&agent_id).bind(&event_type).bind(phase).bind(step)
            .bind(payload_json.to_string())
            .fetch_one(&state.db).await
            {
                Ok(r) => ok_result(json!({"event_id": r.get::<i32,_>("id"), "task_id": task_id, "event_type": event_type})),
                Err(e) => { tracing::error!("mcp progress_mesh_task: {e}"); err_result("database_error") }
            }
        }

        "complete_mesh_task" | "fail_mesh_task" | "block_mesh_task"
        | "unblock_mesh_task" | "cancel_mesh_task" | "retry_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() { return err_result("task_id is required"); }
            let new_status = match tool {
                "complete_mesh_task" => "finished",
                "fail_mesh_task"     => "failed",
                "block_mesh_task"    => "blocked",
                "unblock_mesh_task"  => "ready",
                "cancel_mesh_task"   => "cancelled",
                "retry_mesh_task"    => "ready",
                _                    => return err_result("unknown_tool"),
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

        // ── Mesh agents ───────────────────────────────────────────────────────

        "enroll_mesh_agent" => {
            let mission_id = str_arg(args, "mission_id");
            let agent_id_str = args.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
            if mission_id.is_empty() { return err_result("mission_id is required"); }
            // agent_id may be provided (existing agent) or we enroll a new one
            let id = if agent_id_str.is_empty() { uuid::Uuid::new_v4().to_string() } else { agent_id_str.to_string() };
            let capabilities = args.get("capabilities_json").and_then(|v| v.as_str()).unwrap_or("[]");
            let runtime_kind = str_arg_or(args, "runtime_kind", "claude_code");
            match sqlx::query(
                "INSERT INTO meshagent (id, mission_id, runtime_kind, capabilities, status, enrolled_by_subject, enrolled_at) \
                 VALUES ($1,$2,$3,$4,'offline',$5,NOW()) \
                 ON CONFLICT (id) DO UPDATE SET mission_id=$2, status='offline', enrolled_at=NOW() \
                 RETURNING id"
            )
            .bind(&id).bind(&mission_id).bind(&runtime_kind).bind(capabilities).bind(&principal.subject)
            .fetch_one(&state.db).await
            {
                Ok(r) => ok_result(json!({"agent_id": r.get::<String,_>("id"), "mission_id": mission_id, "status": "offline"})),
                Err(e) => { tracing::error!("mcp enroll_mesh_agent: {e}"); err_result("database_error") }
            }
        }

        "list_mesh_agents" => {
            let mission_id = str_arg(args, "mission_id");
            if mission_id.is_empty() { return err_result("mission_id is required"); }
            match sqlx::query(
                "SELECT id, mission_id, runtime_kind, status, capabilities, enrolled_at, last_heartbeat_at \
                 FROM meshagent WHERE mission_id=$1 ORDER BY enrolled_at DESC LIMIT 100"
            )
            .bind(&mission_id).fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<String,_>("id"),
                    "mission_id": r.get::<String,_>("mission_id"),
                    "runtime_kind": r.get::<String,_>("runtime_kind"),
                    "status": r.get::<String,_>("status"),
                    "capabilities": r.get::<String,_>("capabilities"),
                    "last_heartbeat_at": r.get::<Option<chrono::NaiveDateTime>,_>("last_heartbeat_at"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_mesh_agents: {e}"); err_result("database_error") }
            }
        }

        // ── Mesh messages ─────────────────────────────────────────────────────

        "send_mesh_message" => {
            let mission_id = str_arg(args, "mission_id");
            let from_agent_id = str_arg(args, "sender_agent_id");
            let body = args.get("content").cloned().unwrap_or(json!({}));
            if mission_id.is_empty() || from_agent_id.is_empty() { return err_result("mission_id and sender_agent_id are required"); }
            let to_agent_id = args.get("recipient_agent_id").and_then(|v| v.as_str());
            let kluster_id = args.get("kluster_id").and_then(|v| v.as_str());
            let channel = str_arg_or(args, "channel", "coordination");
            let body_json = if body.is_string() {
                json!({"text": body.as_str().unwrap_or("")})
            } else { body };
            match sqlx::query(
                "INSERT INTO meshmessage (mission_id, kluster_id, from_agent_id, to_agent_id, channel, body_json, created_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,NOW()) RETURNING id"
            )
            .bind(&mission_id).bind(kluster_id).bind(&from_agent_id).bind(to_agent_id)
            .bind(&channel).bind(body_json.to_string())
            .fetch_one(&state.db).await
            {
                Ok(r) => ok_result(json!({"message_id": r.get::<i32,_>("id"), "mission_id": mission_id})),
                Err(e) => { tracing::error!("mcp send_mesh_message: {e}"); err_result("database_error") }
            }
        }

        "list_mesh_messages" => {
            let agent_id = str_arg(args, "agent_id");
            let limit = int_arg(args, "limit").unwrap_or(20).min(100) as i64;
            if agent_id.is_empty() { return err_result("agent_id is required"); }
            match sqlx::query(
                "SELECT id, mission_id, from_agent_id, to_agent_id, channel, body_json, created_at, read_at \
                 FROM meshmessage WHERE (to_agent_id=$1 OR to_agent_id IS NULL) \
                 ORDER BY created_at DESC LIMIT $2"
            )
            .bind(&agent_id).bind(limit).fetch_all(&state.db).await
            {
                Ok(rows) => ok_result(Value::Array(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "mission_id": r.get::<String,_>("mission_id"),
                    "from_agent_id": r.get::<String,_>("from_agent_id"),
                    "to_agent_id": r.get::<Option<String>,_>("to_agent_id"),
                    "channel": r.get::<String,_>("channel"),
                    "body_json": r.get::<String,_>("body_json"),
                    "read_at": r.get::<Option<chrono::NaiveDateTime>,_>("read_at"),
                })).collect())),
                Err(e) => { tracing::error!("mcp list_mesh_messages: {e}"); err_result("database_error") }
            }
        }

        // ── Ledger events ─────────────────────────────────────────────────────

        "list_pending_ledger_events" => {
            let mission_id = str_arg(args, "mission_id");
            let limit = int_arg(args, "limit").unwrap_or(100).min(500);
            let rows = if mission_id.is_empty() {
                sqlx::query("SELECT * FROM ledgerevent WHERE state='pending' ORDER BY created_at ASC LIMIT $1")
                    .bind(limit).fetch_all(&state.db).await
            } else {
                sqlx::query("SELECT * FROM ledgerevent WHERE state='pending' AND mission_id=$1 ORDER BY created_at ASC LIMIT $2")
                    .bind(&mission_id).bind(limit).fetch_all(&state.db).await
            };
            match rows {
                Ok(rows) => ok_result(json!(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "event_id": r.get::<String,_>("event_id"),
                    "mission_id": r.get::<Option<String>,_>("mission_id"),
                    "entity_type": r.get::<String,_>("entity_type"),
                    "entity_id": r.get::<String,_>("entity_id"),
                    "action": r.get::<String,_>("action"),
                    "payload_json": r.get::<Option<String>,_>("payload_json"),
                    "state": r.get::<String,_>("state"),
                    "created_by_subject": r.get::<Option<String>,_>("created_by_subject"),
                    "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
                })).collect::<Vec<_>>())),
                Err(e) => { tracing::error!("mcp list_pending_ledger_events: {e}"); err_result("database_error") }
            }
        }

        "get_entity_history" => {
            let mission_id = str_arg(args, "mission_id");
            let entity_type = str_arg(args, "entity_type");
            let entity_id = str_arg(args, "entity_id");
            let limit = int_arg(args, "limit").unwrap_or(200).min(500);
            if entity_type.is_empty() || entity_id.is_empty() {
                return err_result("entity_type and entity_id are required");
            }
            let rows = if mission_id.is_empty() {
                sqlx::query("SELECT * FROM ledgerevent WHERE entity_type=$1 AND entity_id=$2 ORDER BY created_at DESC LIMIT $3")
                    .bind(&entity_type).bind(&entity_id).bind(limit).fetch_all(&state.db).await
            } else {
                sqlx::query("SELECT * FROM ledgerevent WHERE mission_id=$1 AND entity_type=$2 AND entity_id=$3 ORDER BY created_at DESC LIMIT $4")
                    .bind(&mission_id).bind(&entity_type).bind(&entity_id).bind(limit).fetch_all(&state.db).await
            };
            match rows {
                Ok(rows) => ok_result(json!(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "event_id": r.get::<String,_>("event_id"),
                    "mission_id": r.get::<Option<String>,_>("mission_id"),
                    "entity_type": r.get::<String,_>("entity_type"),
                    "entity_id": r.get::<String,_>("entity_id"),
                    "action": r.get::<String,_>("action"),
                    "payload_json": r.get::<Option<String>,_>("payload_json"),
                    "state": r.get::<String,_>("state"),
                    "created_by_subject": r.get::<Option<String>,_>("created_by_subject"),
                    "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
                })).collect::<Vec<_>>())),
                Err(e) => { tracing::error!("mcp get_entity_history: {e}"); err_result("database_error") }
            }
        }

        // ── Publication ───────────────────────────────────────────────────────

        "resolve_publish_plan" => {
            let mission_id = str_arg(args, "mission_id");
            let entity_kind = str_arg(args, "entity_kind");
            let event_kind = str_arg(args, "event_kind");
            if mission_id.is_empty() || entity_kind.is_empty() {
                return err_result("mission_id and entity_kind are required");
            }
            let row = sqlx::query(
                "SELECT r.id AS route_id, r.format, r.branch, r.rel_path_template, \
                 b.id AS binding_id, b.name AS binding_name, \
                 c.id AS conn_id, c.provider, c.host, c.repo_path \
                 FROM missionpersistenceroute r \
                 JOIN repobinding b ON b.id = r.binding_id \
                 JOIN repoconnection c ON c.id = b.connection_id \
                 WHERE r.mission_id=$1 AND r.entity_kind=$2 AND r.active=true \
                 AND (r.event_kind=$3 OR r.event_kind='') \
                 ORDER BY r.event_kind DESC LIMIT 1"
            )
            .bind(&mission_id).bind(&entity_kind).bind(&event_kind)
            .fetch_optional(&state.db).await;
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
                Err(e) => { tracing::error!("mcp resolve_publish_plan: {e}"); err_result("database_error") }
            }
        }

        "get_publication_status" => {
            let mission_id = str_arg(args, "mission_id");
            let limit = int_arg(args, "limit").unwrap_or(20).min(200);
            let rows = if mission_id.is_empty() {
                sqlx::query("SELECT * FROM publicationrecord WHERE owner_subject=$1 ORDER BY created_at DESC LIMIT $2")
                    .bind(&principal.subject).bind(limit).fetch_all(&state.db).await
            } else {
                sqlx::query("SELECT * FROM publicationrecord WHERE owner_subject=$1 AND mission_id=$2 ORDER BY created_at DESC LIMIT $3")
                    .bind(&principal.subject).bind(&mission_id).bind(limit).fetch_all(&state.db).await
            };
            match rows {
                Ok(rows) => ok_result(json!(rows.iter().map(|r| json!({
                    "id": r.get::<i32,_>("id"),
                    "owner_subject": r.get::<String,_>("owner_subject"),
                    "mission_id": r.get::<Option<String>,_>("mission_id"),
                    "entity_kind": r.get::<String,_>("entity_kind"),
                    "entity_id": r.get::<String,_>("entity_id"),
                    "event_kind": r.get::<Option<String>,_>("event_kind"),
                    "binding_id": r.get::<Option<i32>,_>("binding_id"),
                    "status": r.get::<String,_>("status"),
                    "error": r.get::<Option<String>,_>("error"),
                    "commit_sha": r.get::<Option<String>,_>("commit_sha"),
                    "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
                    "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
                })).collect::<Vec<_>>())),
                Err(e) => { tracing::error!("mcp get_publication_status: {e}"); err_result("database_error") }
            }
        }

        // ── Skill sync state ──────────────────────────────────────────────────

        "get_skill_sync_status" => {
            let mission_id = str_arg(args, "mission_id");
            let kluster_id = str_arg(args, "kluster_id");
            let agent_id = str_arg(args, "agent_id");
            if mission_id.is_empty() { return err_result("mission_id is required"); }
            let row = sqlx::query(
                "SELECT * FROM skilllocalstate WHERE actor_subject=$1 AND mission_id=$2 \
                 AND ($3='' OR kluster_id=$3) AND ($4='' OR agent_id=$4) LIMIT 1"
            )
            .bind(&principal.subject).bind(&mission_id).bind(&kluster_id).bind(&agent_id)
            .fetch_optional(&state.db).await;
            match row {
                Ok(Some(r)) => ok_result(json!({
                    "mission_id": r.get::<String,_>("mission_id"),
                    "kluster_id": r.get::<Option<String>,_>("kluster_id"),
                    "actor_subject": r.get::<String,_>("actor_subject"),
                    "agent_id": r.get::<Option<String>,_>("agent_id"),
                    "last_snapshot_id": r.get::<Option<String>,_>("last_snapshot_id"),
                    "last_snapshot_sha256": r.get::<Option<String>,_>("last_snapshot_sha256"),
                    "local_overlay_sha256": r.get::<Option<String>,_>("local_overlay_sha256"),
                    "degraded_offline": r.get::<Option<bool>,_>("degraded_offline"),
                    "drift_flag": r.get::<Option<bool>,_>("drift_flag"),
                    "drift_details": r.get::<Option<String>,_>("drift_details"),
                    "last_sync_at": r.get::<Option<chrono::NaiveDateTime>,_>("last_sync_at"),
                })),
                Ok(None) => ok_result(json!({"mission_id": mission_id, "status": "no_record"})),
                Err(e) => { tracing::error!("mcp get_skill_sync_status: {e}"); err_result("database_error") }
            }
        }

        "ack_skill_sync" => {
            let mission_id = str_arg(args, "mission_id");
            let kluster_id = str_arg(args, "kluster_id");
            let agent_id = str_arg(args, "agent_id");
            let snapshot_id = str_arg(args, "snapshot_id");
            let snapshot_sha256 = str_arg(args, "snapshot_sha256");
            let local_overlay_sha256 = str_arg(args, "local_overlay_sha256");
            let degraded = args.get("degraded_offline").and_then(|v| v.as_bool()).unwrap_or(false);
            let drift_flag = args.get("drift_flag").and_then(|v| v.as_bool()).unwrap_or(false);
            let drift_details = args.get("drift_details").map(|v| v.to_string());
            if mission_id.is_empty() || kluster_id.is_empty() {
                return err_result("mission_id and kluster_id are required");
            }
            let result = sqlx::query(
                "INSERT INTO skilllocalstate \
                 (actor_subject, mission_id, kluster_id, agent_id, last_snapshot_id, last_snapshot_sha256, \
                  local_overlay_sha256, degraded_offline, drift_flag, drift_details, last_sync_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,NOW()) \
                 ON CONFLICT (actor_subject, mission_id, kluster_id, agent_id) DO UPDATE SET \
                 last_snapshot_id=$5, last_snapshot_sha256=$6, local_overlay_sha256=$7, \
                 degraded_offline=$8, drift_flag=$9, drift_details=$10, last_sync_at=NOW() \
                 RETURNING *"
            )
            .bind(&principal.subject).bind(&mission_id).bind(&kluster_id)
            .bind(if agent_id.is_empty() { None } else { Some(agent_id.clone()) })
            .bind(if snapshot_id.is_empty() { None } else { Some(snapshot_id) })
            .bind(if snapshot_sha256.is_empty() { None } else { Some(snapshot_sha256) })
            .bind(if local_overlay_sha256.is_empty() { None } else { Some(local_overlay_sha256) })
            .bind(degraded).bind(drift_flag).bind(drift_details)
            .fetch_one(&state.db).await;
            match result {
                Ok(r) => ok_result(json!({
                    "mission_id": r.get::<String,_>("mission_id"),
                    "kluster_id": r.get::<Option<String>,_>("kluster_id"),
                    "actor_subject": r.get::<String,_>("actor_subject"),
                    "last_snapshot_id": r.get::<Option<String>,_>("last_snapshot_id"),
                    "drift_flag": r.get::<Option<bool>,_>("drift_flag"),
                    "last_sync_at": r.get::<Option<chrono::NaiveDateTime>,_>("last_sync_at"),
                })),
                Err(e) => { tracing::error!("mcp ack_skill_sync: {e}"); err_result("database_error") }
            }
        }

        "promote_local_skill_overlay" => {
            let mission_id = str_arg(args, "mission_id");
            let kluster_id = str_arg(args, "kluster_id");
            let agent_id = str_arg(args, "agent_id");
            let sha256 = str_arg(args, "local_overlay_sha256");
            let note = str_arg(args, "note");
            if mission_id.is_empty() || kluster_id.is_empty() || sha256.is_empty() {
                return err_result("mission_id, kluster_id, local_overlay_sha256 are required");
            }
            let drift_details = json!({"promoted": true, "note": note}).to_string();
            let result = sqlx::query(
                "INSERT INTO skilllocalstate \
                 (actor_subject, mission_id, kluster_id, agent_id, local_overlay_sha256, drift_details, last_sync_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,NOW()) \
                 ON CONFLICT (actor_subject, mission_id, kluster_id, agent_id) DO UPDATE SET \
                 local_overlay_sha256=$5, drift_details=$6, last_sync_at=NOW() \
                 RETURNING *"
            )
            .bind(&principal.subject).bind(&mission_id).bind(&kluster_id)
            .bind(if agent_id.is_empty() { None } else { Some(agent_id) })
            .bind(&sha256).bind(&drift_details)
            .fetch_one(&state.db).await;
            match result {
                Ok(r) => ok_result(json!({
                    "mission_id": r.get::<String,_>("mission_id"),
                    "kluster_id": r.get::<Option<String>,_>("kluster_id"),
                    "actor_subject": r.get::<String,_>("actor_subject"),
                    "local_overlay_sha256": r.get::<Option<String>,_>("local_overlay_sha256"),
                    "drift_details": drift_details,
                    "last_sync_at": r.get::<Option<chrono::NaiveDateTime>,_>("last_sync_at"),
                })),
                Err(e) => { tracing::error!("mcp promote_local_skill_overlay: {e}"); err_result("database_error") }
            }
        }

        // ── Profile pin ───────────────────────────────────────────────────────

        "pin_profile_version" => {
            let name = str_arg(args, "name");
            let sha256 = str_arg(args, "sha256");
            if name.is_empty() || sha256.is_empty() { return err_result("name and sha256 are required"); }
            let row = sqlx::query(
                "SELECT name, sha256 FROM userprofile WHERE owner_subject=$1 AND name=$2"
            )
            .bind(&principal.subject).bind(&name)
            .fetch_optional(&state.db).await;
            match row {
                Ok(Some(r)) => {
                    let remote_sha: Option<String> = r.get("sha256");
                    let matches = remote_sha.as_deref() == Some(sha256.as_str());
                    ok_result(json!({"name": name, "pinned_sha256": sha256, "remote_sha256": remote_sha, "matches": matches}))
                }
                Ok(None) => err_result("profile_not_found"),
                Err(e) => { tracing::error!("mcp pin_profile_version: {e}"); err_result("database_error") }
            }
        }

        // ── Mission packs (list only; export/install need Python) ─────────────

        "list_mission_packs" => {
            let limit = int_arg(args, "limit").unwrap_or(50).min(200);
            let rows = sqlx::query(
                "SELECT id, name, version, sha256, created_at FROM missionpack \
                 WHERE owner_subject=$1 ORDER BY created_at DESC LIMIT $2"
            )
            .bind(&principal.subject).bind(limit).fetch_all(&state.db).await;
            match rows {
                Ok(rows) => ok_result(json!(rows.iter().map(|r| json!({
                    "id": r.get::<String,_>("id"),
                    "name": r.get::<String,_>("name"),
                    "version": r.get::<Option<String>,_>("version"),
                    "sha256": r.get::<Option<String>,_>("sha256").map(|s| s[..8.min(s.len())].to_string()),
                    "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
                })).collect::<Vec<_>>())),
                Err(e) => { tracing::error!("mcp list_mission_packs: {e}"); err_result("database_error") }
            }
        }

        // ── Remote launch ─────────────────────────────────────────────────────

        "create_remote_launch" => {
            let target_id = str_arg(args, "target_id");
            let target_host = str_arg(args, "target_host");
            let transport = str_arg_or(args, "transport", "ssh");
            let agent_kind = str_arg_or(args, "agent_kind", "generic");
            let agent_profile = str_arg(args, "agent_profile");
            let capability_scope = str_arg_or(args, "capability_scope", "");
            let ttl_hours = int_arg(args, "ttl_hours").unwrap_or(8);
            if target_host.is_empty() && target_id.is_empty() {
                return err_result("target_host or target_id is required");
            }
            let (session_id, raw_token) = match issue_mcp_session(
                &state.db, &principal.subject, ttl_hours, &capability_scope
            ).await {
                Ok(v) => v,
                Err(e) => { tracing::error!("mcp create_remote_launch session: {e}"); return err_result("database_error"); }
            };
            let launch_id = format!("rl-{}", hex::encode(&uuid::Uuid::new_v4().as_bytes()[..4]));
            let result = sqlx::query(
                "INSERT INTO remotelaunchrecord \
                 (id, owner_subject, transport, target_id, target_host, agent_kind, agent_profile, \
                  capability_scope, session_token_id, status, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'launching',$10,$10) RETURNING *"
            )
            .bind(&launch_id).bind(&principal.subject).bind(&transport)
            .bind(if target_id.is_empty() { None } else { Some(target_id) })
            .bind(if target_host.is_empty() { None } else { Some(target_host) })
            .bind(&agent_kind).bind(if agent_profile.is_empty() { None } else { Some(agent_profile) })
            .bind(&capability_scope).bind(session_id).bind(now)
            .fetch_one(&state.db).await;
            match result {
                Ok(r) => ok_result(json!({
                    "launch_id": r.get::<String,_>("id"),
                    "owner_subject": r.get::<String,_>("owner_subject"),
                    "transport": r.get::<String,_>("transport"),
                    "agent_kind": r.get::<String,_>("agent_kind"),
                    "status": r.get::<String,_>("status"),
                    "session_token_id": r.get::<i32,_>("session_token_id"),
                    "session_token": raw_token,
                    "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
                })),
                Err(e) => { tracing::error!("mcp create_remote_launch: {e}"); err_result("database_error") }
            }
        }

        "kill_remote_launch" => {
            let launch_id = str_arg(args, "launch_id");
            if launch_id.is_empty() { return err_result("launch_id is required"); }
            let row = sqlx::query(
                "SELECT id, owner_subject, session_token_id FROM remotelaunchrecord WHERE id=$1"
            )
            .bind(&launch_id).fetch_optional(&state.db).await;
            let row = match row {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("launch_not_found"),
                Err(e) => { tracing::error!("mcp kill_remote_launch fetch: {e}"); return err_result("database_error"); }
            };
            let owner: String = row.get("owner_subject");
            if owner != principal.subject && !principal.is_admin {
                return err_result("forbidden");
            }
            let session_token_id: Option<i32> = row.get("session_token_id");
            if let Some(sid) = session_token_id {
                let _ = sqlx::query("UPDATE usersession SET revoked=true WHERE id=$1")
                    .bind(sid).execute(&state.db).await;
            }
            let _ = sqlx::query(
                "UPDATE remotelaunchrecord SET status='failed', error_message='killed by owner', updated_at=$2 WHERE id=$1"
            )
            .bind(&launch_id).bind(now).execute(&state.db).await;
            ok_result(json!({"killed": launch_id}))
        }

        // ── Skill snapshots ───────────────────────────────────────────────────

        "resolve_skill_snapshot" => {
            let mission_id = str_arg(args, "mission_id");
            let kluster_id = str_arg(args, "kluster_id");
            if mission_id.is_empty() { return err_result("mission_id is required"); }
            // Try kluster-specific first, then fall back to mission-level
            let row = if !kluster_id.is_empty() {
                let r = sqlx::query(
                    "SELECT * FROM skillsnapshot WHERE mission_id=$1 AND kluster_id=$2 \
                     ORDER BY created_at DESC LIMIT 1"
                )
                .bind(&mission_id).bind(&kluster_id)
                .fetch_optional(&state.db).await;
                match r {
                    Ok(Some(r)) => Some(r),
                    Ok(None) => sqlx::query(
                        "SELECT * FROM skillsnapshot WHERE mission_id=$1 AND kluster_id='' \
                         ORDER BY created_at DESC LIMIT 1"
                    )
                    .bind(&mission_id).fetch_optional(&state.db).await.unwrap_or(None),
                    Err(e) => { tracing::error!("mcp resolve_skill_snapshot: {e}"); return err_result("database_error"); }
                }
            } else {
                sqlx::query(
                    "SELECT * FROM skillsnapshot WHERE mission_id=$1 AND kluster_id='' \
                     ORDER BY created_at DESC LIMIT 1"
                )
                .bind(&mission_id).fetch_optional(&state.db).await.unwrap_or(None)
            };
            match row {
                Some(r) => {
                    let manifest_json: String = r.try_get("manifest_json").unwrap_or_default();
                    let manifest: serde_json::Value = serde_json::from_str(&manifest_json).unwrap_or(json!({}));
                    ok_result(json!({"snapshot": {
                        "id": r.get::<String,_>("id"),
                        "mission_id": r.get::<String,_>("mission_id"),
                        "kluster_id": r.get::<String,_>("kluster_id"),
                        "effective_version": r.get::<String,_>("effective_version"),
                        "mission_bundle_id": r.get::<String,_>("mission_bundle_id"),
                        "kluster_bundle_id": r.get::<String,_>("kluster_bundle_id"),
                        "sha256": r.get::<String,_>("sha256"),
                        "size_bytes": r.get::<i32,_>("size_bytes"),
                        "manifest": manifest,
                    }}))
                }
                None => err_result("no_snapshot_found"),
            }
        }

        "download_skill_snapshot" => {
            let snapshot_id = str_arg(args, "snapshot_id");
            if snapshot_id.is_empty() { return err_result("snapshot_id is required"); }
            match sqlx::query("SELECT * FROM skillsnapshot WHERE id=$1")
                .bind(&snapshot_id).fetch_optional(&state.db).await
            {
                Ok(Some(r)) => {
                    let manifest_json: String = r.try_get("manifest_json").unwrap_or_default();
                    let manifest: serde_json::Value = serde_json::from_str(&manifest_json).unwrap_or(json!({}));
                    ok_result(json!({"snapshot": {
                        "id": r.get::<String,_>("id"),
                        "sha256": r.get::<String,_>("sha256"),
                        "size_bytes": r.get::<i32,_>("size_bytes"),
                        "tarball_b64": r.get::<String,_>("tarball_b64"),
                        "manifest": manifest,
                    }}))
                }
                Ok(None) => err_result("not_found"),
                Err(e) => { tracing::error!("mcp download_skill_snapshot: {e}"); err_result("database_error") }
            }
        }

        // ── Profiles ──────────────────────────────────────────────────────────

        "publish_profile" => {
            use sha2::{Digest, Sha256};
            use base64::Engine;
            let name = str_arg(args, "name");
            let tarball_b64 = str_arg(args, "tarball_b64");
            if name.is_empty() || tarball_b64.is_empty() {
                return err_result("name and tarball_b64 are required");
            }
            let expected_sha = str_arg(args, "expected_sha256");
            let description = str_arg(args, "description");
            let is_default = args.get("is_default").and_then(|v| v.as_bool()).unwrap_or(false);
            let manifest_json = args.get("manifest")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "[]".into());

            let raw = match base64::engine::general_purpose::STANDARD.decode(tarball_b64.as_bytes()) {
                Ok(b) => b,
                Err(_) => return err_result("tarball_b64 is not valid base64"),
            };
            let computed_sha = hex::encode(Sha256::new().chain_update(&raw).finalize());
            let size_bytes = raw.len() as i32;

            // Check expected_sha conflict
            if !expected_sha.is_empty() {
                let current: Option<String> = sqlx::query_scalar(
                    "SELECT sha256 FROM userprofile WHERE owner_subject=$1 AND name=$2"
                )
                .bind(&principal.subject).bind(&name)
                .fetch_optional(&state.db).await.unwrap_or(None).flatten();
                if let Some(cur) = current {
                    if cur != expected_sha {
                        return ok_result(json!({"error": "profile_sha_mismatch",
                            "expected_sha256": expected_sha, "current_sha256": cur, "name": name}));
                    }
                }
            }

            let row = sqlx::query(
                "INSERT INTO userprofile \
                 (name, owner_subject, description, is_default, manifest_json, tarball_b64, \
                  sha256, size_bytes, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9) \
                 ON CONFLICT (owner_subject, name) DO UPDATE SET \
                 description=$3, is_default=$4, manifest_json=$5, tarball_b64=$6, \
                 sha256=$7, size_bytes=$8, updated_at=$9 \
                 RETURNING *"
            )
            .bind(&name).bind(&principal.subject).bind(&description)
            .bind(is_default).bind(&manifest_json).bind(&tarball_b64)
            .bind(&computed_sha).bind(size_bytes).bind(now)
            .fetch_one(&state.db).await;

            match row {
                Ok(r) => {
                    // Clear is_default from other profiles if needed
                    if is_default {
                        let _ = sqlx::query(
                            "UPDATE userprofile SET is_default=false, updated_at=$3 \
                             WHERE owner_subject=$1 AND name<>$2 AND is_default=true"
                        )
                        .bind(&principal.subject).bind(&name).bind(now)
                        .execute(&state.db).await;
                    }
                    ok_result(json!({"profile": profile_row_to_json(&r)}))
                }
                Err(e) => { tracing::error!("mcp publish_profile: {e}"); err_result("database_error") }
            }
        }

        "download_profile" => {
            let name = str_arg(args, "name");
            if name.is_empty() { return err_result("name is required"); }
            let if_sha256 = str_arg(args, "if_sha256");
            match sqlx::query(
                "SELECT * FROM userprofile WHERE owner_subject=$1 AND name=$2"
            )
            .bind(&principal.subject).bind(&name)
            .fetch_optional(&state.db).await
            {
                Ok(Some(r)) => {
                    let current_sha: String = r.try_get("sha256").unwrap_or_default();
                    if !if_sha256.is_empty() && if_sha256 != current_sha {
                        return ok_result(json!({"error": "profile_sha_mismatch",
                            "expected_sha256": if_sha256, "current_sha256": current_sha, "name": name}));
                    }
                    let tarball_b64: String = r.try_get("tarball_b64").unwrap_or_default();
                    ok_result(json!({"profile": profile_row_to_json(&r), "tarball_b64": tarball_b64}))
                }
                Ok(None) => err_result("not_found"),
                Err(e) => { tracing::error!("mcp download_profile: {e}"); err_result("database_error") }
            }
        }

        // ── Mission packs ─────────────────────────────────────────────────────

        "export_mission_pack" => {
            use sha2::{Digest, Sha256};
            use base64::Engine;
            use flate2::write::GzEncoder;
            use flate2::Compression;
            let mission_id = str_arg(args, "mission_id");
            if mission_id.is_empty() { return err_result("mission_id is required"); }

            let mission = sqlx::query("SELECT * FROM mission WHERE id=$1")
                .bind(&mission_id).fetch_optional(&state.db).await;
            let mission = match mission {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("mission not found"),
                Err(e) => { tracing::error!("mcp export_mission_pack: {e}"); return err_result("database_error"); }
            };

            let klusters = sqlx::query("SELECT * FROM kluster WHERE mission_id=$1")
                .bind(&mission_id).fetch_all(&state.db).await.unwrap_or_default();
            let skill_bundles = sqlx::query(
                "SELECT * FROM skillbundle WHERE scope_type='mission' AND scope_id=$1"
            )
            .bind(&mission_id).fetch_all(&state.db).await.unwrap_or_default();
            let budget_policies = sqlx::query(
                "SELECT * FROM budgetpolicy WHERE scope_type='mission' AND scope_id=$1 \
                 AND owner_subject=$2 AND active=true"
            )
            .bind(&mission_id).bind(&principal.subject)
            .fetch_all(&state.db).await.unwrap_or_default();

            // Build tar.gz in memory
            let buf = Vec::new();
            let enc = GzEncoder::new(buf, Compression::default());
            let mut tar = tar::Builder::new(enc);

            let mission_name: String = mission.get("name");
            let mission_desc: String = mission.try_get("description").unwrap_or_default();
            let exported_at = chrono::Utc::now().to_rfc3339();

            let manifest = json!({
                "version": 1,
                "mission_id": mission_id,
                "mission_name": mission_name,
                "kluster_count": klusters.len(),
                "skill_count": skill_bundles.len(),
                "budget_count": budget_policies.len(),
                "exported_at": exported_at,
            });

            fn add_json_entry(tar: &mut tar::Builder<GzEncoder<Vec<u8>>>, name: &str, v: &serde_json::Value) -> std::io::Result<()> {
                let content = serde_json::to_vec_pretty(v).unwrap_or_default();
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, name, content.as_slice())
            }

            if let Err(e) = add_json_entry(&mut tar, "mission.json", &json!({
                "id": mission_id, "name": mission_name, "description": mission_desc,
            })) { tracing::error!("mcp export tar mission.json: {e}"); return err_result("tar_error"); }

            for k in &klusters {
                let kid: String = k.get("id");
                let kname: String = k.get("name");
                let kdesc: String = k.try_get("description").unwrap_or_default();
                let path = format!("klusters/{kid}.json");
                if let Err(e) = add_json_entry(&mut tar, &path, &json!({"id": kid, "name": kname, "description": kdesc})) {
                    tracing::error!("mcp export tar {path}: {e}"); return err_result("tar_error");
                }
            }
            for sb in &skill_bundles {
                let sid: String = sb.get("id");
                let sver: i32 = sb.try_get("version").unwrap_or(1);
                let starball: String = sb.try_get("tarball_b64").unwrap_or_default();
                let ssha: String = sb.try_get("sha256").unwrap_or_default();
                let path = format!("skills/{sid}.json");
                if let Err(e) = add_json_entry(&mut tar, &path, &json!({"id": sid, "version": sver, "tarball_b64": starball, "sha256": ssha})) {
                    tracing::error!("mcp export tar {path}: {e}"); return err_result("tar_error");
                }
            }
            for bp in &budget_policies {
                let bid: String = bp.get("id");
                let path = format!("budgets/{bid}.json");
                let v = json!({
                    "scope_type": bp.get::<String,_>("scope_type"),
                    "window_type": bp.get::<String,_>("window_type"),
                    "hard_cap_cents": bp.get::<i32,_>("hard_cap_cents"),
                    "soft_cap_cents": bp.get::<Option<i32>,_>("soft_cap_cents"),
                    "action_on_breach": bp.get::<String,_>("action_on_breach"),
                });
                if let Err(e) = add_json_entry(&mut tar, &path, &v) {
                    tracing::error!("mcp export tar {path}: {e}"); return err_result("tar_error");
                }
            }
            if let Err(e) = add_json_entry(&mut tar, "manifest.json", &manifest) {
                tracing::error!("mcp export tar manifest.json: {e}"); return err_result("tar_error");
            }

            let enc = match tar.into_inner() { Ok(e) => e, Err(e) => { tracing::error!("mcp export tar finish: {e}"); return err_result("tar_error"); } };
            let tarball_bytes = match enc.finish() { Ok(b) => b, Err(e) => { tracing::error!("mcp export gz finish: {e}"); return err_result("tar_error"); } };
            let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(&tarball_bytes);
            let sha256 = hex::encode(Sha256::new().chain_update(&tarball_bytes).finalize());
            let manifest_json = manifest.to_string();
            let pack_id = uuid::Uuid::new_v4().to_string();

            match sqlx::query(
                "INSERT INTO missionpack (id, owner_subject, name, version, sha256, tarball_b64, manifest_json, created_at, updated_at) \
                 VALUES ($1,$2,$3,1,$4,$5,$6,$7,$7) RETURNING id, name, sha256"
            )
            .bind(&pack_id).bind(&principal.subject).bind(&mission_name)
            .bind(&sha256).bind(&tarball_b64).bind(&manifest_json).bind(now)
            .fetch_one(&state.db).await {
                Ok(r) => ok_result(json!({"pack_id": r.get::<String,_>("id"), "name": r.get::<String,_>("name"), "sha256": r.get::<String,_>("sha256")})),
                Err(e) => { tracing::error!("mcp export_mission_pack insert: {e}"); err_result("database_error") }
            }
        }

        "install_mission_pack" => {
            use base64::Engine;
            let pack_id = str_arg(args, "pack_id");
            if pack_id.is_empty() { return err_result("pack_id is required"); }
            let target_mission_id = str_arg(args, "target_mission_id");

            let pack_row = sqlx::query(
                "SELECT * FROM missionpack WHERE id=$1 AND owner_subject=$2"
            )
            .bind(&pack_id).bind(&principal.subject)
            .fetch_optional(&state.db).await;
            let pack_row = match pack_row {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("pack not found"),
                Err(e) => { tracing::error!("mcp install_mission_pack: {e}"); return err_result("database_error"); }
            };

            let tarball_b64: String = pack_row.get("tarball_b64");
            let tarball_bytes = match base64::engine::general_purpose::STANDARD.decode(tarball_b64.as_bytes()) {
                Ok(b) => b,
                Err(_) => return err_result("pack tarball is corrupt (invalid base64)"),
            };

            // Extract tar in memory
            use std::io::Read;
            let cursor = std::io::Cursor::new(tarball_bytes);
            let gz = flate2::read::GzDecoder::new(cursor);
            let mut archive = tar::Archive::new(gz);

            let entries: Vec<(String, Vec<u8>)> = match archive.entries() {
                Ok(entries) => {
                    let mut out = Vec::new();
                    for e in entries {
                        if let Ok(mut entry) = e {
                            if let Ok(path) = entry.path() {
                                let name = path.to_string_lossy().into_owned();
                                let mut buf = Vec::new();
                                let _ = entry.read_to_end(&mut buf);
                                out.push((name, buf));
                            }
                        }
                    }
                    out
                }
                Err(e) => { tracing::error!("mcp install_mission_pack tar: {e}"); return err_result("pack tarball is corrupt"); }
            };

            // Parse manifest + mission
            let manifest: serde_json::Value = entries.iter()
                .find(|(n,_)| n == "manifest.json")
                .and_then(|(_,b)| serde_json::from_slice(b).ok())
                .unwrap_or(json!({}));
            let mission_spec: serde_json::Value = entries.iter()
                .find(|(n,_)| n == "mission.json")
                .and_then(|(_,b)| serde_json::from_slice(b).ok())
                .unwrap_or(json!({}));

            let mut created_missions: Vec<String> = vec![];
            let mut created_klusters: Vec<String> = vec![];
            let mut created_skills: Vec<String> = vec![];
            let mut created_budgets: Vec<String> = vec![];

            // Find or create mission
            let mission_id = if !target_mission_id.is_empty() {
                let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1")
                    .bind(&target_mission_id).fetch_optional(&state.db).await.unwrap_or(None);
                if exists.is_none() { return err_result("target mission not found"); }
                target_mission_id.clone()
            } else {
                let new_id = uuid::Uuid::new_v4().to_string();
                let pack_name: String = pack_row.get("name");
                let mission_name = format!("{pack_name} (from pack)");
                let desc = mission_spec.get("description").and_then(|v| v.as_str()).unwrap_or("");
                sqlx::query(
                    "INSERT INTO mission (id, owners, name, description, created_at, updated_at) \
                     VALUES ($1,$2,$3,$4,$5,$5)"
                )
                .bind(&new_id).bind(&principal.subject).bind(&mission_name)
                .bind(desc).bind(now)
                .execute(&state.db).await
                .map_err(|e| tracing::error!("mcp install mission create: {e}")).ok();
                created_missions.push(new_id.clone());
                new_id
            };

            // Insert klusters
            for (name, body) in entries.iter().filter(|(n,_)| n.starts_with("klusters/") && n.ends_with(".json")) {
                let spec: serde_json::Value = match serde_json::from_slice(body) { Ok(v) => v, Err(_) => continue };
                let kname = spec.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if kname.is_empty() { continue; }
                let exists: Option<String> = sqlx::query_scalar(
                    "SELECT id FROM kluster WHERE mission_id=$1 AND name=$2"
                )
                .bind(&mission_id).bind(kname).fetch_optional(&state.db).await.unwrap_or(None);
                if exists.is_none() {
                    let kid = uuid::Uuid::new_v4().to_string();
                    let kdesc = spec.get("description").and_then(|v| v.as_str()).unwrap_or("");
                    let _ = sqlx::query(
                        "INSERT INTO kluster (id, mission_id, name, description, owners, created_at, updated_at) \
                         VALUES ($1,$2,$3,$4,$5,$6,$6)"
                    )
                    .bind(&kid).bind(&mission_id).bind(kname).bind(kdesc)
                    .bind(&principal.subject).bind(now)
                    .execute(&state.db).await;
                    created_klusters.push(kid);
                }
                let _ = name; // suppress warning
            }

            // Insert skill bundles
            for (name, body) in entries.iter().filter(|(n,_)| n.starts_with("skills/") && n.ends_with(".json")) {
                let spec: serde_json::Value = match serde_json::from_slice(body) { Ok(v) => v, Err(_) => continue };
                let sha = spec.get("sha256").and_then(|v| v.as_str()).unwrap_or("");
                if sha.is_empty() { continue; }
                let exists: Option<String> = sqlx::query_scalar(
                    "SELECT id FROM skillbundle WHERE scope_type='mission' AND scope_id=$1 AND sha256=$2"
                )
                .bind(&mission_id).bind(sha).fetch_optional(&state.db).await.unwrap_or(None);
                if exists.is_none() {
                    let sbid = uuid::Uuid::new_v4().to_string();
                    let tarball = spec.get("tarball_b64").and_then(|v| v.as_str()).unwrap_or("");
                    let ver = spec.get("version").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
                    let _ = sqlx::query(
                        "INSERT INTO skillbundle (id, scope_type, scope_id, mission_id, version, tarball_b64, sha256, created_at, updated_at) \
                         VALUES ($1,'mission',$2,$2,$3,$4,$5,$6,$6)"
                    )
                    .bind(&sbid).bind(&mission_id).bind(ver).bind(tarball).bind(sha).bind(now)
                    .execute(&state.db).await;
                    created_skills.push(sbid);
                }
                let _ = name;
            }

            // Insert budget policies
            for (name, body) in entries.iter().filter(|(n,_)| n.starts_with("budgets/") && n.ends_with(".json")) {
                let spec: serde_json::Value = match serde_json::from_slice(body) { Ok(v) => v, Err(_) => continue };
                let window_type = spec.get("window_type").and_then(|v| v.as_str()).unwrap_or("day");
                let hard_cap = spec.get("hard_cap_cents").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let soft_cap = spec.get("soft_cap_cents").and_then(|v| v.as_i64()).map(|v| v as i32);
                let action = spec.get("action_on_breach").and_then(|v| v.as_str()).unwrap_or("alert_only");
                let bpid = uuid::Uuid::new_v4().to_string();
                let _ = sqlx::query(
                    "INSERT INTO budgetpolicy (id, owner_subject, scope_type, scope_id, window_type, \
                     hard_cap_cents, soft_cap_cents, action_on_breach, active, created_at, updated_at) \
                     VALUES ($1,$2,'mission',$3,$4,$5,$6,$7,true,$8,$8)"
                )
                .bind(&bpid).bind(&principal.subject).bind(&mission_id)
                .bind(window_type).bind(hard_cap).bind(soft_cap).bind(action).bind(now)
                .execute(&state.db).await;
                created_budgets.push(bpid);
                let _ = name;
            }

            ok_result(json!({
                "pack_id": pack_id,
                "mission_id": mission_id,
                "created": {"missions": created_missions, "klusters": created_klusters, "skills": created_skills, "budgets": created_budgets},
                "manifest": manifest,
            }))
        }

        // ── Provision persistence ─────────────────────────────────────────────

        "provision_mission_persistence" => {
            let mission_id = str_arg(args, "mission_id");
            if mission_id.is_empty() { return err_result("mission_id is required"); }

            let conn_input = args.get("connection").and_then(|v| v.as_object()).cloned().unwrap_or_default();
            let bind_input = args.get("binding").and_then(|v| v.as_object()).cloned().unwrap_or_default();
            let routes_input = args.get("routes").and_then(|v| v.as_array()).cloned().unwrap_or_default();

            let conn_name = conn_input.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let repo_path = conn_input.get("repo_path").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if conn_name.is_empty() || repo_path.is_empty() {
                return err_result("connection.name and connection.repo_path are required");
            }
            let provider = conn_input.get("provider").and_then(|v| v.as_str()).unwrap_or("github_app").to_string();
            let host = conn_input.get("host").and_then(|v| v.as_str()).unwrap_or("github.com").to_string();
            let default_branch = conn_input.get("default_branch").and_then(|v| v.as_str()).unwrap_or("main").to_string();
            let credential_ref = conn_input.get("credential_ref").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let options_json = conn_input.get("options").map(|v| v.to_string()).unwrap_or_else(|| "{}".into());

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
                Err(e) => { tracing::error!("mcp provision conn: {e}"); return err_result("database_error"); }
            };
            let conn_id: i32 = conn_row.get("id");

            // Upsert RepoBinding
            let bind_name = bind_input.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if bind_name.is_empty() { return err_result("binding.name is required"); }
            let branch_override = bind_input.get("branch_override").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let base_path = bind_input.get("base_path").and_then(|v| v.as_str()).unwrap_or("missions").trim_matches('/').to_string();
            let bind_active = bind_input.get("active").and_then(|v| v.as_bool()).unwrap_or(true);

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
                Err(e) => { tracing::error!("mcp provision binding: {e}"); return err_result("database_error"); }
            };
            let bind_id: i32 = bind_row.get("id");

            // Upsert MissionPersistencePolicy
            let fallback_mode = str_arg_or(args, "fallback_mode", "fail_closed");
            let require_approval = args.get("require_approval").and_then(|v| v.as_bool()).unwrap_or(false);
            let _ = sqlx::query(
                "INSERT INTO missionpersistencepolicy (mission_id, default_binding_id, fallback_mode, require_approval, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$5) \
                 ON CONFLICT (mission_id) DO UPDATE SET \
                 default_binding_id=$2, fallback_mode=$3, require_approval=$4, updated_at=$5"
            )
            .bind(&mission_id).bind(bind_id).bind(&fallback_mode).bind(require_approval).bind(now)
            .execute(&state.db).await;

            // Replace routes
            let _ = sqlx::query("DELETE FROM missionpersistenceroute WHERE mission_id=$1")
                .bind(&mission_id).execute(&state.db).await;

            for route in &routes_input {
                let target_name = route.get("binding_name")
                    .and_then(|v| v.as_str()).unwrap_or(&bind_name);
                let target_id: Option<i32> = if target_name == bind_name {
                    Some(bind_id)
                } else {
                    sqlx::query_scalar("SELECT id FROM repobinding WHERE owner_subject=$1 AND name=$2")
                        .bind(&principal.subject).bind(target_name)
                        .fetch_optional(&state.db).await.unwrap_or(None)
                };
                let Some(tid) = target_id else { continue; };
                let entity_kind = route.get("entity_kind").and_then(|v| v.as_str()).unwrap_or("");
                if entity_kind.is_empty() { continue; }
                let event_kind = route.get("event_kind").and_then(|v| v.as_str()).unwrap_or("");
                let route_branch = route.get("branch_override").and_then(|v| v.as_str()).unwrap_or("");
                let path_tpl = route.get("path_template").and_then(|v| v.as_str())
                    .unwrap_or("missions/{mission_id}/{entity_kind}/{entity_id}.json");
                let format = route.get("format").and_then(|v| v.as_str()).unwrap_or("json_v1");
                let active = route.get("active").and_then(|v| v.as_bool()).unwrap_or(true);
                let _ = sqlx::query(
                    "INSERT INTO missionpersistenceroute \
                     (mission_id, entity_kind, event_kind, binding_id, branch_override, path_template, format, active, created_at, updated_at) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9)"
                )
                .bind(&mission_id).bind(entity_kind).bind(event_kind).bind(tid)
                .bind(route_branch).bind(path_tpl).bind(format).bind(active).bind(now)
                .execute(&state.db).await;
            }

            let routes = sqlx::query(
                "SELECT * FROM missionpersistenceroute WHERE mission_id=$1 AND active=true ORDER BY id ASC"
            )
            .bind(&mission_id).fetch_all(&state.db).await.unwrap_or_default();

            ok_result(json!({
                "ok": true,
                "mission_id": mission_id,
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
                    "mission_id": r.get::<String,_>("mission_id"),
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
            let mission_id = str_arg(args, "mission_id");
            if mission_id.is_empty() { return err_result("mission_id is required"); }

            // Fetch pending events
            let events = sqlx::query(
                "SELECT * FROM ledgerevent WHERE mission_id=$1 AND state='pending' \
                 ORDER BY created_at ASC LIMIT 500"
            )
            .bind(&mission_id).fetch_all(&state.db).await;
            let events = match events {
                Ok(e) => e,
                Err(e) => { tracing::error!("mcp publish_ledger fetch: {e}"); return err_result("database_error"); }
            };
            if events.is_empty() {
                return ok_result(json!({"published_count": 0, "commit_sha": "", "branch": "", "repo_url": ""}));
            }

            // Get routing: binding + connection
            let route = sqlx::query(
                "SELECT r.path_template, r.format, r.event_kind, \
                 b.branch_override, b.base_path, \
                 c.host, c.repo_path, c.default_branch, c.credential_ref, c.provider \
                 FROM missionpersistenceroute r \
                 JOIN repobinding b ON b.id = r.binding_id \
                 JOIN repoconnection c ON c.id = b.connection_id \
                 WHERE r.mission_id=$1 AND r.active=true \
                 ORDER BY r.id ASC LIMIT 1"
            )
            .bind(&mission_id).fetch_optional(&state.db).await;
            let route = match route {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("no publish route configured for mission"),
                Err(e) => { tracing::error!("mcp publish_ledger route: {e}"); return err_result("database_error"); }
            };

            let host: String = route.get("host");
            let repo_path: String = route.get("repo_path");
            let default_branch: String = route.get("default_branch");
            let credential_ref: String = route.get("credential_ref");
            let branch: String = route.try_get("branch_override")
                .ok().filter(|s: &String| !s.is_empty())
                .unwrap_or_else(|| default_branch.clone());
            let path_tpl: String = route.get("path_template");

            // Resolve credential: "env:VAR_NAME" → token from env
            let token = if credential_ref.starts_with("env:") {
                std::env::var(&credential_ref[4..]).unwrap_or_default()
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
                Err(e) => { tracing::error!("mcp publish_ledger tempdir: {e}"); return err_result("internal_error"); }
            };
            let repo_dir = tmpdir.path().to_string_lossy().to_string();

            let clone_out = std::process::Command::new("git")
                .args(["clone", "--depth=1", "--branch", &branch, &repo_url, &repo_dir])
                .output();
            if let Err(e) = clone_out { tracing::error!("mcp publish_ledger clone: {e}"); return err_result("git_clone_failed"); }

            // Write entity files
            for event in &events {
                let entity_type: String = event.get("entity_type");
                let entity_id: String = event.get("entity_id");
                let payload: String = event.try_get("payload_json").unwrap_or_default();
                let rel = path_tpl
                    .replace("{mission_id}", &mission_id)
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
            let commit_msg = format!("mc-controlplane: publish {} ledger events for {}", events.len(), mission_id);
            let _ = std::process::Command::new("git")
                .args(["-C", &repo_dir, "config", "user.email", "git@ryanmerlin.com"])
                .output();
            let _ = std::process::Command::new("git")
                .args(["-C", &repo_dir, "config", "user.name", "mc-controlplane"])
                .output();
            let commit_out = std::process::Command::new("git")
                .args(["-C", &repo_dir, "commit", "--allow-empty", "-m", &commit_msg])
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
            if let Err(e) = push_out { tracing::error!("mcp publish_ledger push: {e}"); }

            // Update ledger events state
            let published_count = events.len() as i64;
            for event in &events {
                let eid: i32 = event.get("id");
                let entity_type: String = event.get("entity_type");
                let entity_id: String = event.get("entity_id");
                let rel = path_tpl
                    .replace("{mission_id}", &mission_id)
                    .replace("{entity_kind}", &entity_type)
                    .replace("{entity_id}", &entity_id);
                let _ = sqlx::query(
                    "UPDATE ledgerevent SET state='published', git_commit=$2, git_path=$3, published_at=$4, updated_at=$4 WHERE id=$1"
                )
                .bind(eid).bind(&commit_sha).bind(&rel).bind(now)
                .execute(&state.db).await;
            }

            let clean_repo_url = format!("https://{host}/{repo_path}");
            ok_result(json!({"published_count": published_count, "commit_sha": commit_sha, "branch": branch, "repo_url": clean_repo_url}))
        }

        // ── get_artifact_download_url — SigV4 presigned S3 URL ──────────────────
        "get_artifact_download_url" => {
            let artifact_id = int_arg(args, "artifact_id").unwrap_or(0) as i32;
            let expires: u64 = int_arg(args, "expires_seconds").unwrap_or(60).clamp(1, 3600) as u64;
            if artifact_id <= 0 { return err_result("artifact_id is required"); }

            // Look up artifact
            let artifact = sqlx::query("SELECT * FROM artifact WHERE id=$1")
                .bind(artifact_id).fetch_optional(&state.db).await;
            let artifact = match artifact {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Artifact not found"),
                Err(e) => { tracing::error!("get_artifact_download_url artifact: {e}"); return err_result("database_error"); }
            };
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
            let endpoint = std::env::var("MC_OBJECT_STORAGE_ENDPOINT").unwrap_or_default();
            let region = std::env::var("MC_OBJECT_STORAGE_REGION").unwrap_or_else(|_| "us-east-1".into());
            let access_key = std::env::var("MC_OBJECT_STORAGE_ACCESS_KEY")
                .or_else(|_| std::env::var("MC_OBJECT_STORAGE_KEY"))
                .unwrap_or_default();
            let secret_key = std::env::var("MC_OBJECT_STORAGE_ACCESS_SECRET")
                .or_else(|_| std::env::var("MC_OBJECT_STORAGE_SECRET"))
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
            let mut qparams = vec![
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

            let scheme = if std::env::var("MC_OBJECT_STORAGE_SECURE")
                .map(|v| v.to_lowercase() == "false" || v == "0")
                .unwrap_or(false)
            {
                "http"
            } else {
                "https"
            };
            let presigned_url = format!(
                "{scheme}://{host}{url_path}?{canonical_qs}&X-Amz-Signature={signature}"
            );

            ok_result(json!({"url": presigned_url, "expires_seconds": expires}))
        }

        // ── Workspace leases ──────────────────────────────────────────────────

        "load_kluster_workspace" => {
            let kluster_id = str_arg(args, "kluster_id");
            if kluster_id.is_empty() { return err_result("kluster_id is required"); }
            let lease_seconds = int_arg(args, "lease_seconds").unwrap_or(900).clamp(60, 3600) as i32;
            let workspace_label = str_arg(args, "workspace_label");
            let agent_id = str_arg(args, "agent_id");

            // Verify kluster exists and get its mission_id
            let kluster = sqlx::query("SELECT id, mission_id FROM kluster WHERE id=$1")
                .bind(&kluster_id).fetch_optional(&state.db).await;
            let kluster = match kluster {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Kluster not found"),
                Err(e) => { tracing::error!("mcp load_kluster_workspace kluster: {e}"); return err_result("database_error"); }
            };
            let mission_id: Option<String> = kluster.get("mission_id");
            let mission_id = match mission_id {
                Some(m) if !m.is_empty() => m,
                _ => return err_result("Kluster is not linked to a mission"),
            };

            // Build workspace snapshot from DB
            let snapshot = build_workspace_snapshot(&state.db, &mission_id, &kluster_id).await;

            // Create lease
            let lease_id = format!("wl-{}", &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]);
            let expires_at = now + chrono::Duration::seconds(lease_seconds as i64);
            let snapshot_index = serde_json::to_string(&snapshot.get("index").cloned().unwrap_or(json!({}))).unwrap_or_default();

            let lease = sqlx::query(
                "INSERT INTO workspacelease \
                 (id, mission_id, kluster_id, actor_subject, agent_id, workspace_label, \
                  status, base_snapshot_json, lease_seconds, last_heartbeat_at, expires_at, \
                  release_reason, created_at, updated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,'active',$7,$8,$9,$10,'',$9,$9) RETURNING *"
            )
            .bind(&lease_id).bind(&mission_id).bind(&kluster_id)
            .bind(&principal.subject).bind(&agent_id).bind(&workspace_label)
            .bind(&snapshot_index).bind(lease_seconds).bind(now).bind(expires_at)
            .fetch_one(&state.db).await;

            match lease {
                Ok(r) => ok_result(json!({
                    "lease": lease_row_to_json(&r),
                    "workspace_snapshot": snapshot,
                })),
                Err(e) => { tracing::error!("mcp load_kluster_workspace insert: {e}"); err_result("database_error") }
            }
        }

        "heartbeat_workspace_lease" => {
            let lease_id = str_arg(args, "lease_id");
            if lease_id.is_empty() { return err_result("lease_id is required"); }

            let lease = sqlx::query("SELECT * FROM workspacelease WHERE id=$1")
                .bind(&lease_id).fetch_optional(&state.db).await;
            let lease = match lease {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Workspace lease not found"),
                Err(e) => { tracing::error!("mcp heartbeat lease: {e}"); return err_result("database_error"); }
            };

            let owner: String = lease.get("actor_subject");
            if owner != principal.subject && !principal.is_admin { return err_result("forbidden"); }
            let status: String = lease.get("status");
            if status != "active" { return err_result("Workspace lease is not active"); }
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
            if lease_id.is_empty() { return err_result("lease_id is required"); }
            if artifact_id <= 0 { return err_result("artifact_id is required"); }

            let lease = sqlx::query("SELECT * FROM workspacelease WHERE id=$1")
                .bind(&lease_id).fetch_optional(&state.db).await;
            let lease = match lease {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Workspace lease not found"),
                Err(e) => { tracing::error!("mcp fetch_workspace_artifact lease: {e}"); return err_result("database_error"); }
            };
            let owner: String = lease.get("actor_subject");
            if owner != principal.subject && !principal.is_admin { return err_result("forbidden"); }
            let lease_status: String = lease.get("status");
            if lease_status != "active" { return err_result("Workspace lease is not active"); }
            let lease_kluster: String = lease.get("kluster_id");

            let artifact = sqlx::query("SELECT * FROM artifact WHERE id=$1")
                .bind(artifact_id).fetch_optional(&state.db).await;
            let artifact = match artifact {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Artifact not found"),
                Err(e) => { tracing::error!("mcp fetch_workspace_artifact artifact: {e}"); return err_result("database_error"); }
            };
            let art_kluster: String = artifact.get("kluster_id");
            if art_kluster != lease_kluster { return err_result("Artifact is outside lease kluster scope"); }

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
                        let size = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes())
                            .map(|b| b.len()).unwrap_or(0);
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
                    return err_result("Artifact is not S3-backed — use content mode or Python API for download URL");
                }
                err_result("S3 presigned URL generation requires Python API")
            }
        }

        "commit_kluster_workspace" => {
            let lease_id = str_arg(args, "lease_id");
            if lease_id.is_empty() { return err_result("lease_id is required"); }
            let changes = match args.get("change_set").and_then(|v| v.as_array()) {
                Some(c) if !c.is_empty() => c.clone(),
                _ => return err_result("change_set must be a non-empty array"),
            };

            let lease = sqlx::query("SELECT * FROM workspacelease WHERE id=$1")
                .bind(&lease_id).fetch_optional(&state.db).await;
            let lease = match lease {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Workspace lease not found"),
                Err(e) => { tracing::error!("mcp commit_workspace lease: {e}"); return err_result("database_error"); }
            };
            let owner: String = lease.get("actor_subject");
            if owner != principal.subject && !principal.is_admin { return err_result("forbidden"); }
            let lease_status: String = lease.get("status");
            if lease_status != "active" { return err_result("Workspace lease is not active"); }
            let kluster_id: String = lease.get("kluster_id");
            let mission_id: String = lease.get("mission_id");

            let mut applied_count = 0i64;
            let mut applied: Vec<serde_json::Value> = vec![];

            for change in &changes {
                let entity_type = change.get("entity_type").and_then(|v| v.as_str()).unwrap_or("");
                let entity_id = change.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");

                match entity_type {
                    "doc" => {
                        let doc_id: i32 = match entity_id.parse() { Ok(v) => v, Err(_) => continue };
                        let doc = sqlx::query("SELECT * FROM doc WHERE id=$1 AND kluster_id=$2")
                            .bind(doc_id).bind(&kluster_id).fetch_optional(&state.db).await;
                        let doc = match doc { Ok(Some(r)) => r, _ => continue };
                        let mut sets: Vec<String> = vec![];
                        let mut new_title: Option<String> = None;
                        let mut new_body: Option<String> = None;
                        let mut new_doc_type: Option<String> = None;
                        let mut new_status: Option<String> = None;
                        if let Some(v) = change.get("content").and_then(|v| v.as_str()) {
                            new_body = Some(v.to_string()); sets.push("body=$IDX".to_string());
                        }
                        if let Some(v) = change.get("title").and_then(|v| v.as_str()) {
                            new_title = Some(v.to_string()); sets.push("title=$IDX".to_string());
                        }
                        if let Some(v) = change.get("doc_type").and_then(|v| v.as_str()) {
                            new_doc_type = Some(v.to_string()); sets.push("doc_type=$IDX".to_string());
                        }
                        if let Some(v) = change.get("status").and_then(|v| v.as_str()) {
                            new_status = Some(v.to_string()); sets.push("status=$IDX".to_string());
                        }
                        if sets.is_empty() { continue; }
                        let cur_version: i32 = doc.try_get("version").unwrap_or(1);
                        let sql = format!(
                            "UPDATE doc SET version={}, updated_at=$1, {} WHERE id=$2",
                            cur_version + 1,
                            sets.iter().enumerate().map(|(i, s)| s.replace("$IDX", &format!("${}", i + 3))).collect::<Vec<_>>().join(", ")
                        );
                        let mut q = sqlx::query(&sql).bind(now).bind(doc_id);
                        if let Some(v) = &new_body  { q = q.bind(v); }
                        if let Some(v) = &new_title { q = q.bind(v); }
                        if let Some(v) = &new_doc_type { q = q.bind(v); }
                        if let Some(v) = &new_status { q = q.bind(v); }
                        let _ = q.execute(&state.db).await;
                        applied.push(json!({"entity_type": "doc", "entity_id": doc_id, "version": cur_version + 1}));
                        applied_count += 1;
                    }
                    "artifact" => {
                        let art_id: i32 = match entity_id.parse() { Ok(v) => v, Err(_) => continue };
                        let art = sqlx::query("SELECT * FROM artifact WHERE id=$1 AND kluster_id=$2")
                            .bind(art_id).bind(&kluster_id).fetch_optional(&state.db).await;
                        let art = match art { Ok(Some(r)) => r, _ => continue };
                        let cur_version: i32 = art.try_get("version").unwrap_or(1);
                        // Only field updates — skip S3 content_b64 upload
                        let fields = change.get("fields").and_then(|v| v.as_object());
                        let mut parts: Vec<String> = vec![];
                        let mut vals: Vec<String> = vec![];
                        for key in ["name","artifact_type","uri","storage_backend","content_sha256","mime_type","status","provenance"] {
                            if let Some(v) = fields.and_then(|f| f.get(key)).and_then(|v| v.as_str()) {
                                parts.push(format!("{}=$IDX", key));
                                vals.push(v.to_string());
                            }
                        }
                        let sql = format!(
                            "UPDATE artifact SET version={}, updated_at=$1, {} WHERE id=$2",
                            cur_version + 1,
                            parts.iter().enumerate().map(|(i, s)| s.replace("$IDX", &format!("${}", i + 3))).collect::<Vec<_>>().join(", ")
                        );
                        if !parts.is_empty() {
                            let mut q = sqlx::query(&sql).bind(now).bind(art_id);
                            for v in &vals { q = q.bind(v); }
                            let _ = q.execute(&state.db).await;
                        }
                        applied.push(json!({"entity_type": "artifact", "entity_id": art_id, "version": cur_version + 1}));
                        applied_count += 1;
                    }
                    _ => {}
                }

                // Enqueue ledger event
                let event_id = format!("le-{}", &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]);
                let _ = sqlx::query(
                    "INSERT INTO ledgerevent (event_id, mission_id, kluster_id, entity_type, entity_id, \
                     action, payload_json, state, created_by_subject, created_at, updated_at) \
                     VALUES ($1,$2,$3,$4,$5,'workspace_commit','{}'::text,'pending',$6,$7,$7)"
                )
                .bind(&event_id).bind(&mission_id).bind(&kluster_id)
                .bind(entity_type).bind(entity_id).bind(&principal.subject).bind(now)
                .execute(&state.db).await;
            }

            let snapshot = build_workspace_snapshot(&state.db, &mission_id, &kluster_id).await;
            ok_result(json!({"applied_count": applied_count, "applied": applied, "workspace_snapshot": snapshot}))
        }

        "release_kluster_workspace" => {
            let lease_id = str_arg(args, "lease_id");
            let reason = str_arg(args, "reason");
            if lease_id.is_empty() { return err_result("lease_id is required"); }

            let lease = sqlx::query("SELECT * FROM workspacelease WHERE id=$1")
                .bind(&lease_id).fetch_optional(&state.db).await;
            let lease = match lease {
                Ok(Some(r)) => r,
                Ok(None) => return err_result("Workspace lease not found"),
                Err(e) => { tracing::error!("mcp release_workspace lease: {e}"); return err_result("database_error"); }
            };
            let owner: String = lease.get("actor_subject");
            if owner != principal.subject && !principal.is_admin { return err_result("forbidden"); }
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

        _ => err_result("unknown_tool"),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn issue_mcp_session(
    db: &sqlx::PgPool,
    subject: &str,
    ttl_hours: i64,
    capability_scope: &str,
) -> Result<(i32, String), sqlx::Error> {
    use rand::RngCore;
    use sha2::{Digest, Sha256};
    let mut raw_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw_bytes);
    let raw_token = format!("mcs_{}", hex::encode(&raw_bytes));
    let token_hash = hex::encode(Sha256::new().chain_update(raw_token.as_bytes()).finalize());
    let token_prefix = &raw_token[..8.min(raw_token.len())];
    let expires_at = chrono::Utc::now().naive_utc() + chrono::Duration::hours(ttl_hours);
    let now = chrono::Utc::now().naive_utc();
    let session_id: i32 = sqlx::query_scalar(
        "INSERT INTO usersession \
         (subject, token_hash, token_prefix, expires_at, created_at, last_used_at, user_agent, revoked, capability_scope) \
         VALUES ($1,$2,$3,$4,$5,$5,'mc-mcp',false,$6) RETURNING id",
    )
    .bind(subject).bind(&token_hash).bind(token_prefix)
    .bind(expires_at).bind(now).bind(capability_scope)
    .fetch_one(db).await?;
    Ok((session_id, raw_token))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> String {
    args.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn str_arg_or<'a>(args: &'a Value, key: &str, default: &str) -> String {
    let v = args.get(key).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if v.is_empty() { default.to_string() } else { v }
}

fn int_arg(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

fn lease_row_to_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<String,_>("id"),
        "mission_id": r.get::<String,_>("mission_id"),
        "kluster_id": r.get::<String,_>("kluster_id"),
        "actor_subject": r.get::<String,_>("actor_subject"),
        "agent_id": r.get::<String,_>("agent_id"),
        "workspace_label": r.get::<String,_>("workspace_label"),
        "status": r.get::<String,_>("status"),
        "lease_seconds": r.get::<i32,_>("lease_seconds"),
        "last_heartbeat_at": r.get::<chrono::NaiveDateTime,_>("last_heartbeat_at"),
        "expires_at": r.get::<chrono::NaiveDateTime,_>("expires_at"),
    })
}

async fn build_workspace_snapshot(db: &sqlx::PgPool, mission_id: &str, kluster_id: &str) -> Value {
    let tasks = sqlx::query(
        "SELECT id, public_id, title, description, status, owner, priority, labels, updated_at \
         FROM task WHERE kluster_id=$1 AND status NOT IN ('done','cancelled') ORDER BY updated_at DESC LIMIT 200"
    )
    .bind(kluster_id).fetch_all(db).await.unwrap_or_default();

    let docs = sqlx::query(
        "SELECT id, title, doc_type, status, version, updated_at FROM doc WHERE kluster_id=$1 ORDER BY updated_at DESC LIMIT 100"
    )
    .bind(kluster_id).fetch_all(db).await.unwrap_or_default();

    let artifacts = sqlx::query(
        "SELECT id, name, artifact_type, uri, storage_backend, mime_type, size_bytes, status, version, updated_at \
         FROM artifact WHERE kluster_id=$1 ORDER BY updated_at DESC LIMIT 100"
    )
    .bind(kluster_id).fetch_all(db).await.unwrap_or_default();

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
        "mission_id": mission_id,
        "kluster_id": kluster_id,
        "tasks": tasks.iter().map(|r| json!({
            "id": r.get::<i32,_>("id"),
            "public_id": r.try_get::<String,_>("public_id").unwrap_or_default(),
            "title": r.get::<String,_>("title"),
            "description": r.try_get::<String,_>("description").unwrap_or_default(),
            "status": r.get::<String,_>("status"),
            "owner": r.try_get::<String,_>("owner").unwrap_or_default(),
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

fn profile_row_to_json(r: &sqlx::postgres::PgRow) -> Value {
    let manifest_json: String = r.try_get("manifest_json").unwrap_or_default();
    let manifest: Value = serde_json::from_str(&manifest_json).unwrap_or(json!([]));
    json!({
        "id": r.get::<i32,_>("id"),
        "name": r.get::<String,_>("name"),
        "owner_subject": r.get::<String,_>("owner_subject"),
        "description": r.try_get::<String,_>("description").unwrap_or_default(),
        "is_default": r.try_get::<bool,_>("is_default").unwrap_or(false),
        "manifest": manifest,
        "sha256": r.try_get::<String,_>("sha256").unwrap_or_default(),
        "size_bytes": r.try_get::<i32,_>("size_bytes").unwrap_or(0),
        "mirror_uri": r.try_get::<String,_>("mirror_uri").unwrap_or_default(),
        "mirror_sha256": r.try_get::<String,_>("mirror_sha256").unwrap_or_default(),
        "mirror_size_bytes": r.try_get::<i32,_>("mirror_size_bytes").unwrap_or(0),
        "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
        "updated_at": r.get::<chrono::NaiveDateTime,_>("updated_at"),
    })
}

fn task_row_to_json(r: &sqlx::postgres::PgRow) -> Value {
    json!({
        "id": r.get::<i32,_>("id"),
        "kluster_id": r.get::<String,_>("kluster_id"),
        "title": r.get::<String,_>("title"),
        "description": r.get::<String,_>("description"),
        "status": r.get::<String,_>("status"),
        "owner": r.get::<String,_>("owner"),
    })
}
