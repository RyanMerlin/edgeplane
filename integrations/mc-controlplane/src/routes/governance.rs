use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch, post},
    Json, Router,
};
use chrono::{NaiveDateTime, Utc};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use std::sync::Arc;

use crate::{auth::Principal, state::AppState};

const DEFAULT_POLICY: &str = r#"{"global":{"require_approval_for_mutations":false,"allow_create_without_approval":true,"allow_update":true,"allow_delete":true,"allow_publish":true},"actions":{"mission.create":{"enabled":true,"requires_approval":false},"mission.update":{"enabled":true,"requires_approval":false},"mission.delete":{"enabled":true,"requires_approval":false},"mission.publish":{"enabled":true,"requires_approval":false},"kluster.create":{"enabled":true,"requires_approval":false},"kluster.update":{"enabled":true,"requires_approval":false},"kluster.delete":{"enabled":true,"requires_approval":false},"kluster.publish":{"enabled":true,"requires_approval":false},"task.create":{"enabled":true,"requires_approval":false},"task.update":{"enabled":true,"requires_approval":false},"task.delete":{"enabled":true,"requires_approval":false},"task.publish":{"enabled":true,"requires_approval":false},"doc.create":{"enabled":true,"requires_approval":false},"doc.update":{"enabled":true,"requires_approval":false},"doc.delete":{"enabled":true,"requires_approval":false},"doc.publish":{"enabled":true,"requires_approval":false},"artifact.create":{"enabled":true,"requires_approval":false},"artifact.update":{"enabled":true,"requires_approval":false},"artifact.delete":{"enabled":true,"requires_approval":false},"artifact.publish":{"enabled":true,"requires_approval":false},"skills.bundle.publish":{"enabled":true,"requires_approval":false},"skills.bundle.deprecate":{"enabled":true,"requires_approval":false},"skills.snapshot.resolve":{"enabled":true,"requires_approval":false},"workspace.load":{"enabled":true,"requires_approval":false},"workspace.heartbeat":{"enabled":true,"requires_approval":false},"workspace.fetch_artifact":{"enabled":true,"requires_approval":false},"workspace.commit":{"enabled":true,"requires_approval":false},"workspace.release":{"enabled":true,"requires_approval":false}},"terminal":{"allow_create_actions":true,"allow_publish_actions":false},"mcp":{"allow_mutation_tools":true}}"#;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/governance/policy/active", get(get_active_policy))
        .route("/governance/policy/versions", get(list_policy_versions))
        .route("/governance/policy/drafts", post(create_draft))
        .route("/governance/policy/drafts/{id}", patch(update_draft))
        .route("/governance/policy/drafts/{id}/validate", post(validate_draft))
        .route("/governance/policy/drafts/{id}/publish", post(publish_draft))
        .route("/governance/policy/rollback", post(rollback_policy))
        .route("/governance/policy/reload", post(reload_policy))
        .route("/governance/policy/events", get(list_events))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn not_found(msg: &str) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn forbidden(msg: &str) -> axum::response::Response {
    (StatusCode::FORBIDDEN, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn bad_request(msg: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({"detail": msg}))).into_response()
}

fn row_to_policy(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let policy_json_str: String = row.try_get("policy_json").unwrap_or_default();
    let policy: serde_json::Value = serde_json::from_str(&policy_json_str)
        .unwrap_or_else(|_| serde_json::from_str(DEFAULT_POLICY).unwrap_or(serde_json::json!({})));

    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "version": row.get::<i32, _>("version"),
        "state": row.get::<String, _>("state"),
        "policy": policy,
        "change_note": row.get::<String, _>("change_note"),
        "created_by": row.get::<String, _>("created_by"),
        "published_by": row.get::<String, _>("published_by"),
        "published_at": row.try_get::<Option<NaiveDateTime>, _>("published_at").ok().flatten(),
        "created_at": row.get::<NaiveDateTime, _>("created_at"),
        "updated_at": row.get::<NaiveDateTime, _>("updated_at"),
    })
}

fn row_to_event(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    let detail_json_str: String = row.try_get("detail_json").unwrap_or_default();
    let detail: serde_json::Value = serde_json::from_str(&detail_json_str)
        .unwrap_or(serde_json::json!({}));

    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "policy_id": row.try_get::<Option<i32>, _>("policy_id").ok().flatten(),
        "version": row.get::<i32, _>("version"),
        "event_type": row.get::<String, _>("event_type"),
        "actor_subject": row.get::<String, _>("actor_subject"),
        "detail": detail,
        "created_at": row.get::<NaiveDateTime, _>("created_at"),
    })
}

async fn log_event(
    db: &PgPool,
    policy_id: Option<i32>,
    version: i32,
    event_type: &str,
    actor_subject: &str,
    detail: serde_json::Value,
) {
    let detail_json = serde_json::to_string(&detail).unwrap_or_default();
    let now = Utc::now().naive_utc();
    let _ = sqlx::query(
        "INSERT INTO governancepolicyevent (policy_id, version, event_type, actor_subject, detail_json, created_at) \
         VALUES ($1,$2,$3,$4,$5,$6)",
    )
    .bind(policy_id)
    .bind(version)
    .bind(event_type)
    .bind(actor_subject)
    .bind(&detail_json)
    .bind(now)
    .execute(db)
    .await;
}

// ── query structs ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LimitQuery {
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct CreateDraftBody {
    policy: Option<serde_json::Value>,
    change_note: Option<String>,
}

#[derive(Deserialize)]
struct UpdateDraftBody {
    policy: serde_json::Value,
    change_note: Option<String>,
}

#[derive(Deserialize)]
struct PublishBody {
    change_note: Option<String>,
}

#[derive(Deserialize)]
struct RollbackBody {
    version: i32,
    change_note: Option<String>,
}

// ── handlers ──────────────────────────────────────────────────────────────────

async fn get_active_policy(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
) -> impl IntoResponse {
    // Try to fetch existing active policy
    let existing = sqlx::query(
        "SELECT * FROM governancepolicy WHERE state='active' ORDER BY version DESC LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await;

    match existing {
        Ok(Some(row)) => Json(row_to_policy(&row)).into_response(),
        Ok(None) => {
            // Seed default policy
            let now = Utc::now().naive_utc();
            let result = sqlx::query(
                "INSERT INTO governancepolicy \
                 (version, state, policy_json, change_note, created_by, published_by, published_at, created_at, updated_at) \
                 VALUES (1,'active',$1,'Initial default policy','system','system',$2,$2,$2) RETURNING *",
            )
            .bind(DEFAULT_POLICY)
            .bind(now)
            .fetch_one(&state.db)
            .await;

            match result {
                Ok(row) => {
                    let id: i32 = row.get("id");
                    log_event(
                        &state.db,
                        Some(id),
                        1,
                        "seeded",
                        "system",
                        serde_json::json!({"note": "seeded default policy"}),
                    )
                    .await;
                    (StatusCode::CREATED, Json(row_to_policy(&row))).into_response()
                }
                Err(e) => {
                    tracing::error!("get_active_policy seed: {e}");
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            }
        }
        Err(e) => {
            tracing::error!("get_active_policy: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_policy_versions(
    State(state): State<Arc<AppState>>,
    _principal: Principal,
    Query(q): Query<LimitQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(20).min(200);
    match sqlx::query("SELECT * FROM governancepolicy ORDER BY version DESC LIMIT $1")
        .bind(limit)
        .fetch_all(&state.db)
        .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_policy).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_policy_versions: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn create_draft(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<CreateDraftBody>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return forbidden("admin required");
    }

    let policy_json = match &body.policy {
        Some(p) => serde_json::to_string(p).unwrap_or_else(|_| DEFAULT_POLICY.to_string()),
        None => DEFAULT_POLICY.to_string(),
    };
    let change_note = body.change_note.unwrap_or_default();

    // Get next version
    let max_version: i32 = sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM governancepolicy")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let next_version = max_version + 1;

    let now = Utc::now().naive_utc();
    let result = sqlx::query(
        "INSERT INTO governancepolicy \
         (version, state, policy_json, change_note, created_by, published_by, published_at, created_at, updated_at) \
         VALUES ($1,'draft',$2,$3,$4,'',NULL,$5,$5) RETURNING *",
    )
    .bind(next_version)
    .bind(&policy_json)
    .bind(&change_note)
    .bind(&principal.subject)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => {
            let id: i32 = row.get("id");
            log_event(
                &state.db,
                Some(id),
                next_version,
                "draft_created",
                &principal.subject,
                serde_json::json!({"change_note": change_note}),
            )
            .await;
            (StatusCode::CREATED, Json(row_to_policy(&row))).into_response()
        }
        Err(e) => {
            tracing::error!("create_draft: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_draft(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<i32>,
    Json(body): Json<UpdateDraftBody>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return forbidden("admin required");
    }

    let existing = sqlx::query("SELECT * FROM governancepolicy WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    let row = match existing {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Policy not found"),
        Err(e) => {
            tracing::error!("update_draft fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let state_val: String = row.get("state");
    if state_val != "draft" {
        return bad_request("Only draft policies can be updated");
    }

    let version: i32 = row.get("version");
    let policy_json = serde_json::to_string(&body.policy).unwrap_or_default();
    let change_note = body.change_note.unwrap_or_default();
    let now = Utc::now().naive_utc();

    let result = sqlx::query(
        "UPDATE governancepolicy SET policy_json=$2, change_note=$3, updated_at=$4 WHERE id=$1 RETURNING *",
    )
    .bind(id)
    .bind(&policy_json)
    .bind(&change_note)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(updated) => {
            log_event(
                &state.db,
                Some(id),
                version,
                "draft_updated",
                &principal.subject,
                serde_json::json!({"change_note": change_note}),
            )
            .await;
            Json(row_to_policy(&updated)).into_response()
        }
        Err(e) => {
            tracing::error!("update_draft: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn validate_draft(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return forbidden("admin required");
    }

    let existing = sqlx::query("SELECT * FROM governancepolicy WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    let row = match existing {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Policy not found"),
        Err(e) => {
            tracing::error!("validate_draft fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let policy_json_str: String = row.try_get("policy_json").unwrap_or_default();
    match serde_json::from_str::<serde_json::Value>(&policy_json_str) {
        Ok(v) if v.is_object() => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(_) => Json(serde_json::json!({"ok": false, "error": "policy must be a JSON object"}))
            .into_response(),
        Err(e) => {
            Json(serde_json::json!({"ok": false, "error": e.to_string()})).into_response()
        }
    }
}

async fn publish_draft(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(id): Path<i32>,
    Json(body): Json<PublishBody>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return forbidden("admin required");
    }

    let existing = sqlx::query("SELECT * FROM governancepolicy WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.db)
        .await;

    let row = match existing {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Policy not found"),
        Err(e) => {
            tracing::error!("publish_draft fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let state_val: String = row.get("state");
    if state_val != "draft" {
        return bad_request("Only draft policies can be published");
    }

    let version: i32 = row.get("version");
    let now = Utc::now().naive_utc();

    // Archive current active
    let _ = sqlx::query(
        "UPDATE governancepolicy SET state='archived', updated_at=$1 WHERE state='active'",
    )
    .bind(now)
    .execute(&state.db)
    .await;

    // Publish this draft
    let change_note = body.change_note.unwrap_or_default();
    let result = sqlx::query(
        "UPDATE governancepolicy SET state='active', published_by=$2, published_at=$3, \
         change_note=CASE WHEN $4='' THEN change_note ELSE $4 END, updated_at=$3 \
         WHERE id=$1 RETURNING *",
    )
    .bind(id)
    .bind(&principal.subject)
    .bind(now)
    .bind(&change_note)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(updated) => {
            log_event(
                &state.db,
                Some(id),
                version,
                "published",
                &principal.subject,
                serde_json::json!({"change_note": change_note}),
            )
            .await;
            Json(row_to_policy(&updated)).into_response()
        }
        Err(e) => {
            tracing::error!("publish_draft: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn rollback_policy(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Json(body): Json<RollbackBody>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return forbidden("admin required");
    }

    // Find the target version
    let target = sqlx::query("SELECT * FROM governancepolicy WHERE version=$1 LIMIT 1")
        .bind(body.version)
        .fetch_optional(&state.db)
        .await;

    let target_row = match target {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Policy version not found"),
        Err(e) => {
            tracing::error!("rollback fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let source_policy_json: String = target_row.try_get("policy_json").unwrap_or_default();
    let change_note = body
        .change_note
        .unwrap_or_else(|| format!("Rollback to version {}", body.version));

    // Get next version
    let max_version: i32 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM governancepolicy")
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);
    let next_version = max_version + 1;
    let now = Utc::now().naive_utc();

    // Archive current active
    let _ = sqlx::query(
        "UPDATE governancepolicy SET state='archived', updated_at=$1 WHERE state='active'",
    )
    .bind(now)
    .execute(&state.db)
    .await;

    // Insert as new active
    let result = sqlx::query(
        "INSERT INTO governancepolicy \
         (version, state, policy_json, change_note, created_by, published_by, published_at, created_at, updated_at) \
         VALUES ($1,'active',$2,$3,$4,$4,$5,$5,$5) RETURNING *",
    )
    .bind(next_version)
    .bind(&source_policy_json)
    .bind(&change_note)
    .bind(&principal.subject)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match result {
        Ok(row) => {
            let id: i32 = row.get("id");
            log_event(
                &state.db,
                Some(id),
                next_version,
                "rollback",
                &principal.subject,
                serde_json::json!({"rolled_back_to_version": body.version, "change_note": change_note}),
            )
            .await;
            Json(row_to_policy(&row)).into_response()
        }
        Err(e) => {
            tracing::error!("rollback: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn reload_policy(
    _state: State<Arc<AppState>>,
    principal: Principal,
) -> impl IntoResponse {
    if !principal.is_admin {
        return forbidden("admin required");
    }
    Json(serde_json::json!({"ok": true})).into_response()
}

async fn list_events(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Query(q): Query<LimitQuery>,
) -> impl IntoResponse {
    if !principal.is_admin {
        return forbidden("admin required");
    }

    let limit = q.limit.unwrap_or(50).min(500);
    match sqlx::query(
        "SELECT * FROM governancepolicyevent ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => Json(rows.iter().map(row_to_event).collect::<Vec<_>>()).into_response(),
        Err(e) => {
            tracing::error!("list_events: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
