use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{
    auth::Principal,
    models::task::{Task, TaskCreate, TaskUpdate},
    state::AppState,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/domains/{domain_id}/m/{mission_id}/t", get(list_tasks).post(create_task))
        .route(
            "/domains/{domain_id}/m/{mission_id}/t/{task_id}",
            get(get_task).patch(update_task).delete(delete_task),
        )
        .route(
            "/domains/{domain_id}/m/{mission_id}/t/{task_id}/overlaps",
            get(list_overlaps),
        )
        // Shortcut: list tasks by mission_id without requiring domain_id (used by TUI)
        .route("/missions/{mission_id}/t", get(list_tasks_by_mission))
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect()
}

async fn domain_access(
    state: &AppState,
    domain_id: &str,
    mission_id: &str,
    principal: &Principal,
    require_write: bool,
    require_owner: bool,
) -> Result<(), axum::response::Response> {
    let m = sqlx::query("SELECT * FROM domain WHERE id=$1")
        .bind(domain_id).fetch_optional(&state.db).await
        .map_err(|e| { tracing::error!("domain_access: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() })?;
    let row = m.ok_or_else(|| not_found("Domain not found"))?;

    let vis: String = row.try_get("visibility").unwrap_or_default();
    let owners: String = row.try_get("owners").unwrap_or_default();
    let contribs: String = row.try_get("contributors").unwrap_or_default();
    let id = principal.subject.to_lowercase();

    if require_owner && !principal.is_admin && !split_csv(&owners).contains(&id) {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    if require_write && !principal.is_admin
        && !split_csv(&owners).contains(&id)
        && !split_csv(&contribs).contains(&id)
    {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    if !require_write && !principal.is_admin && vis.to_lowercase() != "public"
        && !split_csv(&owners).contains(&id)
        && !split_csv(&contribs).contains(&id)
    {
        return Err(StatusCode::FORBIDDEN.into_response());
    }

    let k: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1 AND domain_id=$2")
        .bind(mission_id).bind(domain_id)
        .fetch_optional(&state.db).await.unwrap_or(None);
    if k.is_none() { return Err(not_found("Mission not found")); }
    Ok(())
}

#[derive(Deserialize)]
struct ListQuery { status: Option<String>, limit: Option<i64> }

async fn list_tasks(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id)): Path<(String, String)>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    if let Err(r) = domain_access(&state, &domain_id, &mission_id, &principal, false, false).await { return r; }
    let limit = q.limit.unwrap_or(100).min(500);
    let rows = if let Some(s) = &q.status {
        sqlx::query_as::<_, Task>(
            "SELECT * FROM task WHERE mission_id=$1 AND status=$2 ORDER BY updated_at DESC LIMIT $3"
        )
        .bind(&mission_id).bind(s).bind(limit).fetch_all(&state.db).await
    } else {
        sqlx::query_as::<_, Task>(
            "SELECT * FROM task WHERE mission_id=$1 ORDER BY updated_at DESC LIMIT $2"
        )
        .bind(&mission_id).bind(limit).fetch_all(&state.db).await
    };
    match rows {
        Ok(tasks) => Json(tasks).into_response(),
        Err(e) => { tracing::error!("list_tasks: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn create_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id)): Path<(String, String)>,
    Json(payload): Json<TaskCreate>,
) -> impl IntoResponse {
    if let Err(r) = domain_access(&state, &domain_id, &mission_id, &principal, true, false).await { return r; }
    if payload.title.trim().is_empty() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": "title is required"}))).into_response();
    }

    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, Task>(
        "INSERT INTO task (public_id, mission_id, epic_id, title, description, status, owner, \
         contributors, dependencies, definition_of_done, related_artifacts, created_at, updated_at) \
         VALUES ('',$1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$11) RETURNING *"
    )
    .bind(&mission_id).bind(payload.epic_id).bind(payload.title.trim())
    .bind(&payload.description).bind(&payload.status).bind(&payload.owner)
    .bind(&payload.contributors).bind(&payload.dependencies)
    .bind(&payload.definition_of_done).bind(&payload.related_artifacts)
    .bind(now)
    .fetch_one(&state.db).await {
        Ok(t) => (StatusCode::OK, Json(t)).into_response(),
        Err(e) => { tracing::error!("create_task: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn get_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id, task_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if let Err(r) = domain_access(&state, &domain_id, &mission_id, &principal, false, false).await { return r; }

    // task_id may be numeric id or public_id string
    let row = if let Ok(numeric_id) = task_id.parse::<i64>() {
        sqlx::query_as::<_, Task>("SELECT * FROM task WHERE id=$1 AND mission_id=$2")
            .bind(numeric_id).bind(&mission_id).fetch_optional(&state.db).await
    } else {
        sqlx::query_as::<_, Task>("SELECT * FROM task WHERE public_id=$1 AND mission_id=$2")
            .bind(&task_id).bind(&mission_id).fetch_optional(&state.db).await
    };

    match row {
        Ok(Some(t)) => Json(t).into_response(),
        Ok(None) => not_found("Task not found"),
        Err(e) => { tracing::error!("get_task: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn update_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id, task_id)): Path<(String, String, String)>,
    Json(payload): Json<TaskUpdate>,
) -> impl IntoResponse {
    if let Err(r) = domain_access(&state, &domain_id, &mission_id, &principal, true, false).await { return r; }

    let existing = if let Ok(numeric_id) = task_id.parse::<i64>() {
        sqlx::query_as::<_, Task>("SELECT * FROM task WHERE id=$1 AND mission_id=$2")
            .bind(numeric_id).bind(&mission_id).fetch_optional(&state.db).await
    } else {
        sqlx::query_as::<_, Task>("SELECT * FROM task WHERE public_id=$1 AND mission_id=$2")
            .bind(&task_id).bind(&mission_id).fetch_optional(&state.db).await
    };

    let task = match existing {
        Ok(Some(t)) => t,
        Ok(None) => return not_found("Task not found"),
        Err(e) => { tracing::error!("update_task fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let title                = payload.title.unwrap_or(task.title);
    let description          = payload.description.unwrap_or(task.description);
    let status               = payload.status.unwrap_or(task.status);
    let owner                = payload.owner.unwrap_or(task.owner);
    let contributors         = payload.contributors.unwrap_or(task.contributors);
    let dependencies         = payload.dependencies.unwrap_or(task.dependencies);
    let definition_of_done   = payload.definition_of_done.unwrap_or(task.definition_of_done);
    let related_artifacts    = payload.related_artifacts.unwrap_or(task.related_artifacts);
    let epic_id              = payload.epic_id.or(task.epic_id);
    let now = Utc::now().naive_utc();

    match sqlx::query_as::<_, Task>(
        "UPDATE task SET title=$2, description=$3, status=$4, owner=$5, contributors=$6, \
         dependencies=$7, definition_of_done=$8, related_artifacts=$9, epic_id=$10, \
         updated_at=$11 WHERE id=$1 RETURNING *"
    )
    .bind(task.id).bind(&title).bind(&description).bind(&status).bind(&owner)
    .bind(&contributors).bind(&dependencies).bind(&definition_of_done)
    .bind(&related_artifacts).bind(epic_id).bind(now)
    .fetch_one(&state.db).await {
        Ok(t) => Json(t).into_response(),
        Err(e) => { tracing::error!("update_task: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

async fn delete_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id, task_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if let Err(r) = domain_access(&state, &domain_id, &mission_id, &principal, false, true).await { return r; }

    let existing = if let Ok(numeric_id) = task_id.parse::<i64>() {
        sqlx::query_as::<_, Task>("SELECT * FROM task WHERE id=$1 AND mission_id=$2")
            .bind(numeric_id).bind(&mission_id).fetch_optional(&state.db).await
    } else {
        sqlx::query_as::<_, Task>("SELECT * FROM task WHERE public_id=$1 AND mission_id=$2")
            .bind(&task_id).bind(&mission_id).fetch_optional(&state.db).await
    };

    let task = match existing {
        Ok(Some(t)) => t,
        Ok(None) => return not_found("Task not found"),
        Err(e) => { tracing::error!("delete_task fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let _ = sqlx::query("DELETE FROM overlapsuggestion WHERE task_id=$1 OR candidate_task_id=$1")
        .bind(task.id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM taskassignment WHERE task_id=$1").bind(task.id).execute(&state.db).await;
    let _ = sqlx::query("UPDATE agentmessage SET task_id=NULL WHERE task_id=$1").bind(task.id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM task WHERE id=$1").bind(task.id).execute(&state.db).await;

    let deleted_id = if task.public_id.is_empty() { task.id.to_string() } else { task.public_id.clone() };
    Json(serde_json::json!({"ok": true, "deleted_id": deleted_id})).into_response()
}

// ── Shortcut: GET /missions/{mission_id}/t ────────────────────────────────────
// Used by the TUI which only knows mission_id, not the parent domain_id.
// No auth check — mirrors the unauthenticated pattern; add Principal if auth is needed.

async fn list_tasks_by_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_by_mission(&state.db, &principal, &mission_id).await
    {
        return r;
    }
    match sqlx::query_as::<_, Task>(
        "SELECT * FROM task WHERE mission_id=$1 ORDER BY created_at ASC LIMIT 200"
    )
    .bind(&mission_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(tasks) => Json(tasks).into_response(),
        Err(e) => {
            tracing::error!("list_tasks_by_mission: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_overlaps(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id, task_id)): Path<(String, String, String)>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    if let Err(r) = domain_access(&state, &domain_id, &mission_id, &principal, false, false).await { return r; }

    let task_row = if let Ok(numeric_id) = task_id.parse::<i64>() {
        sqlx::query_as::<_, Task>("SELECT * FROM task WHERE id=$1 AND mission_id=$2")
            .bind(numeric_id).bind(&mission_id).fetch_optional(&state.db).await
    } else {
        sqlx::query_as::<_, Task>("SELECT * FROM task WHERE public_id=$1 AND mission_id=$2")
            .bind(&task_id).bind(&mission_id).fetch_optional(&state.db).await
    };
    let task = match task_row {
        Ok(Some(t)) => t,
        Ok(None) => return not_found("Task not found"),
        Err(e) => { tracing::error!("list_overlaps fetch task: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let limit = q.limit.unwrap_or(20).min(100);
    // overlapsuggestion's real columns are similarity_score/evidence/suggested_action
    // (all NOT NULL) — score/reason never existed; every call here 500'd with
    // "column does not exist" since this route was written. Mirrors the correct
    // column names/types already used by the MCP get_overlap_suggestions handler
    // (routes/mcp.rs) for the same table.
    match sqlx::query(
        "SELECT id, task_id, candidate_task_id, similarity_score, evidence, suggested_action, created_at \
         FROM overlapsuggestion WHERE task_id=$1 ORDER BY similarity_score DESC LIMIT $2"
    )
    .bind(task.id).bind(limit)
    .fetch_all(&state.db).await {
        Ok(rows) => Json(rows.iter().map(|r| serde_json::json!({
            "id": r.get::<i32,_>("id"),
            "task_id": r.get::<i32,_>("task_id"),
            "candidate_task_id": r.get::<i32,_>("candidate_task_id"),
            "similarity_score": r.get::<f64,_>("similarity_score"),
            "evidence": r.get::<String,_>("evidence"),
            "suggested_action": r.get::<String,_>("suggested_action"),
            "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
        })).collect::<Vec<_>>()).into_response(),
        Err(e) => { tracing::error!("list_overlaps: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}
