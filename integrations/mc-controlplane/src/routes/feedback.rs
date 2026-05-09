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
use std::collections::HashMap;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/feedback/agent", post(create_agent_feedback))
        .route("/feedback/human", post(create_human_feedback))
        .route("/feedback", get(list_feedback))
        .route("/feedback/{id}/triage", patch(update_triage))
        .route("/feedback/summary", get(feedback_summary))
}

#[derive(Deserialize)]
struct FeedbackCreate {
    mission_id: String,
    #[serde(default)]
    kluster_id: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    channel_id: String,
    #[serde(default)]
    category: String,
    #[serde(default = "default_medium")]
    severity: String,
    summary: String,
    #[serde(default)]
    recommendation: String,
    metadata: Option<serde_json::Value>,
}

fn default_medium() -> String {
    "medium".to_string()
}

#[derive(Deserialize)]
struct TriageUpdate {
    triage_status: Option<String>,
    priority: Option<String>,
    owner: Option<String>,
    disposition: Option<String>,
    outcome_ref: Option<String>,
}

#[derive(Deserialize)]
struct ListFeedbackQuery {
    mission_id: String,
    triage_status: Option<String>,
    source_type: Option<String>,
    priority: Option<String>,
    #[serde(default = "default_50")]
    limit: i64,
}

fn default_50() -> i64 {
    50
}

#[derive(Deserialize)]
struct SummaryQuery {
    mission_id: String,
}

fn row_to_feedback(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let metadata: serde_json::Value =
        serde_json::from_str(row.get::<&str, _>("metadata_json"))
            .unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "id": row.get::<i64, _>("id"),
        "mission_id": row.get::<String, _>("mission_id"),
        "kluster_id": row.get::<String, _>("kluster_id"),
        "source_type": row.get::<String, _>("source_type"),
        "source_subject": row.get::<String, _>("source_subject"),
        "provider": row.get::<String, _>("provider"),
        "channel_id": row.get::<String, _>("channel_id"),
        "category": row.get::<String, _>("category"),
        "severity": row.get::<String, _>("severity"),
        "summary": row.get::<String, _>("summary"),
        "recommendation": row.get::<String, _>("recommendation"),
        "status": row.get::<String, _>("status"),
        "triage_status": row.get::<String, _>("triage_status"),
        "priority": row.get::<String, _>("priority"),
        "owner": row.get::<String, _>("owner"),
        "disposition": row.get::<String, _>("disposition"),
        "outcome_ref": row.get::<String, _>("outcome_ref"),
        "metadata": metadata,
        "created_at": format!("{}Z", row.get::<chrono::NaiveDateTime, _>("created_at").format("%Y-%m-%dT%H:%M:%S")),
        "updated_at": format!("{}Z", row.get::<chrono::NaiveDateTime, _>("updated_at").format("%Y-%m-%dT%H:%M:%S")),
    })
}

fn not_found() -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"detail": "Not found"})),
    )
        .into_response()
}

async fn create_agent_feedback(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<FeedbackCreate>,
) -> impl IntoResponse {
    create_feedback("agent", state, principal, body).await
}

async fn create_human_feedback(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<FeedbackCreate>,
) -> impl IntoResponse {
    create_feedback("human", state, principal, body).await
}

async fn create_feedback(
    source_type: &str,
    state: Arc<AppState>,
    principal: Principal,
    body: FeedbackCreate,
) -> axum::response::Response {
    let now = Utc::now().naive_utc();
    let metadata_json = serde_json::to_string(
        &body.metadata.unwrap_or(serde_json::json!({})),
    )
    .unwrap_or_else(|_| "{}".into());

    match sqlx::query(
        "INSERT INTO feedbackentry \
         (mission_id, kluster_id, source_type, source_subject, provider, channel_id, \
          category, severity, summary, recommendation, status, triage_status, priority, \
          owner, disposition, outcome_ref, metadata_json, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'open','new','p2','','','', $11,$12,$12) \
         RETURNING *",
    )
    .bind(&body.mission_id)
    .bind(&body.kluster_id)
    .bind(source_type)
    .bind(&principal.subject)
    .bind(&body.provider)
    .bind(&body.channel_id)
    .bind(&body.category)
    .bind(&body.severity)
    .bind(&body.summary)
    .bind(&body.recommendation)
    .bind(&metadata_json)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::CREATED, Json(row_to_feedback(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_feedback: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_feedback(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Query(q): Query<ListFeedbackQuery>,
) -> impl IntoResponse {
    let limit = q.limit.min(200);

    match sqlx::query(
        "SELECT * FROM feedbackentry \
         WHERE mission_id=$1 \
           AND ($2::text IS NULL OR triage_status=$2) \
           AND ($3::text IS NULL OR source_type=$3) \
           AND ($4::text IS NULL OR priority=$4) \
         ORDER BY updated_at DESC LIMIT $5",
    )
    .bind(&q.mission_id)
    .bind(&q.triage_status)
    .bind(&q.source_type)
    .bind(&q.priority)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_feedback).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_feedback: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_triage(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(id): Path<i64>,
    Json(body): Json<TriageUpdate>,
) -> impl IntoResponse {
    // Load existing row
    let existing = sqlx::query("SELECT * FROM feedbackentry WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    let row = match existing {
        Ok(Some(r)) => r,
        Ok(None) => return not_found(),
        Err(e) => {
            tracing::error!("update_triage fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let triage_status = body
        .triage_status
        .unwrap_or_else(|| row.get("triage_status"));
    let priority = body.priority.unwrap_or_else(|| row.get("priority"));
    let owner = body.owner.unwrap_or_else(|| row.get("owner"));
    let disposition = body.disposition.unwrap_or_else(|| row.get("disposition"));
    let outcome_ref = body.outcome_ref.unwrap_or_else(|| row.get("outcome_ref"));
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "UPDATE feedbackentry \
         SET triage_status=$2, priority=$3, owner=$4, disposition=$5, outcome_ref=$6, \
             updated_at=$7 \
         WHERE id=$1 RETURNING *",
    )
    .bind(id)
    .bind(&triage_status)
    .bind(&priority)
    .bind(&owner)
    .bind(&disposition)
    .bind(&outcome_ref)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => Json(row_to_feedback(&row)).into_response(),
        Err(e) => {
            tracing::error!("update_triage update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn feedback_summary(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Query(q): Query<SummaryQuery>,
) -> impl IntoResponse {
    let rows = match sqlx::query(
        "SELECT * FROM feedbackentry WHERE mission_id=$1",
    )
    .bind(&q.mission_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("feedback_summary: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let total = rows.len() as i64;
    let mut by_source_type: HashMap<String, i64> = HashMap::new();
    let mut by_severity: HashMap<String, i64> = HashMap::new();
    let mut by_category: HashMap<String, i64> = HashMap::new();
    let mut by_triage_status: HashMap<String, i64> = HashMap::new();
    let mut by_priority: HashMap<String, i64> = HashMap::new();

    for row in &rows {
        let source_type: String = row.get("source_type");
        let severity: String = row.get("severity");
        let category: String = row.get("category");
        let triage_status: String = row.get("triage_status");
        let priority: String = row.get("priority");

        *by_source_type.entry(source_type).or_insert(0) += 1;
        *by_severity.entry(severity).or_insert(0) += 1;
        *by_category.entry(category).or_insert(0) += 1;
        *by_triage_status.entry(triage_status).or_insert(0) += 1;
        *by_priority.entry(priority).or_insert(0) += 1;
    }

    Json(serde_json::json!({
        "mission_id": q.mission_id,
        "total": total,
        "by_source_type": by_source_type,
        "by_severity": by_severity,
        "by_category": by_category,
        "by_triage_status": by_triage_status,
        "by_priority": by_priority,
    }))
    .into_response()
}
