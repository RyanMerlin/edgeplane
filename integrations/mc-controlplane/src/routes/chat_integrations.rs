use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/integrations/chat/bindings", post(create_binding).get(list_bindings))
        .route("/integrations/chat/bindings/{binding_id}", delete(delete_binding))
}

#[derive(Deserialize)]
struct CreateBindingBody {
    mission_id: String,
    channel_id: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    workspace_external_id: String,
    #[serde(default)]
    channel_name: String,
    channel_metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ListQuery {
    mission_id: String,
    #[serde(default)]
    provider: String,
    limit: Option<i64>,
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn row_to_binding(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let metadata_json: String = row.get("channel_metadata_json");
    let metadata: serde_json::Value =
        serde_json::from_str(&metadata_json).unwrap_or(serde_json::json!({}));
    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "provider": row.get::<String, _>("provider"),
        "mission_id": row.get::<String, _>("mission_id"),
        "workspace_external_id": row.get::<String, _>("workspace_external_id"),
        "channel_id": row.get::<String, _>("channel_id"),
        "channel_name": row.get::<String, _>("channel_name"),
        "channel_metadata": metadata,
        "created_by": row.get::<String, _>("created_by"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

async fn create_binding(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<CreateBindingBody>,
) -> impl IntoResponse {
    let provider = if body.provider.is_empty() { "slack".to_string() } else { body.provider };
    let metadata_json = body
        .channel_metadata
        .as_ref()
        .and_then(|m| serde_json::to_string(m).ok())
        .unwrap_or_else(|| "{}".to_string());
    let now = Utc::now().naive_utc();

    // Check for existing binding first
    let existing = sqlx::query(
        "SELECT * FROM slackchannelbinding WHERE provider=$1 AND mission_id=$2 AND channel_id=$3",
    )
    .bind(&provider)
    .bind(&body.mission_id)
    .bind(&body.channel_id)
    .fetch_optional(&state.db)
    .await;

    match existing {
        Ok(Some(row)) => return Json(row_to_binding(&row)).into_response(),
        Err(e) => {
            tracing::error!("create_binding check: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Ok(None) => {}
    }

    let result = sqlx::query(
        "INSERT INTO slackchannelbinding \
         (provider, mission_id, workspace_external_id, channel_id, channel_name, \
          channel_metadata_json, created_by, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8) RETURNING *",
    )
    .bind(&provider)
    .bind(&body.mission_id)
    .bind(&body.workspace_external_id)
    .bind(&body.channel_id)
    .bind(&body.channel_name)
    .bind(&metadata_json)
    .bind(&principal.subject)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => (StatusCode::CREATED, Json(row_to_binding(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_binding insert: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_bindings(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let provider = if q.provider.is_empty() { "slack".to_string() } else { q.provider };
    let limit = q.limit.unwrap_or(100).min(500);

    let rows = sqlx::query(
        "SELECT * FROM slackchannelbinding WHERE provider=$1 AND mission_id=$2 \
         ORDER BY updated_at DESC LIMIT $3",
    )
    .bind(&provider)
    .bind(&q.mission_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_binding).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_bindings: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_binding(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Path(binding_id): Path<i32>,
) -> impl IntoResponse {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM slackchannelbinding WHERE id=$1)",
    )
    .bind(binding_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);

    if !exists {
        return not_found("Chat binding not found");
    }

    match sqlx::query("DELETE FROM slackchannelbinding WHERE id=$1")
        .bind(binding_id)
        .execute(&state.db)
        .await
    {
        Ok(_) => Json(serde_json::json!({"ok": true, "deleted_id": binding_id})).into_response(),
        Err(e) => {
            tracing::error!("delete_binding: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
