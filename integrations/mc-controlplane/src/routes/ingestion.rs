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

use crate::state::AppState;

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
    kluster_id: String,
    #[serde(default)]
    config: serde_json::Value,
}

#[derive(Deserialize)]
struct ListQuery {
    kluster_id: Option<String>,
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn row_to_job(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "kluster_id": row.get::<String, _>("kluster_id"),
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
    kluster_id: &str,
    source: &str,
    config: &serde_json::Value,
) -> Result<serde_json::Value, sqlx::Error> {
    let config_str = serde_json::to_string(config).unwrap_or_else(|_| "{}".to_string());
    let now = Utc::now().naive_utc();
    let row = sqlx::query(
        "INSERT INTO ingestionjob (kluster_id, source, status, config, logs, result_summary, created_at, updated_at) \
         VALUES ($1,$2,'queued',$3,'','',$4,$4) RETURNING *",
    )
    .bind(kluster_id)
    .bind(source)
    .bind(&config_str)
    .bind(now)
    .fetch_one(db)
    .await?;
    Ok(row_to_job(&row))
}

async fn ingest_github(
    State(state): State<Arc<AppState>>,
    Json(body): Json<IngestRequest>,
) -> impl IntoResponse {
    match create_job(&state.db, &body.kluster_id, "github", &body.config).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            tracing::error!("ingest_github: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn ingest_drive(
    State(state): State<Arc<AppState>>,
    Json(body): Json<IngestRequest>,
) -> impl IntoResponse {
    match create_job(&state.db, &body.kluster_id, "google_drive", &body.config).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            tracing::error!("ingest_drive: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn ingest_slack(
    State(state): State<Arc<AppState>>,
    Json(body): Json<IngestRequest>,
) -> impl IntoResponse {
    match create_job(&state.db, &body.kluster_id, "slack", &body.config).await {
        Ok(job) => Json(job).into_response(),
        Err(e) => {
            tracing::error!("ingest_slack: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_jobs(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let rows = if let Some(kluster_id) = &q.kluster_id {
        sqlx::query(
            "SELECT * FROM ingestionjob WHERE kluster_id=$1 ORDER BY updated_at DESC",
        )
        .bind(kluster_id)
        .fetch_all(&state.db)
        .await
    } else {
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
    Path(job_id): Path<i32>,
) -> impl IntoResponse {
    match sqlx::query("SELECT * FROM ingestionjob WHERE id=$1")
        .bind(job_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(row)) => Json(row_to_job(&row)).into_response(),
        Ok(None) => not_found("Job not found"),
        Err(e) => {
            tracing::error!("get_job: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
