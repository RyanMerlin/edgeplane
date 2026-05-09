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

use crate::{auth::Principal, state::AppState};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/budgets", get(list_budget_policies).post(create_budget_policy))
        .route("/budgets/usage", get(get_usage_summary))
        .route("/budgets/usage/batch", post(record_usage_batch))
        .route("/budgets/{policy_id}", get(get_budget_policy).delete(delete_budget_policy))
        .route("/budgets/{policy_id}/override", post(override_budget_window))
}

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct BudgetPolicyCreate {
    scope_type: String,
    scope_id: String,
    window_type: String,
    hard_cap_cents: i32,
    soft_cap_cents: Option<i32>,
    token_hard_cap: Option<i32>,
    token_soft_cap: Option<i32>,
    #[serde(default = "default_alert_only")]
    action_on_breach: String,
}

fn default_alert_only() -> String {
    "alert_only".to_string()
}

#[derive(serde::Deserialize)]
struct UsageBatchItem {
    runtime_kind: String,
    #[serde(default)]
    input_tokens: i32,
    #[serde(default)]
    output_tokens: i32,
    #[serde(default)]
    reasoning_tokens: i32,
    #[serde(default)]
    tool_calls: i32,
    #[serde(default)]
    wall_ms: i32,
    #[serde(default = "default_unknown")]
    provider: String,
    #[serde(default = "default_unknown")]
    model: String,
    run_id: Option<String>,
    mesh_task_id: Option<String>,
    mesh_agent_id: Option<String>,
    mission_id: Option<String>,
    kluster_id: Option<String>,
    #[serde(default = "default_adapter")]
    source: String,
}

fn default_unknown() -> String {
    "unknown".to_string()
}

fn default_adapter() -> String {
    "adapter".to_string()
}

#[derive(serde::Deserialize)]
struct UsageBatchRequest {
    records: Vec<UsageBatchItem>,
}

#[derive(serde::Deserialize)]
struct UsageSummaryQuery {
    mission_id: Option<String>,
    since: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn not_found(msg: &str) -> axum::response::Response {
    (axum::http::StatusCode::NOT_FOUND, axum::Json(serde_json::json!({"detail": msg}))).into_response()
}

fn row_to_policy(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<String, _>("id"),
        "owner_subject": row.get::<String, _>("owner_subject"),
        "scope_type": row.get::<String, _>("scope_type"),
        "scope_id": row.get::<String, _>("scope_id"),
        "window_type": row.get::<String, _>("window_type"),
        "hard_cap_cents": row.get::<i32, _>("hard_cap_cents"),
        "soft_cap_cents": row.get::<Option<i32>, _>("soft_cap_cents"),
        "token_hard_cap": row.get::<Option<i32>, _>("token_hard_cap"),
        "token_soft_cap": row.get::<Option<i32>, _>("token_soft_cap"),
        "action_on_breach": row.get::<String, _>("action_on_breach"),
        "active": row.get::<bool, _>("active"),
        "created_at": row.get::<chrono::NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<chrono::NaiveDateTime, _>("updated_at"),
    })
}

fn parse_since(s: &str) -> Option<chrono::NaiveDateTime> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ")
        .ok()
        .or_else(|| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok())
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn create_budget_policy(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<BudgetPolicyCreate>,
) -> impl IntoResponse {
    let policy_id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        "INSERT INTO budgetpolicy \
         (id, owner_subject, scope_type, scope_id, window_type, hard_cap_cents, soft_cap_cents, \
          token_hard_cap, token_soft_cap, action_on_breach, active, created_at, updated_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,true,$11,$11) RETURNING *",
    )
    .bind(&policy_id)
    .bind(&principal.subject)
    .bind(&body.scope_type)
    .bind(&body.scope_id)
    .bind(&body.window_type)
    .bind(body.hard_cap_cents)
    .bind(body.soft_cap_cents)
    .bind(body.token_hard_cap)
    .bind(body.token_soft_cap)
    .bind(&body.action_on_breach)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => (StatusCode::CREATED, Json(row_to_policy(&row))).into_response(),
        Err(e) => {
            tracing::error!("create_budget_policy: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_budget_policies(
    State(state): State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    let rows = sqlx::query(
        "SELECT * FROM budgetpolicy WHERE owner_subject=$1 AND active=true ORDER BY created_at DESC",
    )
    .bind(&principal.subject)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(rows) => Json(rows.iter().map(row_to_policy).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_budget_policies: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_budget_policy(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(policy_id): Path<String>,
) -> impl IntoResponse {
    match sqlx::query("SELECT * FROM budgetpolicy WHERE id=$1 AND owner_subject=$2")
        .bind(&policy_id)
        .bind(&principal.subject)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(row)) => Json(row_to_policy(&row)).into_response(),
        Ok(None) => not_found("Budget policy not found"),
        Err(e) => {
            tracing::error!("get_budget_policy: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn delete_budget_policy(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(policy_id): Path<String>,
) -> impl IntoResponse {
    // Check ownership first
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM budgetpolicy WHERE id=$1 AND owner_subject=$2)",
    )
    .bind(&policy_id)
    .bind(&principal.subject)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);

    if !exists {
        return not_found("Budget policy not found");
    }

    let now = Utc::now().naive_utc();
    let result = sqlx::query(
        "UPDATE budgetpolicy SET active=false, updated_at=$2 WHERE id=$1 AND owner_subject=$3",
    )
    .bind(&policy_id)
    .bind(now)
    .bind(&principal.subject)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => Json(serde_json::json!({"deleted": policy_id})).into_response(),
        Err(e) => {
            tracing::error!("delete_budget_policy: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn override_budget_window(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(policy_id): Path<String>,
) -> impl IntoResponse {
    // Check policy ownership (owner_subject OR principal.is_admin)
    let owns = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM budgetpolicy WHERE id=$1 AND owner_subject=$2)",
    )
    .bind(&policy_id)
    .bind(&principal.subject)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);

    if !owns && !principal.is_admin {
        return not_found("Budget policy not found");
    }

    // SELECT latest window
    let window_row = sqlx::query(
        "SELECT * FROM budgetwindow WHERE policy_id=$1 ORDER BY window_start DESC LIMIT 1",
    )
    .bind(&policy_id)
    .fetch_optional(&state.db)
    .await;

    let window_row = match window_row {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("No active window found"),
        Err(e) => {
            tracing::error!("override_budget_window fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let window_id: String = window_row.get("id");
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        "UPDATE budgetwindow SET state='open', updated_at=$2 WHERE id=$1",
    )
    .bind(&window_id)
    .bind(now)
    .execute(&state.db)
    .await;

    match result {
        Ok(_) => Json(serde_json::json!({"reset": policy_id})).into_response(),
        Err(e) => {
            tracing::error!("override_budget_window update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_usage_summary(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<UsageSummaryQuery>,
) -> impl IntoResponse {
    let since_dt = q.since.as_deref().and_then(parse_since);

    let row = sqlx::query(
        "SELECT \
           COALESCE(SUM(cost_cents), 0) as total_cost_cents, \
           COALESCE(SUM(input_tokens), 0) as total_input_tokens, \
           COALESCE(SUM(output_tokens), 0) as total_output_tokens, \
           COALESCE(SUM(tool_calls), 0) as total_tool_calls, \
           COUNT(*) as record_count \
         FROM usagerecord \
         WHERE owner_subject=$1 \
           AND ($2::text IS NULL OR mission_id=$2) \
           AND ($3::timestamp IS NULL OR recorded_at >= $3)",
    )
    .bind(&principal.subject)
    .bind(&q.mission_id)
    .bind(since_dt)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(row) => Json(serde_json::json!({
            "total_cost_cents": row.get::<i64, _>("total_cost_cents"),
            "total_input_tokens": row.get::<i64, _>("total_input_tokens"),
            "total_output_tokens": row.get::<i64, _>("total_output_tokens"),
            "total_tool_calls": row.get::<i64, _>("total_tool_calls"),
            "record_count": row.get::<i64, _>("record_count"),
        }))
        .into_response(),
        Err(e) => {
            tracing::error!("get_usage_summary: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn record_usage_batch(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<UsageBatchRequest>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();
    let count = body.records.len();

    for record in &body.records {
        let record_id = Uuid::new_v4().to_string();
        let result = sqlx::query(
            "INSERT INTO usagerecord \
             (id, owner_subject, run_id, mesh_task_id, mesh_agent_id, mission_id, kluster_id, \
              runtime_kind, provider, model, input_tokens, output_tokens, reasoning_tokens, \
              tool_calls, wall_ms, cost_cents, recorded_at, source) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,0,$16,$17)",
        )
        .bind(&record_id)
        .bind(&principal.subject)
        .bind(&record.run_id)
        .bind(&record.mesh_task_id)
        .bind(&record.mesh_agent_id)
        .bind(&record.mission_id)
        .bind(&record.kluster_id)
        .bind(&record.runtime_kind)
        .bind(&record.provider)
        .bind(&record.model)
        .bind(record.input_tokens)
        .bind(record.output_tokens)
        .bind(record.reasoning_tokens)
        .bind(record.tool_calls)
        .bind(record.wall_ms)
        .bind(now)
        .bind(&record.source)
        .execute(&state.db)
        .await;

        if let Err(e) = result {
            tracing::error!("record_usage_batch insert: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    Json(serde_json::json!({"recorded": count})).into_response()
}
