use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{
    auth::Principal,
    models::agent::{Agent, AgentCreate, AgentIdent, AgentMessage, AgentUpdate, MessageSend},
    state::AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/{agent_id}", get(get_agent).patch(update_agent).delete(delete_agent))
        .route("/agents/{agent_id}/restart", post(restart_agent))
        .route("/agents/{agent_id}/clear-context", post(clear_agent_context))
        .route("/agents/{agent_id}/domain", patch(attach_domain))
        .route("/agents/{agent_id}/message", post(send_message))
        .route("/agents/{agent_id}/messages", get(list_messages))
}

fn row_to_agent(row: &sqlx::postgres::PgRow) -> Agent {
    let metadata: String = row.get("metadata");
    let (runtime, node_id) = extract_metadata_fields(&metadata);
    Agent {
        id: row.get("id"),
        public_id: row.get("public_id"),
        name: row.get("name"),
        capabilities: row.get("capabilities"),
        status: row.get("status"),
        metadata,
        home_domain_id: row.try_get("home_domain_id").unwrap_or(None),
        current_domain_id: row.try_get("current_domain_id").unwrap_or(None),
        domain_name: row.try_get("domain_name").unwrap_or(None),
        runtime,
        node_id,
        runtime_node_id: row.try_get("runtime_node_id").unwrap_or(None),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn extract_metadata_fields(metadata: &str) -> (Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(metadata) else {
        return (None, None);
    };
    let runtime = v["runtime"].as_str().map(String::from);
    let node_id = v["node_id"].as_str().map(String::from);
    (runtime, node_id)
}

/// Generate a fresh public_id for a new agent row. Shape is
/// `{name}-{8-hex-chars}` — readable in CLI/TUI output, and unique enough
/// across delete-and-recreate cycles that a re-registered agent
/// doesn't collide with the previous one's identifier. Used only on the
/// INSERT side of `create_agent`; the UPDATE path preserves the existing
/// value because public_id is immutable after creation.
fn generate_public_id(name: &str) -> String {
    let raw = uuid::Uuid::new_v4().to_string();
    // First 8 hex chars of the UUID, dashes stripped — matches the migration
    // backfill shape (md5-substr 1,8) so tooling that scans for the format
    // doesn't have to special-case provenance.
    let suffix: String = raw.chars().filter(|c| *c != '-').take(8).collect();
    format!("{name}-{suffix}")
}

/// Upsert an agent row by name and return its `public_id`. Used by every
/// meshagent enrollment path to link a topology row to a persistent agent
/// identity: see `docs/plans/2026-05-11-agent-public-id-edgeplaned-fix.md`.
///
/// Semantics mirror `create_agent`: re-upsertting refreshes `capabilities`
/// and `updated_at`, un-archives the row, and preserves `public_id` (so the
/// wire identifier edgeplaned stores stays stable across re-enrollments).
/// Rejects reserved names (anonymous, system:*) and surfaces the row's
/// status as `offline` on first creation — runtimes flip it to `online`
/// when they actually start.
pub async fn upsert_agent_by_name(
    db: &sqlx::PgPool,
    name: &str,
    capabilities: &str,
) -> anyhow::Result<String> {
    if is_reserved_agent_name(name) {
        anyhow::bail!("reserved agent name");
    }
    let now = chrono::Utc::now().naive_utc();
    let public_id = generate_public_id(name);
    let row = sqlx::query(
        "INSERT INTO agent \
            (name, capabilities, status, metadata, created_at, updated_at, last_seen_at, public_id) \
         VALUES ($1,$2,'offline','{}',$3,$3,$3,$4) \
         ON CONFLICT (name) DO UPDATE SET \
            capabilities = EXCLUDED.capabilities, \
            updated_at   = EXCLUDED.updated_at, \
            last_seen_at = EXCLUDED.last_seen_at, \
            archived_at  = NULL \
         RETURNING public_id",
    )
    .bind(name)
    .bind(capabilities)
    .bind(now)
    .bind(&public_id)
    .fetch_one(db)
    .await?;
    Ok(row.get::<String, _>("public_id"))
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

/// Resolve an [`AgentIdent`] to the internal numeric row id, or return a
/// response (404 / 500) the caller can short-circuit on. Keeps the
/// resolve-or-render pattern out of every handler body.
async fn resolve_agent_or_404(
    ident: &AgentIdent,
    db: &sqlx::PgPool,
) -> Result<i32, axum::response::Response> {
    match ident.resolve_id(db).await {
        Ok(Some(id)) => Ok(id),
        Ok(None) => Err(not_found("Agent not found")),
        Err(e) => {
            tracing::error!("agent ident resolve {}: {e}", ident.as_display());
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// Reserved agent names that callers may not register. `anonymous` was the
/// historical sink for unauthenticated hook callers (see routes/hooks.rs);
/// the `system:` prefix is held for future controlplane-internal agents.
pub fn is_reserved_agent_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.eq_ignore_ascii_case("anonymous")
        || trimmed.starts_with("system:")
        || trimmed.starts_with("system/")
}

#[derive(Deserialize)]
struct ListQuery {
    status: Option<String>,
    limit: Option<i64>,
    /// Return only messages with id > since_id. Used by edgeplaned message relay to
    /// avoid re-delivering already-seen messages across process restarts.
    since_id: Option<i64>,
    /// When true, list_agents returns archived rows alongside live ones. Off
    /// by default so the steady-state TUI/CLI views stay focused. Phase 1 of
    /// the agent-identity spec.
    #[serde(default)]
    include_archived: bool,
}

// ── Agents ────────────────────────────────────────────────────────────────────

async fn list_agents(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    let archived_clause = if q.include_archived { "" } else { " AND a.archived_at IS NULL" };
    let rows = if let Some(s) = &q.status {
        let sql = format!(
            "SELECT a.*, m.name AS domain_name \
             FROM agent a \
             LEFT JOIN domain m ON m.id = a.current_domain_id \
             WHERE a.status=$1{archived_clause} ORDER BY a.updated_at DESC LIMIT $2"
        );
        sqlx::query(&sql).bind(s).bind(limit).fetch_all(&state.db).await
    } else {
        let sql = format!(
            "SELECT a.*, m.name AS domain_name \
             FROM agent a \
             LEFT JOIN domain m ON m.id = a.current_domain_id \
             WHERE 1=1{archived_clause} ORDER BY a.updated_at DESC LIMIT $1"
        );
        sqlx::query(&sql).bind(limit).fetch_all(&state.db).await
    };
    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_agent).collect::<Vec<_>>()).into_response(),
        Err(e) => { tracing::error!("list_agents: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn create_agent(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Json(payload): Json<AgentCreate>,
) -> impl IntoResponse {
    if is_reserved_agent_name(&payload.name) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"detail": "Reserved agent name"})),
        )
            .into_response();
    }

    let now = Utc::now().naive_utc();
    let new_public_id = generate_public_id(&payload.name);
    let result = sqlx::query(
        "INSERT INTO agent \
            (name, capabilities, status, metadata, created_at, updated_at, last_seen_at, public_id) \
         VALUES ($1,$2,$3,$4,$5,$5,$5,$6) \
         ON CONFLICT (name) DO UPDATE SET \
            capabilities = EXCLUDED.capabilities, \
            status       = EXCLUDED.status, \
            metadata     = EXCLUDED.metadata, \
            updated_at   = EXCLUDED.updated_at, \
            last_seen_at = EXCLUDED.last_seen_at, \
            archived_at  = NULL \
         RETURNING *"
    )
    .bind(&payload.name).bind(&payload.capabilities).bind(&payload.status)
    .bind(&payload.metadata).bind(now).bind(&new_public_id)
    .fetch_one(&state.db).await;

    let row = match result {
        Ok(row) => row,
        Err(e) => { tracing::error!("create_agent: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let agent_id: i32 = row.get("id");
    let home_domain_id: Option<String> = row.try_get("home_domain_id").unwrap_or(None);

    // Auto-provision a home domain + Inbox mission on first registration.
    if home_domain_id.is_none() {
        match provision_home_domain(&state.db, agent_id, &payload.name).await {
            Ok(domain_id) => {
                // Re-fetch so the response includes the new home/current domain fields.
                match sqlx::query(
                    "SELECT a.*, m.name AS domain_name \
                     FROM agent a LEFT JOIN domain m ON m.id = a.current_domain_id \
                     WHERE a.id=$1"
                ).bind(agent_id).fetch_one(&state.db).await {
                    Ok(refreshed) => return (StatusCode::OK, Json(row_to_agent(&refreshed))).into_response(),
                    Err(e) => {
                        tracing::error!("create_agent re-fetch after provision: {e}");
                        // Return the original row without domain fields rather than failing.
                    }
                }
                let _ = domain_id;
            }
            Err(e) => {
                tracing::error!("create_agent home domain provision for {}: {e}", payload.name);
                // Non-fatal — agent is registered, domain can be provisioned by backfill.
            }
        }
    }

    (StatusCode::OK, Json(row_to_agent(&row))).into_response()
}

/// Create a home domain + Inbox mission for an agent that has none, then
/// set both `home_domain_id` and `current_domain_id` on the agent row.
/// Wrapped in a transaction so a partial provision can't leave orphaned rows.
pub async fn provision_home_domain(db: &sqlx::PgPool, agent_id: i32, agent_name: &str) -> anyhow::Result<String> {
    let now = chrono::Utc::now().naive_utc();

    // Generate a candidate domain id (6-byte hex, same pattern as domains.rs).
    let mut candidate_id = hex_id();
    for _ in 0..5 {
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM domain WHERE id=$1")
            .bind(&candidate_id).fetch_optional(db).await.unwrap_or(None);
        if exists.is_none() { break; }
        candidate_id = hex_id();
    }

    let mut tx = db.begin().await?;

    // ON CONFLICT (name) DO NOTHING makes this idempotent: if a domain with
    // this agent's name already exists (e.g. from a prior boot), the INSERT is
    // silently skipped and we fetch the pre-existing id below.
    sqlx::query(
        "INSERT INTO domain \
            (id, name, description, owners, contributors, tags, visibility, status, \
             northstar_md, northstar_version, northstar_created_by, northstar_modified_by, \
             northstar_created_at, northstar_modified_at, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,'','','public','active','',1,'','',NULL,NULL,$5,$5) \
         ON CONFLICT (name) DO NOTHING"
    )
    .bind(&candidate_id)
    .bind(agent_name)
    .bind(format!("Home domain for {agent_name}"))
    .bind(agent_name)
    .bind(now)
    .execute(&mut *tx).await?;

    // Resolve the actual domain_id — may differ from candidate_id if the domain
    // already existed and the INSERT was a no-op.
    let domain_id: String = sqlx::query_scalar("SELECT id FROM domain WHERE name=$1")
        .bind(agent_name)
        .fetch_one(&mut *tx)
        .await?;

    // Inbox mission under the home domain (non-conflicting; no unique constraint).
    let mission_id = hex_id();
    sqlx::query(
        "INSERT INTO mission \
            (id, domain_id, name, description, owners, contributors, tags, status, \
             workstream_md, workstream_version, workstream_created_by, workstream_modified_by, \
             workstream_created_at, workstream_modified_at, created_at, updated_at) \
         VALUES ($1,$2,'Inbox','Default inbox mission',$3,'','','active','',1,'','',NULL,NULL,$4,$4)"
    )
    .bind(&mission_id)
    .bind(&domain_id)
    .bind(agent_name)
    .bind(now)
    .execute(&mut *tx).await?;

    sqlx::query(
        "UPDATE agent SET home_domain_id=$1, current_domain_id=$1 WHERE id=$2"
    )
    .bind(&domain_id)
    .bind(agent_id)
    .execute(&mut *tx).await?;

    tx.commit().await?;

    tracing::info!(agent_id, domain_id = %domain_id, "provisioned home domain");
    Ok(domain_id)
}

fn hex_id() -> String {
    let bytes: [u8; 6] = rand::random();
    hex::encode(bytes)
}

async fn get_agent(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    match sqlx::query(
        "SELECT a.*, m.name AS domain_name, \
               (SELECT ma.runtime_node_id FROM meshagent ma \
                WHERE ma.agent_public_id = a.public_id AND ma.runtime_node_id IS NOT NULL \
                ORDER BY ma.enrolled_at DESC LIMIT 1) AS runtime_node_id \
         FROM agent a LEFT JOIN domain m ON m.id = a.current_domain_id \
         WHERE a.id=$1"
    ).bind(agent_id).fetch_optional(&state.db).await {
        Ok(Some(row)) => Json(row_to_agent(&row)).into_response(),
        Ok(None) => not_found("Agent not found"),
        Err(e) => { tracing::error!("get_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn delete_agent(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(ident): Path<AgentIdent>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_by_control_plane_agent(&state.db, &principal, agent_id).await
    {
        return resp;
    }
    match sqlx::query("DELETE FROM agent WHERE id=$1")
        .bind(agent_id).execute(&state.db).await {
        Ok(r) if r.rows_affected() == 0 => not_found("Agent not found"),
        Ok(_) => (StatusCode::NO_CONTENT, ()).into_response(),
        Err(e) => { tracing::error!("delete_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

/// Mark all open sessions ended with reason="restart_requested" and set the
/// agent offline. The controlplane only signals — runtimes are responsible for
/// observing the state change and actually restarting the agent process.
async fn restart_agent(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(ident): Path<AgentIdent>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_by_control_plane_agent(&state.db, &principal, agent_id).await
    {
        return resp;
    }

    let now = Utc::now().naive_utc();
    let _ = sqlx::query(
        "UPDATE agentsession SET ended_at=$2, end_reason='restart_requested' \
         WHERE agent_id=$1 AND ended_at IS NULL"
    ).bind(agent_id).bind(now).execute(&state.db).await;

    match sqlx::query("UPDATE agent SET status='offline', updated_at=$2 WHERE id=$1 RETURNING *")
        .bind(agent_id).bind(now).fetch_one(&state.db).await {
        Ok(row) => (StatusCode::OK, Json(row_to_agent(&row))).into_response(),
        Err(e) => { tracing::error!("restart_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

/// Stamp `metadata.last_context_clear_at` so listening runtimes can observe
/// the request and reset their own conversation state. The controlplane is
/// otherwise opaque to context content.
async fn clear_agent_context(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(ident): Path<AgentIdent>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_by_control_plane_agent(&state.db, &principal, agent_id).await
    {
        return resp;
    }
    let existing = sqlx::query("SELECT * FROM agent WHERE id=$1")
        .bind(agent_id).fetch_optional(&state.db).await;
    let agent = match existing {
        Ok(Some(r)) => row_to_agent(&r),
        Ok(None) => return not_found("Agent not found"),
        Err(e) => { tracing::error!("clear_agent_context fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let now = Utc::now().naive_utc();
    let now_iso = Utc::now().to_rfc3339();
    // metadata is text in the schema; parse-merge-serialize so we don't clobber
    // unrelated keys other systems may have written.
    let mut meta_obj: serde_json::Map<String, serde_json::Value> =
        if agent.metadata.is_empty() {
            serde_json::Map::new()
        } else {
            serde_json::from_str::<serde_json::Value>(&agent.metadata)
                .ok()
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default()
        };
    meta_obj.insert("last_context_clear_at".into(), serde_json::Value::String(now_iso));
    let new_metadata = serde_json::Value::Object(meta_obj).to_string();

    match sqlx::query("UPDATE agent SET metadata=$2, updated_at=$3 WHERE id=$1 RETURNING *")
        .bind(agent_id).bind(&new_metadata).bind(now).fetch_one(&state.db).await {
        Ok(row) => (StatusCode::OK, Json(row_to_agent(&row))).into_response(),
        Err(e) => { tracing::error!("clear_agent_context: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn update_agent(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(ident): Path<AgentIdent>,
    Json(payload): Json<AgentUpdate>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_by_control_plane_agent(&state.db, &principal, agent_id).await
    {
        return resp;
    }
    let existing = sqlx::query("SELECT * FROM agent WHERE id=$1")
        .bind(agent_id).fetch_optional(&state.db).await;
    let agent = match existing {
        Ok(Some(r)) => row_to_agent(&r),
        Ok(None) => return not_found("Agent not found"),
        Err(e) => { tracing::error!("update_agent fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let name         = payload.name.unwrap_or(agent.name);
    let capabilities = payload.capabilities.unwrap_or(agent.capabilities);
    let status       = payload.status.unwrap_or(agent.status);
    let metadata     = payload.metadata.unwrap_or(agent.metadata);
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "UPDATE agent SET name=$2, capabilities=$3, status=$4, metadata=$5, updated_at=$6 WHERE id=$1 RETURNING *"
    )
    .bind(agent_id).bind(&name).bind(&capabilities).bind(&status).bind(&metadata).bind(now)
    .fetch_one(&state.db).await {
        Ok(row) => Json(row_to_agent(&row)).into_response(),
        Err(e) => { tracing::error!("update_agent: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

// ── Domain attachment ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct AttachDomain {
    /// Target domain id. Omit (or null) to detach — resets current_domain_id to home_domain_id.
    domain_id: Option<String>,
}

async fn attach_domain(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
    Json(payload): Json<AttachDomain>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let new_current_id: Option<String> = match &payload.domain_id {
        Some(mid) => {
            // Validate domain exists.
            let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM domain WHERE id=$1")
                .bind(mid).fetch_optional(&state.db).await.unwrap_or(None);
            if exists.is_none() { return not_found("Domain not found"); }
            Some(mid.clone())
        }
        None => {
            // Detach: return to home.
            sqlx::query_scalar::<_, String>("SELECT home_domain_id FROM agent WHERE id=$1")
                .bind(agent_id).fetch_optional(&state.db).await.unwrap_or(None)
        }
    };

    let now = Utc::now().naive_utc();
    match sqlx::query(
        "UPDATE agent SET current_domain_id=$1, updated_at=$2 WHERE id=$3 RETURNING id"
    )
    .bind(&new_current_id).bind(now).bind(agent_id)
    .fetch_optional(&state.db).await {
        Ok(None) => not_found("Agent not found"),
        Ok(_) => {
            match sqlx::query(
                "SELECT a.*, m.name AS domain_name \
                 FROM agent a LEFT JOIN domain m ON m.id = a.current_domain_id \
                 WHERE a.id=$1"
            ).bind(agent_id).fetch_one(&state.db).await {
                Ok(row) => Json(row_to_agent(&row)).into_response(),
                Err(e) => { tracing::error!("attach_domain re-fetch: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
            }
        }
        Err(e) => { tracing::error!("attach_domain: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

// ── Messages ──────────────────────────────────────────────────────────────────

async fn send_message(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(from_ident): Path<AgentIdent>,
    Json(payload): Json<MessageSend>,
) -> impl IntoResponse {
    let from_id = match resolve_agent_or_404(&from_ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let to_id = match payload.to_agent_id.resolve_id(&state.db).await {
        Ok(Some(id)) => id,
        Ok(None) => return not_found("Recipient agent not found"),
        Err(e) => {
            tracing::error!("send_message resolve to_agent_id: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, AgentMessage>(
        "INSERT INTO agentmessage (from_agent_id, to_agent_id, content, message_type, task_id, read, created_at) \
         VALUES ($1,$2,$3,$4,$5,false,$6) RETURNING *"
    )
    .bind(from_id).bind(to_id).bind(&payload.content)
    .bind(&payload.message_type).bind(payload.task_id).bind(now)
    .fetch_one(&state.db).await {
        Ok(m) => (StatusCode::OK, Json(m)).into_response(),
        Err(e) => { tracing::error!("send_message: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn list_messages(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(ident): Path<AgentIdent>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let agent_id = match resolve_agent_or_404(&ident, &state.db).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let limit = q.limit.unwrap_or(50).min(200);
    let since_id = q.since_id.unwrap_or(0);
    match sqlx::query_as::<_, AgentMessage>(
        "SELECT * FROM agentmessage \
         WHERE (from_agent_id=$1 OR to_agent_id=$1) AND id > $3 \
         ORDER BY id ASC LIMIT $2"
    )
    .bind(agent_id).bind(limit).bind(since_id).fetch_all(&state.db).await {
        Ok(msgs) => Json(msgs).into_response(),
        Err(e) => { tracing::error!("list_messages: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

#[cfg(test)]
mod reserved_name_tests {
    use super::is_reserved_agent_name;

    #[test]
    fn anonymous_is_reserved_any_case() {
        assert!(is_reserved_agent_name("anonymous"));
        assert!(is_reserved_agent_name("ANONYMOUS"));
        assert!(is_reserved_agent_name("Anonymous"));
        assert!(is_reserved_agent_name("  anonymous  "));
    }

    #[test]
    fn system_prefixes_are_reserved() {
        assert!(is_reserved_agent_name("system:reaper"));
        assert!(is_reserved_agent_name("system/heartbeat"));
    }

    #[test]
    fn ordinary_names_are_not_reserved() {
        assert!(!is_reserved_agent_name("my-agent-operator"));
        assert!(!is_reserved_agent_name("my-agent-engineer"));
        assert!(!is_reserved_agent_name("anonymouslab"));
        assert!(!is_reserved_agent_name("system-agent"));
    }
}
