//! Shared domain-authorization guard for privileged dispatch/ledger/stream handlers.
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::auth::{authorized_for, Principal};

fn deny(status: StatusCode, detail: &str) -> Response {
    (status, Json(json!({ "detail": detail }))).into_response()
}

/// Load `domain_id`'s owners/contributors and authorize `p`. Default deny.
///
/// Returns `Ok(())` on success, or an `Err(Response)` with the appropriate
/// status code: 422 on empty domain_id, 404 if absent, 403 if unauthorized,
/// 500 on DB error.
pub async fn authz_domain(db: &PgPool, p: &Principal, domain_id: &str) -> Result<(), Response> {
    if domain_id.is_empty() {
        return Err(deny(
            StatusCode::UNPROCESSABLE_ENTITY,
            "target has no domain",
        ));
    }
    let row = sqlx::query("SELECT owners, contributors FROM domain WHERE id = $1")
        .bind(domain_id)
        .fetch_optional(db)
        .await
        .map_err(|e| {
            tracing::error!("authz_domain load {domain_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
    let Some(row) = row else {
        return Err(deny(StatusCode::NOT_FOUND, "Domain not found"));
    };
    let owners: String = row.get("owners");
    let contributors: String = row.get("contributors");
    if authorized_for(domain_id, &owners, &contributors, p) {
        Ok(())
    } else {
        Err(deny(StatusCode::FORBIDDEN, "not authorized for domain"))
    }
}

async fn resolve(
    db: &PgPool,
    sql: &'static str,
    id: &str,
    missing: &'static str,
) -> Result<String, Response> {
    let v: Option<Option<String>> = sqlx::query_scalar(sql)
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(|e| {
            tracing::error!("resolver ({sql}) {id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
    v.flatten()
        .ok_or_else(|| deny(StatusCode::NOT_FOUND, missing))
}

pub async fn domain_id_for_mission(db: &PgPool, mission_id: &str) -> Result<String, Response> {
    resolve(
        db,
        "SELECT domain_id FROM mission WHERE id=$1",
        mission_id,
        "Mission not found",
    )
    .await
}

pub async fn domain_id_for_task(db: &PgPool, task_id: &str) -> Result<String, Response> {
    resolve(
        db,
        "SELECT domain_id FROM meshtask WHERE id=$1",
        task_id,
        "Task not found",
    )
    .await
}

pub async fn domain_id_for_agent(db: &PgPool, agent_id: &str) -> Result<String, Response> {
    resolve(
        db,
        "SELECT domain_id FROM meshagent WHERE id=$1",
        agent_id,
        "Agent not found",
    )
    .await
}

pub async fn domain_id_for_gate(db: &PgPool, gate_id: &str) -> Result<String, Response> {
    // reviewgate.mesh_task_id → meshtask.domain_id
    // (schema: reviewgate FK column is `mesh_task_id`, not `task_id`)
    resolve(
        db,
        "SELECT t.domain_id FROM reviewgate g JOIN meshtask t ON t.id = g.mesh_task_id WHERE g.id=$1",
        gate_id,
        "Gate not found",
    )
    .await
}
