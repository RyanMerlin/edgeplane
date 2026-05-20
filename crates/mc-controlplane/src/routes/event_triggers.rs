use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/event-triggers", post(create_trigger).get(list_triggers))
        .route(
            "/event-triggers/{id}",
            get(get_trigger).delete(deactivate_trigger),
        )
}

#[derive(Deserialize)]
struct TriggerCreate {
    event_type: String,
    predicate_json: Option<String>,
    #[serde(default = "default_mesh_task")]
    target_type: String,
    target_spec_json: String,
    #[serde(default)]
    cooldown_seconds: i32,
}

fn default_mesh_task() -> String {
    "mesh_task".to_string()
}

fn row_to_trigger(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "event_type": row.get::<String, _>("event_type"),
        "predicate_json": row.get::<Option<String>, _>("predicate_json"),
        "target_type": row.get::<String, _>("target_type"),
        "target_spec_json": row.get::<String, _>("target_spec_json"),
        "active": row.get::<bool, _>("active"),
        "cooldown_seconds": row.get::<i32, _>("cooldown_seconds"),
        "last_fired_at": row.get::<Option<chrono::NaiveDateTime>, _>("last_fired_at")
            .map(|t| format!("{}Z", t.format("%Y-%m-%dT%H:%M:%S"))),
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

async fn create_trigger(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<TriggerCreate>,
) -> impl IntoResponse {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    match sqlx::query(
        "INSERT INTO eventtrigger \
         (id, owner_subject, event_type, predicate_json, target_type, target_spec_json, \
          active, cooldown_seconds, last_fired_at, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,true,$7,NULL,$8,$8) RETURNING *",
    )
    .bind(&id)
    .bind(&principal.subject)
    .bind(&body.event_type)
    .bind(&body.predicate_json)
    .bind(&body.target_type)
    .bind(&body.target_spec_json)
    .bind(body.cooldown_seconds)
    .bind(now)
    .fetch_one(&state.db)
    .await
    {
        Ok(row) => (StatusCode::CREATED, Json(row_to_trigger(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_trigger: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_triggers(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    match sqlx::query(
        "SELECT * FROM eventtrigger \
         WHERE owner_subject=$1 AND active=true \
         ORDER BY created_at DESC",
    )
    .bind(&principal.subject)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_trigger).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_triggers: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_trigger(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match sqlx::query("SELECT * FROM eventtrigger WHERE id=$1")
        .bind(&id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(row)) => {
            let owner: String = row.get("owner_subject");
            if owner != principal.subject && !principal.is_admin {
                return not_found();
            }
            Json(row_to_trigger(&row)).into_response()
        }
        Ok(None) => not_found(),
        Err(e) => {
            tracing::error!("get_trigger: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn deactivate_trigger(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Verify ownership first
    let existing = sqlx::query(
        "SELECT owner_subject FROM eventtrigger WHERE id=$1",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await;

    match existing {
        Ok(Some(row)) => {
            let owner: String = row.get("owner_subject");
            if owner != principal.subject && !principal.is_admin {
                return not_found();
            }
        }
        Ok(None) => return not_found(),
        Err(e) => {
            tracing::error!("deactivate_trigger fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let now = Utc::now().naive_utc();
    match sqlx::query(
        "UPDATE eventtrigger SET active=false, updated_at=$2 WHERE id=$1",
    )
    .bind(&id)
    .bind(now)
    .execute(&state.db)
    .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("deactivate_trigger update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
