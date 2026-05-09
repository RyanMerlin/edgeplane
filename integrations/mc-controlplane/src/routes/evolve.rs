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
        .route("/evolve/missions", post(seed_mission))
        .route("/evolve/missions/{mission_id}/run", post(run_mission))
        .route("/evolve/missions/{mission_id}/status", get(mission_status))
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct EvolveSpec {
    spec: serde_json::Value,
}

#[derive(Deserialize)]
struct EvolveRunRequest {
    runtime_kind: String,
    #[allow(dead_code)]
    agent: String,
    #[allow(dead_code)]
    policy: Option<serde_json::Value>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn new_evolve_id(prefix: &str) -> String {
    let raw = Uuid::new_v4().to_string().replace('-', "");
    format!("{}-{}", prefix, &raw[..8])
}

fn not_found(msg: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"detail": msg})),
    )
        .into_response()
}

/// Count the number of tasks in a spec JSON blob.
fn count_tasks(spec_json: &str) -> i64 {
    serde_json::from_str::<serde_json::Value>(spec_json)
        .ok()
        .and_then(|v| v.get("tasks").and_then(|t| t.as_array()).map(|a| a.len() as i64))
        .unwrap_or(0)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn seed_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(payload): Json<EvolveSpec>,
) -> impl IntoResponse {
    let mission_id = new_evolve_id("evolve");
    let owner_subject = principal.subject.clone();
    let spec_json = serde_json::to_string(&payload.spec).unwrap_or_else(|_| "{}".into());
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        "INSERT INTO evolvemission \
         (mission_id, owner_subject, status, spec_json, created_at, updated_at) \
         VALUES ($1, $2, 'seeded', $3, $4, $4) \
         RETURNING mission_id, status, created_at, spec_json",
    )
    .bind(&mission_id)
    .bind(&owner_subject)
    .bind(&spec_json)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => {
            let returned_mission_id: String = row.get("mission_id");
            let status: String = row.get("status");
            let created_at: chrono::NaiveDateTime = row.get("created_at");
            let returned_spec_json: String = row.get("spec_json");
            let task_count = count_tasks(&returned_spec_json);

            Json(serde_json::json!({
                "mission_id": returned_mission_id,
                "status": status,
                "created_at": created_at,
                "task_count": task_count,
            }))
            .into_response()
        }
        Err(e) => {
            tracing::error!("seed_mission: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn run_mission(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
    Json(payload): Json<EvolveRunRequest>,
) -> impl IntoResponse {
    // Verify mission exists and caller is the owner
    let mission_row = sqlx::query(
        "SELECT mission_id, owner_subject, status FROM evolvemission WHERE mission_id = $1",
    )
    .bind(&mission_id)
    .fetch_optional(&state.db)
    .await;

    let mission_row = match mission_row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("run_mission fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let owner_subject: String = mission_row.get("owner_subject");
    if owner_subject != principal.subject && !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "not the mission owner"})),
        )
            .into_response();
    }

    let run_id = new_evolve_id("run");
    let now = Utc::now().naive_utc();

    // Insert EvolveRun
    let run_result = sqlx::query(
        "INSERT INTO evolverun \
         (run_id, mission_id, owner_subject, agent, status, started_at) \
         VALUES ($1, $2, $3, $4, 'running', $5) \
         RETURNING run_id, mission_id, agent, status, started_at",
    )
    .bind(&run_id)
    .bind(&mission_id)
    .bind(&owner_subject)
    .bind(&payload.runtime_kind)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    let run_row = match run_result {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("run_mission insert run: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Update mission status to "running"
    let _ = sqlx::query(
        "UPDATE evolvemission SET status = 'running', updated_at = $2 WHERE mission_id = $1",
    )
    .bind(&mission_id)
    .bind(now)
    .execute(&state.db)
    .await;

    let returned_run_id: String = run_row.get("run_id");
    let returned_mission_id: String = run_row.get("mission_id");
    let agent: String = run_row.get("agent");
    let status: String = run_row.get("status");
    let started_at: chrono::NaiveDateTime = run_row.get("started_at");

    Json(serde_json::json!({
        "mission_id": returned_mission_id,
        "run_id": returned_run_id,
        "agent": agent,
        "runtime_kind": payload.runtime_kind,
        "status": status,
        "started_at": started_at,
        "ai_session_id": null,
    }))
    .into_response()
}

async fn mission_status(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    // Fetch mission; verify owner
    let mission_row = sqlx::query(
        "SELECT mission_id, owner_subject, status, spec_json, created_at FROM evolvemission WHERE mission_id = $1",
    )
    .bind(&mission_id)
    .fetch_optional(&state.db)
    .await;

    let mission_row = match mission_row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("mission_status fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let owner_subject: String = mission_row.get("owner_subject");
    if owner_subject != principal.subject && !principal.is_admin {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "not the mission owner"})),
        )
            .into_response();
    }

    let status: String = mission_row.get("status");
    let spec_json: String = mission_row.get("spec_json");
    let created_at: chrono::NaiveDateTime = mission_row.get("created_at");
    let task_count = count_tasks(&spec_json);

    // Fetch runs
    let runs_result = sqlx::query(
        "SELECT run_id, mission_id, owner_subject, agent, status, started_at, ai_session_id, score, recipe_path \
         FROM evolverun WHERE mission_id = $1 ORDER BY started_at ASC",
    )
    .bind(&mission_id)
    .fetch_all(&state.db)
    .await;

    let runs = match runs_result {
        Ok(rows) => rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "run_id": r.get::<String, _>("run_id"),
                    "mission_id": r.get::<String, _>("mission_id"),
                    "owner_subject": r.get::<String, _>("owner_subject"),
                    "agent": r.get::<String, _>("agent"),
                    "status": r.get::<String, _>("status"),
                    "started_at": r.get::<chrono::NaiveDateTime, _>("started_at"),
                    "ai_session_id": r.get::<Option<String>, _>("ai_session_id"),
                    "score": r.get::<Option<f64>, _>("score"),
                    "recipe_path": r.get::<Option<String>, _>("recipe_path"),
                })
            })
            .collect::<Vec<_>>(),
        Err(e) => {
            tracing::error!("mission_status fetch runs: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let run_count = runs.len() as i64;

    Json(serde_json::json!({
        "mission_id": mission_id,
        "status": status,
        "created_at": created_at,
        "task_count": task_count,
        "run_count": run_count,
        "runs": runs,
    }))
    .into_response()
}
