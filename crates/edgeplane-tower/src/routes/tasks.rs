use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::{NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use crate::{auth::Principal, state::AppState};

/// Status a `create_task` row starts in. `update_task` mints a completion
/// token (`claim_lease_id`) the moment a task's status first moves away from
/// this value — see migration 0014's design doc (task/meshtask unification):
/// the completion token is minted for both `kind='claimable'` (on claim) and
/// `kind='assigned'` (here, on first status transition = "work started").
pub(crate) const INITIAL_STATUS: &str = "proposed";

/// Status strings that mark an assigned task as terminal (stamp `finalized_at`
/// here; also used by `routes::work`'s complete/fail/cancel handlers to gate
/// the kind='assigned' branch now that completion is dispatched by kind
/// rather than rejected outright for assigned rows).
/// Assigned tasks use free-text, PM-style status vocabulary (not the
/// claimable-pool's `finished`/`failed`/`cancelled`) — `"done"` is the
/// established legacy convention (see `web/src/routes/domains.test.tsx`'s
/// `task_status_counts: { open, done }` fixture); `finished`/`failed`/
/// `cancelled` are accepted too since nothing stops a caller from reusing the
/// claimable vocabulary on an assigned task's freeform status field.
pub(crate) fn is_terminal_status(s: &str) -> bool {
    matches!(s, "done" | "finished" | "failed" | "cancelled")
}

/// An assigned task (`kind='assigned'` row of the unified `task` table,
/// migration 0014's task/meshtask merge) — the human/PM-facing shape that was
/// the standalone `task` table before the merge. Column names on the unified
/// table changed (`definition_of_done` -> `done_criteria`,
/// `dependencies`/`related_artifacts` -> `*_note`, both now display-only
/// text); `#[serde(rename)]` keeps the JSON wire shape byte-compatible with
/// the pre-merge `models::task::Task` struct this replaces, since CLI/TUI/web
/// callers of this endpoint are a separate, later PR (PR2).
///
/// `description`/`owner`/`contributors`/`done_criteria`/`dependencies_note`/
/// `related_artifacts_note` are modeled as non-`Option` here (not `NULL`-safe)
/// even though the unified column definitions are nullable at the DB level
/// (inherited from meshtask's already-nullable definitions, not the legacy
/// task table's `NOT NULL` constraints) — `create_task`/migrated rows always
/// supply a real (possibly empty) string for these, by construction, for
/// every `kind='assigned'` row. This is a conscious, slightly weaker
/// guarantee than the DB enforced pre-merge; a stray out-of-band `NULL` would
/// surface as a graceful decode error (500), not a panic.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AssignedTask {
    pub id: String,
    pub public_id: String,
    pub mission_id: String,
    pub domain_id: String,
    pub parent_task_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub owner: String,
    pub contributors: String,
    #[serde(rename = "dependencies")]
    pub dependencies_note: String,
    #[serde(rename = "definition_of_done")]
    pub done_criteria: String,
    #[serde(rename = "related_artifacts")]
    pub related_artifacts_note: String,
    /// `integer`, matching `artifact.id` — was `varchar` pre-migration.
    pub result_artifact_id: Option<i32>,
    /// Completion token — minted for this row the moment its status first
    /// moves away from `INITIAL_STATUS` (see `update_task`). Validated the
    /// same way as a claimable task's lease id via
    /// `authz::authz_task_owner` (extended to check `owner` too).
    pub claim_lease_id: Option<String>,
    pub attempt: i16,
    pub max_attempts: i16,
    pub finalized_at: Option<chrono::DateTime<Utc>>,
    pub created_by_subject: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

const TASK_COLUMNS: &str = "id, public_id, mission_id, domain_id, parent_task_id, kind, title, \
     description, status, owner, contributors, dependencies_note, done_criteria, \
     related_artifacts_note, result_artifact_id, claim_lease_id, attempt, max_attempts, \
     finalized_at, created_by_subject, created_at, updated_at";

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

/// Validates that the URL's `mission_id`/`domain_id` pair are actually
/// related — a consistency check, not an authorization decision. Extracted
/// from the former `domain_access` (which ran this unconditionally after its
/// authorization checks, regardless of mode); preserved verbatim and called
/// at every task route call site, always *after* the authz gate, matching
/// `domain_access`'s original ordering.
async fn verify_mission_domain(
    db: &sqlx::PgPool,
    mission_id: &str,
    domain_id: &str,
) -> Result<(), axum::response::Response> {
    let k: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mission WHERE id=$1 AND domain_id=$2")
        .bind(mission_id).bind(domain_id)
        .fetch_optional(db).await.unwrap_or(None);
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
    if let Err(r) = crate::routes::authz::authz_domain_readable(&state.db, &principal, &domain_id).await { return r; }
    if let Err(r) = verify_mission_domain(&state.db, &mission_id, &domain_id).await { return r; }
    let limit = q.limit.unwrap_or(100).min(500);
    let rows = if let Some(s) = &q.status {
        sqlx::query_as::<_, AssignedTask>(
            &format!("SELECT {TASK_COLUMNS} FROM task WHERE mission_id=$1 AND kind='assigned' AND status=$2 ORDER BY updated_at DESC LIMIT $3")
        )
        .bind(&mission_id).bind(s).bind(limit).fetch_all(&state.db).await
    } else {
        sqlx::query_as::<_, AssignedTask>(
            &format!("SELECT {TASK_COLUMNS} FROM task WHERE mission_id=$1 AND kind='assigned' ORDER BY updated_at DESC LIMIT $2")
        )
        .bind(&mission_id).bind(limit).fetch_all(&state.db).await
    };
    match rows {
        Ok(tasks) => Json(tasks).into_response(),
        Err(e) => { tracing::error!("list_tasks: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

#[derive(Debug, Deserialize)]
struct TaskCreate {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_initial_status")]
    pub status: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub contributors: String,
    #[serde(default, rename = "dependencies")]
    pub dependencies_note: String,
    #[serde(default, rename = "definition_of_done")]
    pub done_criteria: String,
    #[serde(default, rename = "related_artifacts")]
    pub related_artifacts_note: String,
}
fn default_initial_status() -> String { INITIAL_STATUS.to_string() }

async fn create_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id)): Path<(String, String)>,
    Json(payload): Json<TaskCreate>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await { return r; }
    if let Err(r) = verify_mission_domain(&state.db, &mission_id, &domain_id).await { return r; }
    if payload.title.trim().is_empty() {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({"detail": "title is required"}))).into_response();
    }

    let id = Uuid::new_v4().to_string();
    let public_id = crate::routes::work::new_public_id();
    let now = Utc::now().naive_utc();
    match sqlx::query_as::<_, AssignedTask>(
        &format!(
            "INSERT INTO task (id, public_id, mission_id, domain_id, kind, title, description, \
             status, owner, contributors, dependencies_note, done_criteria, related_artifacts_note, \
             priority, version_counter, created_by_subject, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,'assigned',$5,$6,$7,$8,$9,$10,$11,$12,0,0,$13,$14,$14) \
             RETURNING {TASK_COLUMNS}"
        )
    )
    .bind(&id).bind(&public_id).bind(&mission_id).bind(&domain_id).bind(payload.title.trim())
    .bind(&payload.description).bind(&payload.status).bind(&payload.owner)
    .bind(&payload.contributors).bind(&payload.dependencies_note)
    .bind(&payload.done_criteria).bind(&payload.related_artifacts_note)
    .bind(&principal.subject)
    .bind(now)
    .fetch_one(&state.db).await {
        Ok(t) => (StatusCode::OK, Json(t)).into_response(),
        Err(e) => { tracing::error!("create_task: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

/// Fetch by public_id first, then the raw `id` (both varchar post-unification;
/// there is no more numeric id to branch on).
async fn fetch_assigned_task(
    db: &sqlx::PgPool,
    task_id: &str,
    mission_id: &str,
) -> Result<Option<AssignedTask>, sqlx::Error> {
    let by_public_id = sqlx::query_as::<_, AssignedTask>(
        &format!("SELECT {TASK_COLUMNS} FROM task WHERE public_id=$1 AND mission_id=$2 AND kind='assigned'")
    )
    .bind(task_id).bind(mission_id).fetch_optional(db).await?;
    if by_public_id.is_some() {
        return Ok(by_public_id);
    }
    sqlx::query_as::<_, AssignedTask>(
        &format!("SELECT {TASK_COLUMNS} FROM task WHERE id=$1 AND mission_id=$2 AND kind='assigned'")
    )
    .bind(task_id).bind(mission_id).fetch_optional(db).await
}

async fn get_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id, task_id)): Path<(String, String, String)>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_domain_readable(&state.db, &principal, &domain_id).await { return r; }
    if let Err(r) = verify_mission_domain(&state.db, &mission_id, &domain_id).await { return r; }

    match fetch_assigned_task(&state.db, &task_id, &mission_id).await {
        Ok(Some(t)) => Json(t).into_response(),
        Ok(None) => not_found("Task not found"),
        Err(e) => { tracing::error!("get_task: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}

#[derive(Debug, Deserialize)]
struct TaskUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub owner: Option<String>,
    pub contributors: Option<String>,
    #[serde(rename = "dependencies")]
    pub dependencies_note: Option<String>,
    #[serde(rename = "definition_of_done")]
    pub done_criteria: Option<String>,
    #[serde(rename = "related_artifacts")]
    pub related_artifacts_note: Option<String>,
}

async fn update_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path((domain_id, mission_id, task_id)): Path<(String, String, String)>,
    Json(payload): Json<TaskUpdate>,
) -> impl IntoResponse {
    if let Err(r) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await { return r; }
    if let Err(r) = verify_mission_domain(&state.db, &mission_id, &domain_id).await { return r; }

    let task = match fetch_assigned_task(&state.db, &task_id, &mission_id).await {
        Ok(Some(t)) => t,
        Ok(None) => return not_found("Task not found"),
        Err(e) => { tracing::error!("update_task fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let title                = payload.title.unwrap_or(task.title);
    let description          = payload.description.unwrap_or(task.description);
    let status               = payload.status.unwrap_or_else(|| task.status.clone());
    let owner                = payload.owner.unwrap_or(task.owner);
    let contributors         = payload.contributors.unwrap_or(task.contributors);
    let dependencies_note    = payload.dependencies_note.unwrap_or(task.dependencies_note);
    let done_criteria        = payload.done_criteria.unwrap_or(task.done_criteria);
    let related_artifacts_note = payload.related_artifacts_note.unwrap_or(task.related_artifacts_note);

    // Completion-token unification (migration 0014's design point): mint a
    // claim_lease_id the moment this task's status first moves away from its
    // initial value ("work started" / owner acknowledged), the same
    // completion-token concept a claimable task gets on claim. Validated
    // uniformly via authz::authz_task_owner (extended to also check `owner`),
    // reused rather than duplicated.
    let claim_lease_id = if task.claim_lease_id.is_none()
        && task.status == INITIAL_STATUS
        && status != INITIAL_STATUS
    {
        Some(Uuid::new_v4().to_string())
    } else {
        task.claim_lease_id.clone()
    };

    let now = Utc::now().naive_utc();
    // Stamp finalized_at only on the transition *into* a terminal status (not
    // on every subsequent update while already terminal); clear it on the
    // transition *out* (e.g. reopened); otherwise leave it untouched.
    let finalized_at = if !is_terminal_status(&task.status) && is_terminal_status(&status) {
        Some(Utc::now())
    } else if is_terminal_status(&task.status) && !is_terminal_status(&status) {
        None
    } else {
        task.finalized_at
    };

    match sqlx::query_as::<_, AssignedTask>(
        &format!(
            "UPDATE task SET title=$2, description=$3, status=$4, owner=$5, contributors=$6, \
             dependencies_note=$7, done_criteria=$8, related_artifacts_note=$9, \
             claim_lease_id=$10, finalized_at=$11, updated_at=$12 WHERE id=$1 \
             RETURNING {TASK_COLUMNS}"
        )
    )
    .bind(&task.id).bind(&title).bind(&description).bind(&status).bind(&owner)
    .bind(&contributors).bind(&dependencies_note).bind(&done_criteria)
    .bind(&related_artifacts_note).bind(&claim_lease_id).bind(finalized_at).bind(now)
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
    if let Err(r) = crate::routes::authz::authz_domain_owner(&state.db, &principal, &domain_id).await { return r; }
    if let Err(r) = verify_mission_domain(&state.db, &mission_id, &domain_id).await { return r; }

    let task = match fetch_assigned_task(&state.db, &task_id, &mission_id).await {
        Ok(Some(t)) => t,
        Ok(None) => return not_found("Task not found"),
        Err(e) => { tracing::error!("delete_task fetch: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    // taskassignment: dropped by migration 0014 (write-dead — no INSERT/UPDATE/
    // SELECT anywhere in the Rust codebase, only this cleanup DELETE, which is
    // removed along with the table).
    let _ = sqlx::query("DELETE FROM overlapsuggestion WHERE task_id=$1 OR candidate_task_id=$1")
        .bind(&task.id).execute(&state.db).await;
    let _ = sqlx::query("UPDATE agentmessage SET task_id=NULL WHERE task_id=$1").bind(&task.id).execute(&state.db).await;
    let _ = sqlx::query("DELETE FROM task WHERE id=$1").bind(&task.id).execute(&state.db).await;

    Json(serde_json::json!({"ok": true, "deleted_id": task.public_id})).into_response()
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
    match sqlx::query_as::<_, AssignedTask>(
        &format!("SELECT {TASK_COLUMNS} FROM task WHERE mission_id=$1 AND kind='assigned' ORDER BY created_at ASC LIMIT 200")
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
    if let Err(r) = crate::routes::authz::authz_domain_readable(&state.db, &principal, &domain_id).await { return r; }
    if let Err(r) = verify_mission_domain(&state.db, &mission_id, &domain_id).await { return r; }

    let task = match fetch_assigned_task(&state.db, &task_id, &mission_id).await {
        Ok(Some(t)) => t,
        Ok(None) => return not_found("Task not found"),
        Err(e) => { tracing::error!("list_overlaps fetch task: {e}"); return StatusCode::INTERNAL_SERVER_ERROR.into_response(); }
    };

    let limit = q.limit.unwrap_or(20).min(100);
    // overlapsuggestion.task_id/candidate_task_id are `character varying` post
    // migration 0014 (retyped + remapped from the legacy integer task.id) —
    // were `i32`. similarity_score/evidence/suggested_action are the real
    // NOT NULL columns (score/reason never existed — see the MCP
    // get_overlap_suggestions handler for the same column names).
    match sqlx::query(
        "SELECT id, task_id, candidate_task_id, similarity_score, evidence, suggested_action, created_at \
         FROM overlapsuggestion WHERE task_id=$1 ORDER BY similarity_score DESC LIMIT $2"
    )
    .bind(&task.id).bind(limit)
    .fetch_all(&state.db).await {
        Ok(rows) => Json(rows.iter().map(|r| serde_json::json!({
            "id": r.get::<i32,_>("id"),
            "task_id": r.get::<String,_>("task_id"),
            "candidate_task_id": r.get::<String,_>("candidate_task_id"),
            "similarity_score": r.get::<f64,_>("similarity_score"),
            "evidence": r.get::<String,_>("evidence"),
            "suggested_action": r.get::<String,_>("suggested_action"),
            "created_at": r.get::<chrono::NaiveDateTime,_>("created_at"),
        })).collect::<Vec<_>>()).into_response(),
        Err(e) => { tracing::error!("list_overlaps: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}
