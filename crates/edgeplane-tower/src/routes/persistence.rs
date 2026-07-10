use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/persistence/connections",
            get(list_connections).post(create_connection),
        )
        .route(
            "/persistence/bindings",
            get(list_bindings).post(create_binding),
        )
        .route(
            "/persistence/domains/{domain_id}/policy",
            get(get_policy).put(put_policy),
        )
        .route(
            "/persistence/publish/plan",
            axum::routing::post(publish_plan),
        )
        .route(
            "/persistence/publish/execute",
            axum::routing::post(publish_execute),
        )
        .route(
            "/persistence/publish/records",
            get(list_publication_records),
        )
        .route(
            "/persistence/publish/records/{record_id}",
            get(get_publication_record),
        )
}

fn row_to_connection(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<i32, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "name": row.get::<String, _>("name"),
        "provider": row.get::<String, _>("provider"),
        "host": row.get::<String, _>("host"),
        "repo_path": row.get::<String, _>("repo_path"),
        "default_branch": row.get::<String, _>("default_branch"),
        "credential_ref": row.get::<String, _>("credential_ref"),
        "options_json": row.get::<String, _>("options_json"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_binding(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<i32, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "name": row.get::<String, _>("name"),
        "connection_id": row.get::<i32, _>("connection_id"),
        "branch_override": row.get::<String, _>("branch_override"),
        "base_path": row.get::<String, _>("base_path"),
        "active": row.get::<bool, _>("active"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_policy(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<i32, _>("id"),
        "domain_id": row.get::<String, _>("domain_id"),
        "default_binding_id": row.get::<Option<i32>, _>("default_binding_id"),
        "fallback_mode": row.get::<String, _>("fallback_mode"),
        "require_approval": row.get::<bool, _>("require_approval"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_route(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<i32, _>("id"),
        "domain_id": row.get::<String, _>("domain_id"),
        "entity_kind": row.get::<String, _>("entity_kind"),
        "event_kind": row.get::<String, _>("event_kind"),
        "binding_id": row.get::<i32, _>("binding_id"),
        "branch_override": row.get::<String, _>("branch_override"),
        "path_template": row.get::<String, _>("path_template"),
        "format": row.get::<String, _>("format"),
        "active": row.get::<bool, _>("active"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_publication_record(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    json!({
        "id": row.get::<i32, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "domain_id": row.get::<Option<String>, _>("domain_id"),
        "ledger_event_id": row.get::<Option<i32>, _>("ledger_event_id"),
        "entity_kind": row.get::<String, _>("entity_kind"),
        "entity_id": row.get::<String, _>("entity_id"),
        "event_kind": row.get::<String, _>("event_kind"),
        "binding_id": row.get::<i32, _>("binding_id"),
        "repo_url": row.get::<String, _>("repo_url"),
        "branch": row.get::<String, _>("branch"),
        "file_path": row.get::<String, _>("file_path"),
        "commit_sha": row.get::<String, _>("commit_sha"),
        "status": row.get::<String, _>("status"),
        "error": row.get::<String, _>("error"),
        "detail_json": row.get::<String, _>("detail_json"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

/// Check if the principal is an owner of a domain (for policy endpoints).
async fn is_domain_owner(
    db: &sqlx::PgPool,
    principal: &Principal,
    domain_id: &str,
) -> bool {
    if principal.is_admin {
        return true;
    }
    if let Ok(Some(row)) =
        sqlx::query("SELECT owners FROM domain WHERE id=$1")
            .bind(domain_id)
            .fetch_optional(db)
            .await
    {
        let owners: String = row.get("owners");
        let sub = principal.subject.to_lowercase();
        return owners
            .split(',')
            .map(|x| x.trim().to_lowercase())
            .any(|x| x == sub);
    }
    false
}

async fn list_connections(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    match sqlx::query(
        "SELECT * FROM repoconnection WHERE owner_subject=$1 ORDER BY updated_at DESC",
    )
    .bind(&principal.subject)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            let connections: Vec<serde_json::Value> = rows.iter().map(row_to_connection).collect();
            Json(connections).into_response()
        }
        Err(e) => {
            tracing::error!("list_connections: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create_connection(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let repo_path = payload
        .get("repo_path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if name.is_empty() || repo_path.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "name and repo_path are required"})),
        )
            .into_response();
    }

    let provider = payload
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("github_app")
        .trim()
        .to_string();
    let host = payload
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("github.com")
        .trim()
        .to_string();
    let default_branch = payload
        .get("default_branch")
        .and_then(|v| v.as_str())
        .unwrap_or("main")
        .trim()
        .to_string();
    let credential_ref = payload
        .get("credential_ref")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let options_json = payload
        .get("options_json")
        .and_then(|v| v.as_str())
        .unwrap_or("{}")
        .to_string();

    let now = Utc::now().naive_utc();
    match sqlx::query(
        r#"INSERT INTO repoconnection
            (owner_subject, name, provider, host, repo_path, default_branch,
             credential_ref, options_json, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9) RETURNING *"#,
    )
    .bind(&principal.subject)
    .bind(&name)
    .bind(&provider)
    .bind(&host)
    .bind(&repo_path)
    .bind(&default_branch)
    .bind(&credential_ref)
    .bind(&options_json)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::OK, Json(row_to_connection(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_connection: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_bindings(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    match sqlx::query(
        "SELECT * FROM repobinding WHERE owner_subject=$1 ORDER BY updated_at DESC",
    )
    .bind(&principal.subject)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => {
            let bindings: Vec<serde_json::Value> = rows.iter().map(row_to_binding).collect();
            Json(bindings).into_response()
        }
        Err(e) => {
            tracing::error!("list_bindings: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create_binding(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let connection_id = payload
        .get("connection_id")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;

    if name.is_empty() || connection_id <= 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "name and connection_id are required"})),
        )
            .into_response();
    }

    // Verify connection exists and belongs to principal
    let conn_row = match sqlx::query(
        "SELECT id, owner_subject FROM repoconnection WHERE id=$1",
    )
    .bind(connection_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "connection not found"})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("create_binding fetch connection: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let conn_owner: String = conn_row.get("owner_subject");
    if conn_owner != principal.subject && !principal.is_admin {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "connection not found"})),
        )
            .into_response();
    }

    let branch_override = payload
        .get("branch_override")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let base_path = payload
        .get("base_path")
        .and_then(|v| v.as_str())
        .unwrap_or("domains")
        .trim()
        .trim_matches('/')
        .to_string();
    let active = payload
        .get("active")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let now = Utc::now().naive_utc();
    match sqlx::query(
        r#"INSERT INTO repobinding
            (owner_subject, name, connection_id, branch_override, base_path, active, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$7) RETURNING *"#,
    )
    .bind(&principal.subject)
    .bind(&name)
    .bind(connection_id)
    .bind(&branch_override)
    .bind(&base_path)
    .bind(active)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::OK, Json(row_to_binding(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_binding: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_policy(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
) -> impl IntoResponse {
    if !is_domain_owner(&state.db, &principal, &domain_id).await {
        return StatusCode::FORBIDDEN.into_response();
    }

    let policy_row = match sqlx::query(
        "SELECT * FROM domainpersistencepolicy WHERE domain_id=$1",
    )
    .bind(&domain_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("get_policy fetch policy: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let route_rows = match sqlx::query(
        "SELECT * FROM domainpersistenceroute WHERE domain_id=$1 AND active=true ORDER BY id ASC",
    )
    .bind(&domain_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("get_policy fetch routes: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let policy_val = policy_row.as_ref().map(row_to_policy);
    let routes_val: Vec<serde_json::Value> = route_rows.iter().map(row_to_route).collect();
    Json(json!({"policy": policy_val, "routes": routes_val})).into_response()
}

async fn put_policy(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !is_domain_owner(&state.db, &principal, &domain_id).await {
        return StatusCode::FORBIDDEN.into_response();
    }

    let now = Utc::now().naive_utc();
    let default_binding_id: Option<i32> = payload
        .get("default_binding_id")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let fallback_mode = payload
        .get("fallback_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("fail_closed")
        .to_string();
    let require_approval = payload
        .get("require_approval")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // UPSERT policy
    let upsert_result = sqlx::query(
        r#"INSERT INTO domainpersistencepolicy
            (domain_id, default_binding_id, fallback_mode, require_approval, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$5)
           ON CONFLICT (domain_id) DO UPDATE SET
             default_binding_id=EXCLUDED.default_binding_id,
             fallback_mode=EXCLUDED.fallback_mode,
             require_approval=EXCLUDED.require_approval,
             updated_at=EXCLUDED.updated_at"#,
    )
    .bind(&domain_id)
    .bind(default_binding_id)
    .bind(&fallback_mode)
    .bind(require_approval)
    .bind(now)
    .execute(&state.db)
    .await;

    if let Err(e) = upsert_result {
        tracing::error!("put_policy upsert: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Delete existing routes
    if let Err(e) = sqlx::query("DELETE FROM domainpersistenceroute WHERE domain_id=$1")
        .bind(&domain_id)
        .execute(&state.db)
        .await
    {
        tracing::error!("put_policy delete routes: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Insert new routes
    let routes = payload
        .get("routes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for item in &routes {
        let entity_kind = item
            .get("entity_kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if entity_kind.is_empty() {
            continue;
        }
        let event_kind = item
            .get("event_kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let binding_id = item
            .get("binding_id")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let branch_override = item
            .get("branch_override")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let path_template = item
            .get("path_template")
            .and_then(|v| v.as_str())
            .unwrap_or("domains/{domain_id}/{entity_kind}/{entity_id}.json")
            .to_string();
        let format = item
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("json_v1")
            .to_string();
        let active = item
            .get("active")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if let Err(e) = sqlx::query(
            r#"INSERT INTO domainpersistenceroute
                (domain_id, entity_kind, event_kind, binding_id, branch_override,
                 path_template, format, active, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$9)"#,
        )
        .bind(&domain_id)
        .bind(&entity_kind)
        .bind(&event_kind)
        .bind(binding_id)
        .bind(&branch_override)
        .bind(&path_template)
        .bind(&format)
        .bind(active)
        .bind(now)
        .execute(&state.db)
        .await
        {
            tracing::error!("put_policy insert route: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    Json(json!({"ok": true, "domain_id": domain_id, "updated_by": principal.subject}))
        .into_response()
}

// ── Publish plan / execute ───────────────────────────────────────────────────

struct PublishPlan {
    binding_id: i32,
    binding_name: String,
    provider: String,
    host: String,
    repo_path: String,
    branch: String,
    path_template: String, // raw template with {domain_id}/{entity_kind}/{entity_id}/{event_kind}
    rel_path: String,      // rendered for the request's specific entity_id
    format: String,
    credential_ref: String,
}

async fn resolve_publish_plan_inner(
    db: &sqlx::PgPool,
    owner_subject: &str,
    domain_id: &str,
    entity_kind: &str,
    event_kind: &str,
    entity_id: &str,
) -> Result<PublishPlan, String> {
    // Find best-matching route (specific event_kind first, then empty wildcard)
    let route_row = sqlx::query(
        "SELECT * FROM domainpersistenceroute \
         WHERE domain_id=$1 AND entity_kind=$2 AND active=true \
         AND (event_kind=$3 OR event_kind='') \
         ORDER BY event_kind DESC LIMIT 1",
    )
    .bind(domain_id)
    .bind(entity_kind)
    .bind(event_kind)
    .fetch_optional(db)
    .await
    .map_err(|e| format!("db_error: {e}"))?;

    let (binding_id, path_tpl, format_str, route_branch) = if let Some(ref r) = route_row {
        (
            r.get::<i32, _>("binding_id"),
            r.get::<String, _>("path_template"),
            r.get::<String, _>("format"),
            r.try_get::<String, _>("branch_override").ok().filter(|s| !s.is_empty()),
        )
    } else {
        // Fallback to policy default binding
        let policy = sqlx::query(
            "SELECT default_binding_id FROM domainpersistencepolicy WHERE domain_id=$1",
        )
        .bind(domain_id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("db_error: {e}"))?;
        let bid = policy
            .as_ref()
            .and_then(|p| p.try_get::<Option<i32>, _>("default_binding_id").ok().flatten())
            .ok_or_else(|| {
                format!(
                    "no persistence route/default binding for domain '{}' entity '{}' event '{}'",
                    domain_id, entity_kind, event_kind
                )
            })?;
        (bid, String::new(), "json_v1".to_string(), None)
    };

    // Load binding
    let binding = sqlx::query("SELECT * FROM repobinding WHERE id=$1")
        .bind(binding_id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("db_error: {e}"))?
        .ok_or_else(|| format!("binding '{}' not found", binding_id))?;

    let b_owner: String = binding.get("owner_subject");
    let b_active: bool = binding.get("active");
    if !b_active || b_owner != owner_subject {
        return Err(format!("binding '{}' is not available for current principal", binding_id));
    }
    let b_name: String = binding.get("name");
    let connection_id: i32 = binding.get("connection_id");
    let b_branch: String = binding.try_get("branch_override").unwrap_or_default();
    let base_path: String = binding.get("base_path");

    // Load connection
    let connection = sqlx::query("SELECT * FROM repoconnection WHERE id=$1")
        .bind(connection_id)
        .fetch_optional(db)
        .await
        .map_err(|e| format!("db_error: {e}"))?
        .ok_or_else(|| format!("connection '{}' not found", connection_id))?;

    let c_owner: String = connection.get("owner_subject");
    if c_owner != owner_subject {
        return Err(format!("connection '{}' is not available for current principal", connection_id));
    }
    let host: String = connection.get("host");
    let repo_path: String = connection.get("repo_path");
    let default_branch: String = connection.get("default_branch");
    let provider: String = connection.get("provider");
    let credential_ref: String = connection.get("credential_ref");

    // Resolve branch
    let branch = route_branch
        .or_else(|| if b_branch.is_empty() { None } else { Some(b_branch.clone()) })
        .unwrap_or_else(|| default_branch.clone());

    // Resolve path template — default: "{base_path}/{domain_id}/{entity_kind}/{entity_id}.json"
    let path_template = if path_tpl.is_empty() {
        format!("{base_path}/{{domain_id}}/{entity_kind}/{{entity_id}}.json")
    } else {
        path_tpl.clone()
    };
    let rel_path = path_template
        .replace("{domain_id}", domain_id)
        .replace("{entity_kind}", entity_kind)
        .replace("{entity_id}", entity_id)
        .replace("{event_kind}", event_kind)
        .trim_start_matches('/')
        .to_string();

    Ok(PublishPlan {
        binding_id,
        binding_name: b_name,
        provider,
        host,
        repo_path,
        branch,
        path_template,
        rel_path,
        format: format_str,
        credential_ref,
    })
}

fn resolve_git_url(plan: &PublishPlan) -> String {
    let token = if plan.credential_ref.starts_with("env:") {
        std::env::var(&plan.credential_ref[4..]).unwrap_or_default()
    } else if !plan.credential_ref.is_empty() {
        std::env::var(&plan.credential_ref).unwrap_or_default()
    } else {
        std::env::var("GIT_PUBLISH_TOKEN").unwrap_or_default()
    };
    if token.is_empty() {
        format!("https://{}/{}", plan.host, plan.repo_path)
    } else {
        format!("https://x-access-token:{}@{}/{}", token, plan.host, plan.repo_path)
    }
}

async fn publish_plan(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let domain_id = payload.get("domain_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let entity_kind = payload.get("entity_kind").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let event_kind = payload.get("event_kind").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let entity_id = payload.get("entity_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();

    if domain_id.is_empty() || entity_kind.is_empty() || event_kind.is_empty() || entity_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"detail": "domain_id, entity_kind, event_kind, entity_id are required"})),
        )
            .into_response();
    }

    if !is_domain_owner(&state.db, &principal, &domain_id).await {
        return StatusCode::FORBIDDEN.into_response();
    }

    match resolve_publish_plan_inner(
        &state.db,
        &principal.subject,
        &domain_id,
        &entity_kind,
        &event_kind,
        &entity_id,
    )
    .await
    {
        Ok(plan) => Json(json!({
            "domain_id": domain_id,
            "entity_kind": entity_kind,
            "event_kind": event_kind,
            "entity_id": entity_id,
            "binding_id": plan.binding_id,
            "binding_name": plan.binding_name,
            "provider": plan.provider,
            "repo": format!("{}/{}", plan.host, plan.repo_path),
            "branch": plan.branch,
            "path": plan.rel_path,
            "format": plan.format,
        }))
        .into_response(),
        Err(msg) => (StatusCode::CONFLICT, Json(json!({"detail": msg}))).into_response(),
    }
}

async fn publish_execute(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let domain_id = payload.get("domain_id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let limit = payload.get("limit").and_then(|v| v.as_i64()).unwrap_or(500).clamp(1, 500);

    if domain_id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({"detail": "domain_id is required"}))).into_response();
    }
    if !is_domain_owner(&state.db, &principal, &domain_id).await {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Load pending events
    let events = match sqlx::query(
        "SELECT * FROM ledgerevent WHERE domain_id=$1 AND state='pending' ORDER BY created_at ASC LIMIT $2",
    )
    .bind(&domain_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("publish_execute fetch events: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if events.is_empty() {
        return Json(json!({"published_count": 0, "commit_sha": "", "branch": "", "repo_url": ""}))
            .into_response();
    }

    // Resolve plan for first event; verify same binding for all
    let first = &events[0];
    let first_entity_kind: String = first.get("entity_type");
    let first_event_kind: String = first.get("action");
    let first_entity_id: String = first.get("entity_id");

    let plan = match resolve_publish_plan_inner(
        &state.db,
        &principal.subject,
        &domain_id,
        &first_entity_kind,
        &first_event_kind,
        &first_entity_id,
    )
    .await
    {
        Ok(p) => p,
        Err(msg) => return (StatusCode::CONFLICT, Json(json!({"detail": msg}))).into_response(),
    };

    // Clone repo and write files
    let repo_url = resolve_git_url(&plan);
    let clean_repo_url = format!("https://{}/{}", plan.host, plan.repo_path);
    let tmpdir = match tempfile::TempDir::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("publish_execute tempdir: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let repo_dir = tmpdir.path().to_string_lossy().to_string();

    let clone_ok = std::process::Command::new("git")
        .args(["clone", "--depth=1", "--branch", &plan.branch, &repo_url, &repo_dir])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !clone_ok {
        return (StatusCode::CONFLICT, Json(json!({"detail": "git clone failed"}))).into_response();
    }
    let _ = std::process::Command::new("git").args(["-C", &repo_dir, "config", "user.email", "edgeplane-tower@localhost"]).output();
    let _ = std::process::Command::new("git").args(["-C", &repo_dir, "config", "user.name", "edgeplane-tower"]).output();

    let now = Utc::now().naive_utc();
    for event in &events {
        let entity_kind: String = event.get("entity_type");
        let event_kind: String = event.get("action");
        let entity_id: String = event.get("entity_id");
        let payload_json: String = event.try_get("payload_json").unwrap_or_default();

        let rel = plan.path_template
            .replace("{domain_id}", &domain_id)
            .replace("{entity_kind}", &entity_kind)
            .replace("{entity_id}", &entity_id)
            .replace("{event_kind}", &event_kind)
            .trim_start_matches('/')
            .to_string();

        let mut doc: serde_json::Value = serde_json::from_str(&payload_json).unwrap_or(serde_json::json!({}));
        doc["published_at"] = serde_json::json!(now.to_string());
        doc["published_by"] = serde_json::json!(principal.subject);
        doc["publication"] = serde_json::json!({
            "binding": plan.binding_name,
            "route_entity_kind": entity_kind,
            "route_event_kind": event_kind,
        });

        let full_path = std::path::Path::new(&repo_dir).join(&rel);
        if let Some(parent) = full_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = serde_json::to_string_pretty(&doc).unwrap_or_default();
        let _ = std::fs::write(&full_path, content.as_bytes());
        let _ = std::process::Command::new("git").args(["-C", &repo_dir, "add", &rel]).output();
    }

    let commit_msg = format!("edgeplane-tower: publish {} ledger events for {}", events.len(), domain_id);
    let _ = std::process::Command::new("git")
        .args(["-C", &repo_dir, "commit", "--allow-empty", "-m", &commit_msg])
        .output();
    let commit_sha = std::process::Command::new("git")
        .args(["-C", &repo_dir, "rev-parse", "HEAD"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let push_ok = std::process::Command::new("git")
        .args(["-C", &repo_dir, "push", "origin", &plan.branch])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !push_ok {
        // The commit exists locally in the discarded tmpdir but never reached
        // the remote — Git is the memory of record, so the ledger must not be
        // marked published unless the push actually landed. Fail the request
        // and leave the events in `pending` state so a retry (or operator
        // investigation) can pick them up; do not touch the DB below.
        tracing::error!(
            "publish_execute: git push failed for domain {} (local commit {} not published)",
            domain_id,
            commit_sha
        );
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "detail": "git push failed; ledger events were not marked published",
                "commit_sha": commit_sha,
            })),
        )
            .into_response();
    }

    // Update DB
    let published_count = events.len() as i64;
    for event in &events {
        let eid: i32 = event.get("id");
        let entity_kind: String = event.get("entity_type");
        let entity_id: String = event.get("entity_id");
        let event_kind: String = event.get("action");
        let rel = plan.path_template
            .replace("{domain_id}", &domain_id)
            .replace("{entity_kind}", &entity_kind)
            .replace("{entity_id}", &entity_id)
            .replace("{event_kind}", &event_kind)
            .trim_start_matches('/')
            .to_string();
        let _ = sqlx::query(
            "UPDATE ledgerevent SET state='published', git_commit=$2, git_path=$3, published_at=$4, updated_at=$4 WHERE id=$1",
        )
        .bind(eid)
        .bind(&commit_sha)
        .bind(&rel)
        .bind(now)
        .execute(&state.db)
        .await;

        let _ = sqlx::query(
            r#"INSERT INTO publicationrecord
                (owner_subject, domain_id, ledger_event_id, entity_kind, entity_id, event_kind,
                 binding_id, repo_url, branch, file_path, commit_sha, status, error, detail_json,
                 created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'succeeded','',$12,$13,$13)"#,
        )
        .bind(&principal.subject)
        .bind(&domain_id)
        .bind(eid)
        .bind(&entity_kind)
        .bind(&entity_id)
        .bind(&event_kind)
        .bind(plan.binding_id)
        .bind(&clean_repo_url)
        .bind(&plan.branch)
        .bind(&rel)
        .bind(&commit_sha)
        .bind(format!(r#"{{"format":"{}"}}"#, plan.format))
        .bind(now)
        .execute(&state.db)
        .await;
    }

    Json(json!({
        "published_count": published_count,
        "commit_sha": commit_sha,
        "branch": plan.branch,
        "repo_url": clean_repo_url,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct ListRecordsQuery {
    domain_id: Option<String>,
    limit: Option<i64>,
}

async fn list_publication_records(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListRecordsQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(20).min(500);

    let rows = if let Some(ref mid) = q.domain_id {
        sqlx::query(
            "SELECT * FROM publicationrecord WHERE owner_subject=$1 AND domain_id=$2 \
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(&principal.subject)
        .bind(mid)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    } else {
        sqlx::query(
            "SELECT * FROM publicationrecord WHERE owner_subject=$1 \
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(&principal.subject)
        .bind(limit)
        .fetch_all(&state.db)
        .await
    };

    match rows {
        Ok(rows) => {
            let records: Vec<serde_json::Value> =
                rows.iter().map(row_to_publication_record).collect();
            Json(json!({"records": records})).into_response()
        }
        Err(e) => {
            tracing::error!("list_publication_records: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_publication_record(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(record_id): Path<i32>,
) -> impl IntoResponse {
    let row = match sqlx::query("SELECT * FROM publicationrecord WHERE id=$1")
        .bind(record_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"detail": "publication record not found"})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("get_publication_record: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let owner: String = row.get("owner_subject");
    if owner != principal.subject && !principal.is_admin {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "publication record not found"})),
        )
            .into_response();
    }

    // If record has a domain_id, verify ownership
    let domain_id: Option<String> = row.try_get("domain_id").ok().and_then(|v: Option<String>| v);
    if let Some(ref mid) = domain_id
        && !is_domain_owner(&state.db, &principal, mid).await {
            return StatusCode::FORBIDDEN.into_response();
        }

    Json(json!({"record": row_to_publication_record(&row)})).into_response()
}
