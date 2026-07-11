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
        .route("/ingest/github", post(ingest_github))
        .route("/ingest/drive", post(ingest_drive))
        .route("/ingest/slack", post(ingest_slack))
        .route("/ingest/jobs", get(list_jobs))
        .route("/ingest/jobs/{job_id}", get(get_job))
}

#[derive(Deserialize)]
struct IngestRequest {
    mission_id: String,
    #[serde(default)]
    config: serde_json::Value,
}

#[derive(Deserialize)]
struct ListQuery {
    mission_id: Option<String>,
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn row_to_job(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "mission_id": row.get::<String, _>("mission_id"),
        "source": row.get::<String, _>("source"),
        "status": row.get::<String, _>("status"),
        "config": row.get::<String, _>("config"),
        "logs": row.get::<String, _>("logs"),
        "result_summary": row.get::<String, _>("result_summary"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

async fn create_job(
    db: &sqlx::PgPool,
    mission_id: &str,
    source: &str,
    config: &serde_json::Value,
) -> Result<serde_json::Value, sqlx::Error> {
    let config_str = serde_json::to_string(config).unwrap_or_else(|_| "{}".to_string());
    let now = Utc::now().naive_utc();
    let row = sqlx::query(
        "INSERT INTO ingestionjob (mission_id, source, status, config, logs, result_summary, created_at, updated_at) \
         VALUES ($1,$2,'queued',$3,'','',$4,$4) RETURNING *",
    )
    .bind(mission_id)
    .bind(source)
    .bind(&config_str)
    .bind(now)
    .fetch_one(db)
    .await?;
    Ok(row_to_job(&row))
}

async fn ingest_github(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Json(body): Json<IngestRequest>,
) -> impl IntoResponse {
    match create_job(&state.db, &body.mission_id, "github", &body.config).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            tracing::error!("ingest_github: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn ingest_drive(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Json(body): Json<IngestRequest>,
) -> impl IntoResponse {
    match create_job(&state.db, &body.mission_id, "google_drive", &body.config).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            tracing::error!("ingest_drive: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn ingest_slack(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Json(body): Json<IngestRequest>,
) -> impl IntoResponse {
    match create_job(&state.db, &body.mission_id, "slack", &body.config).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            tracing::error!("ingest_slack: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Default-deny gate: resolve the mission's owning domain, then require the
/// caller to be a member. Fails closed (404 on missing mission/NULL domain,
/// 500 on DB error, 403 on non-member). Local rather than the shared
/// `authz_by_mission` combinator, which lives on the Group A branch.
async fn authz_mission(
    db: &sqlx::PgPool,
    principal: &Principal,
    mission_id: &str,
) -> Result<(), axum::response::Response> {
    let domain_id = crate::routes::authz::domain_id_for_mission(db, mission_id).await?;
    crate::routes::authz::authz_domain(db, principal, &domain_id).await
}

async fn list_jobs(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let rows = if let Some(mission_id) = &q.mission_id {
        if let Err(resp) = authz_mission(&state.db, &principal, mission_id).await {
            return resp;
        }
        sqlx::query(
            "SELECT * FROM ingestionjob WHERE mission_id=$1 ORDER BY updated_at DESC",
        )
        .bind(mission_id)
        .fetch_all(&state.db)
        .await
    } else {
        // No mission filter would dump every tenant's jobs. Require a mission_id
        // for ordinary callers; only an admin may list across all missions.
        if !principal.is_admin {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"detail": "mission_id query parameter is required"})),
            )
                .into_response();
        }
        sqlx::query("SELECT * FROM ingestionjob ORDER BY updated_at DESC LIMIT 200")
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

async fn get_job(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(job_id): Path<i32>,
) -> impl IntoResponse {
    let row = match sqlx::query("SELECT * FROM ingestionjob WHERE id=$1")
        .bind(job_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return not_found("Job not found"),
        Err(e) => {
            tracing::error!("get_job: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // Gate on the job's mission-domain before returning it (config can name
    // source systems / connectors). NB: ingestionjob ids are a sequential serial,
    // so the 403-vs-404 split is a minor existence oracle — tracked as follow-up.
    let mission_id: String = row.get("mission_id");
    if let Err(resp) = authz_mission(&state.db, &principal, &mission_id).await {
        return resp;
    }
    Json(row_to_job(&row)).into_response()
}
