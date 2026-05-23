use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    auth::Principal,
    models::run::{CheckpointRequest, ListRunsQuery, ResumeRequest, StartRunRequest},
    state::AppState,
};

const TERMINAL_STATUSES: &[&str] = &["completed", "failed", "cancelled"];

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/runs", get(list_runs).post(start_run))
        .route("/runs/{run_id}", get(get_run))
        .route("/runs/{run_id}/pause", post(pause_run))
        .route("/runs/{run_id}/cancel", post(cancel_run))
        .route("/runs/{run_id}/complete", post(complete_run))
        .route("/runs/{run_id}/completed", post(complete_run))
        .route("/runs/{run_id}/fail", post(fail_run))
        .route("/runs/{run_id}/failed", post(fail_run))
        .route("/runs/{run_id}/resume", post(resume_run))
        .route("/runs/{run_id}/checkpoint", post(create_checkpoint))
        .route("/runs/{run_id}/checkpoints", get(list_checkpoints))
}

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

// ── Runs ──────────────────────────────────────────────────────────────────────

async fn list_runs(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<ListRunsQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(200);
    // Build query with optional filters
    let rows = sqlx::query(
        "SELECT * FROM agentrun WHERE owner_subject = $1 \
         AND ($2::text IS NULL OR mesh_agent_id = $2) \
         AND ($3::text IS NULL OR mesh_task_id = $3) \
         AND ($4::text IS NULL OR status = $4) \
         ORDER BY started_at DESC NULLS LAST LIMIT $5",
    )
    .bind(&principal.subject)
    .bind(&q.agent_id)
    .bind(&q.task_id)
    .bind(&q.status)
    .bind(limit)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_run).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_runs: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn start_run(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<StartRunRequest>,
) -> impl IntoResponse {
    // Idempotency: return existing run if key matches
    if let Some(ref key) = body.idempotency_key {
        let existing = sqlx::query(
            "SELECT * FROM agentrun WHERE owner_subject=$1 AND idempotency_key=$2",
        )
        .bind(&principal.subject)
        .bind(key)
        .fetch_optional(&state.db)
        .await;
        if let Ok(Some(row)) = existing {
            return Json(row_to_run(&row)).into_response();
        }
    }

    let run_id = Uuid::new_v4().to_string();
    let resume_token = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        "INSERT INTO agentrun \
         (id, owner_subject, mesh_agent_id, mesh_task_id, runtime_kind, runtime_session_id, \
          idempotency_key, status, resume_token, started_at, total_cost_cents, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,'starting',$8,$9,0,$9,$9) RETURNING *",
    )
    .bind(&run_id)
    .bind(&principal.subject)
    .bind(&body.agent_id)
    .bind(&body.task_id)
    .bind(&body.runtime_kind)
    .bind(&body.runtime_session_id)
    .bind(&body.idempotency_key)
    .bind(&resume_token)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => (StatusCode::OK, Json(row_to_run(&row))).into_response(),
        Err(e) => {
            tracing::error!("start_run: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_run(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    match sqlx::query("SELECT * FROM agentrun WHERE id=$1 AND owner_subject=$2")
        .bind(&run_id)
        .bind(&principal.subject)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(row)) => Json(row_to_run(&row)).into_response(),
        Ok(None) => not_found("Run not found"),
        Err(e) => {
            tracing::error!("get_run: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn pause_run(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    transition(&state, &run_id, "paused", &principal.subject).await
}

async fn cancel_run(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    transition(&state, &run_id, "cancelled", &principal.subject).await
}

async fn complete_run(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    transition(&state, &run_id, "completed", &principal.subject).await
}

async fn fail_run(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    transition(&state, &run_id, "failed", &principal.subject).await
}

async fn resume_run(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(run_id): Path<String>,
    Json(body): Json<ResumeRequest>,
) -> impl IntoResponse {
    let run_row = sqlx::query(
        "SELECT * FROM agentrun WHERE id=$1 AND owner_subject=$2",
    )
    .bind(&run_id)
    .bind(&principal.subject)
    .fetch_optional(&state.db)
    .await;

    let run_row = match run_row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Run not found"),
        Err(e) => {
            tracing::error!("resume_run lookup: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let stored_token: String = run_row.get("resume_token");
    if stored_token != body.resume_token {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "invalid_resume_token"})),
        )
            .into_response();
    }

    let last_cp = sqlx::query(
        "SELECT * FROM runcheckpoint WHERE run_id=$1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(&run_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
    .map(|r| row_to_checkpoint(&r));

    Json(serde_json::json!({
        "run": row_to_run(&run_row),
        "last_checkpoint": last_cp,
        "resume_token": body.resume_token,
    }))
    .into_response()
}

async fn list_checkpoints(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    // Verify ownership
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM agentrun WHERE id=$1 AND owner_subject=$2)",
    )
    .bind(&run_id)
    .bind(&principal.subject)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);

    if !owns {
        return not_found("Run not found");
    }

    match sqlx::query("SELECT * FROM runcheckpoint WHERE run_id=$1 ORDER BY seq")
        .bind(&run_id)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_checkpoint).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_checkpoints: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create_checkpoint(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(run_id): Path<String>,
    Json(body): Json<CheckpointRequest>,
) -> impl IntoResponse {
    // Verify ownership
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM agentrun WHERE id=$1 AND owner_subject=$2)",
    )
    .bind(&run_id)
    .bind(&principal.subject)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);

    if !owns {
        return not_found("Run not found");
    }

    let cp_id = Uuid::new_v4().to_string();
    let payload_json = serde_json::to_string(&body.payload).unwrap_or_else(|_| "{}".into());
    let now = Utc::now().naive_utc();

    // Get next seq (monotonic, handles concurrent writers via retry at DB level)
    let result = sqlx::query(
        "INSERT INTO runcheckpoint (id, run_id, seq, kind, payload_json, created_at) \
         VALUES ($1, $2, \
           COALESCE((SELECT MAX(seq)+1 FROM runcheckpoint WHERE run_id=$2), 0), \
           $3, $4, $5) RETURNING *",
    )
    .bind(&cp_id)
    .bind(&run_id)
    .bind(&body.kind)
    .bind(&payload_json)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    // Also bump last_checkpoint_at on the run
    let _ = sqlx::query(
        "UPDATE agentrun SET last_checkpoint_at=$2, updated_at=$2 WHERE id=$1",
    )
    .bind(&run_id)
    .bind(now)
    .execute(&state.db)
    .await;

    match result {
        Ok(row) => (StatusCode::OK, Json(row_to_checkpoint(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_checkpoint: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn transition(
    state: &AppState,
    run_id: &str,
    new_status: &str,
    owner_subject: &str,
) -> axum::response::Response {
    let now = Utc::now().naive_utc();
    let is_terminal = TERMINAL_STATUSES.contains(&new_status);

    let result = if is_terminal {
        sqlx::query(
            "UPDATE agentrun SET status=$3, ended_at=$4, updated_at=$4 \
             WHERE id=$1 AND owner_subject=$2 RETURNING *",
        )
        .bind(run_id)
        .bind(owner_subject)
        .bind(new_status)
        .bind(now)
        .fetch_optional(&state.db)
        .await
    } else {
        sqlx::query(
            "UPDATE agentrun SET status=$3, updated_at=$4 \
             WHERE id=$1 AND owner_subject=$2 RETURNING *",
        )
        .bind(run_id)
        .bind(owner_subject)
        .bind(new_status)
        .bind(now)
        .fetch_optional(&state.db)
        .await
    };

    match result {
        Ok(Some(row)) => Json(row_to_run(&row)).into_response(),
        Ok(None) => not_found("Run not found"),
        Err(e) => {
            tracing::error!("transition {new_status}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn row_to_run(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "mesh_agent_id": row.get::<Option<String>, _>("mesh_agent_id"),
        "mesh_task_id": row.get::<Option<String>, _>("mesh_task_id"),
        "runtime_kind": row.get::<String, _>("runtime_kind"),
        "runtime_session_id": row.get::<Option<String>, _>("runtime_session_id"),
        "status": row.get::<String, _>("status"),
        "started_at": row.get::<Option<chrono::NaiveDateTime>, _>("started_at"),
        "ended_at": row.get::<Option<chrono::NaiveDateTime>, _>("ended_at"),
        "resume_token": row.get::<String, _>("resume_token"),
        "last_checkpoint_at": row.get::<Option<chrono::NaiveDateTime>, _>("last_checkpoint_at"),
        "total_cost_cents": row.get::<i32, _>("total_cost_cents"),
        "parent_run_id": row.get::<Option<String>, _>("parent_run_id"),
        "metadata_json": row.get::<Option<String>, _>("metadata_json"),
        "idempotency_key": row.get::<Option<String>, _>("idempotency_key"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_checkpoint(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "run_id": row.get::<String, _>("run_id"),
        "seq": row.get::<i32, _>("seq"),
        "kind": row.get::<String, _>("kind"),
        "payload_json": row.get::<String, _>("payload_json"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
    })
}
