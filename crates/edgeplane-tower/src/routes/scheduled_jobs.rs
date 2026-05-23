use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/scheduled-jobs", get(list_jobs).post(create_job))
        .route("/scheduled-jobs/{id}", get(get_job)
            .put(update_job)
            .delete(delete_job))
        .route("/scheduled-jobs/{id}/run", post(trigger_job))
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct CreateJob {
    name: String,
    #[serde(default)]
    description: String,
    cron_expr: String,
    #[serde(default)]
    runtime_kind: String,
    initial_prompt: String,
    system_context: Option<String>,
    #[serde(default)]
    policy: serde_json::Value,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool { true }

#[derive(Deserialize)]
struct UpdateJob {
    name: Option<String>,
    description: Option<String>,
    cron_expr: Option<String>,
    runtime_kind: Option<String>,
    initial_prompt: Option<String>,
    system_context: Option<String>,
    policy: Option<serde_json::Value>,
    enabled: Option<bool>,
}

fn row_to_job(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let policy_json: String = row.get("policy_json");
    let policy: serde_json::Value = serde_json::from_str(&policy_json).unwrap_or(serde_json::json!({}));
    let last_run_at: Option<chrono::NaiveDateTime> = row.get("last_run_at");
    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "name": row.get::<String, _>("name"),
        "description": row.get::<String, _>("description"),
        "cron_expr": row.get::<String, _>("cron_expr"),
        "runtime_kind": row.get::<String, _>("runtime_kind"),
        "initial_prompt": row.get::<String, _>("initial_prompt"),
        "system_context": row.get::<Option<String>, _>("system_context"),
        "policy": policy,
        "enabled": row.get::<bool, _>("enabled"),
        "last_run_at": last_run_at.map(|t| format!("{}Z", t.format("%Y-%m-%dT%H:%M:%S"))),
        "last_session_id": row.get::<Option<String>, _>("last_session_id"),
        "created_at": format!("{}Z", row.get::<chrono::NaiveDateTime, _>("created_at").format("%Y-%m-%dT%H:%M:%S")),
        "updated_at": format!("{}Z", row.get::<chrono::NaiveDateTime, _>("updated_at").format("%Y-%m-%dT%H:%M:%S")),
    })
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

async fn list_jobs(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).min(500);
    match sqlx::query(
        "SELECT * FROM scheduledagentjob WHERE owner_subject=$1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&principal.subject)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_job).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_jobs: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create_job(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<CreateJob>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();
    let policy_json = serde_json::to_string(&body.policy).unwrap_or_else(|_| "{}".into());
    let runtime_kind = if body.runtime_kind.is_empty() {
        "claude_code".to_string()
    } else {
        body.runtime_kind
    };

    match sqlx::query(
        "INSERT INTO scheduledagentjob \
         (owner_subject, name, description, cron_expr, runtime_kind, initial_prompt, \
          system_context, policy_json, enabled, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$10) RETURNING *",
    )
    .bind(&principal.subject)
    .bind(&body.name)
    .bind(&body.description)
    .bind(&body.cron_expr)
    .bind(&runtime_kind)
    .bind(&body.initial_prompt)
    .bind(&body.system_context)
    .bind(&policy_json)
    .bind(body.enabled)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::OK, Json(row_to_job(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_job: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_job(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    match sqlx::query(
        "SELECT * FROM scheduledagentjob WHERE id=$1 AND owner_subject=$2",
    )
    .bind(id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(row)) => Json(row_to_job(&row)).into_response(),
        Ok(None) => not_found("scheduled job not found"),
        Err(e) => {
            tracing::error!("get_job: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_job(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<i32>,
    Json(body): Json<UpdateJob>,
) -> impl IntoResponse {
    // Load existing
    let existing = sqlx::query(
        "SELECT * FROM scheduledagentjob WHERE id=$1 AND owner_subject=$2",
    )
    .bind(id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await;

    let row = match existing {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("scheduled job not found"),
        Err(e) => {
            tracing::error!("update_job fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let name = body.name.unwrap_or_else(|| row.get("name"));
    let description = body.description.unwrap_or_else(|| row.get("description"));
    let cron_expr = body.cron_expr.unwrap_or_else(|| row.get("cron_expr"));
    let runtime_kind = body.runtime_kind.unwrap_or_else(|| row.get("runtime_kind"));
    let initial_prompt = body.initial_prompt.unwrap_or_else(|| row.get("initial_prompt"));
    let system_context: Option<String> = body.system_context.or_else(|| row.get("system_context"));
    let enabled = body.enabled.unwrap_or_else(|| row.get("enabled"));
    let policy_json = body.policy
        .as_ref()
        .and_then(|p| serde_json::to_string(p).ok())
        .unwrap_or_else(|| row.get("policy_json"));
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "UPDATE scheduledagentjob SET name=$3, description=$4, cron_expr=$5, runtime_kind=$6, \
         initial_prompt=$7, system_context=$8, policy_json=$9, enabled=$10, updated_at=$11 \
         WHERE id=$1 AND owner_subject=$2 RETURNING *",
    )
    .bind(id)
    .bind(&principal.subject)
    .bind(&name)
    .bind(&description)
    .bind(&cron_expr)
    .bind(&runtime_kind)
    .bind(&initial_prompt)
    .bind(&system_context)
    .bind(&policy_json)
    .bind(enabled)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => Json(row_to_job(&row)).into_response(),
        Err(e) => {
            tracing::error!("update_job: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_job(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    match sqlx::query(
        "DELETE FROM scheduledagentjob WHERE id=$1 AND owner_subject=$2",
    )
    .bind(id)
    .bind(&principal.subject)
    .execute(&state.db)
    .await
    {
        Ok(r) if r.rows_affected() == 0 => not_found("scheduled job not found"),
        Ok(_) => Json(serde_json::json!({"deleted": true, "id": id})).into_response(),
        Err(e) => {
            tracing::error!("delete_job: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn trigger_job(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    // Verify ownership only; actual execution stays Python-side for now
    let exists: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM scheduledagentjob WHERE id=$1 AND owner_subject=$2",
    )
    .bind(id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    match exists {
        Some(_) => Json(serde_json::json!({"triggered": true, "id": id})).into_response(),
        None => not_found("scheduled job not found"),
    }
}
