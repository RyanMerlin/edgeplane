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

/// Resolve the owning domain of a **control-plane `agent` row** (the `agent`
/// table, keyed by its integer id), preferring `current_domain_id` over
/// `home_domain_id`. Distinct from [`domain_id_for_agent`], which resolves a
/// `meshagent` topology row — a different table and id space. Fails closed: 404
/// if the agent is absent or has no domain at all, 500 on DB error.
pub async fn domain_id_for_control_plane_agent(
    db: &PgPool,
    agent_id: i32,
) -> Result<String, Response> {
    let v: Option<Option<String>> = sqlx::query_scalar(
        "SELECT COALESCE(current_domain_id, home_domain_id) FROM agent WHERE id=$1",
    )
    .bind(agent_id)
    .fetch_optional(db)
    .await
    .map_err(|e| {
        tracing::error!("domain_id_for_control_plane_agent {agent_id}: {e}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;
    v.flatten()
        .ok_or_else(|| deny(StatusCode::NOT_FOUND, "Agent not found"))
}

/// After domain authz: a non-full-trust caller may only act on a task it holds.
/// Full-trust (session/node) and admin bypass. `lease_id` is the caller-presented
/// claim_lease_id (None for endpoints that don't take one).
pub async fn authz_task_owner(
    db: &PgPool,
    p: &Principal,
    task_id: &str,
    lease_id: Option<&str>,
) -> Result<(), Response> {
    if crate::auth::is_full_trust(p) || p.is_admin {
        return Ok(());
    }
    let row = sqlx::query("SELECT claimed_by_agent_id, claim_lease_id FROM meshtask WHERE id=$1")
        .bind(task_id)
        .fetch_optional(db)
        .await
        .map_err(|e| {
            tracing::error!("authz_task_owner {task_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
    let Some(row) = row else {
        return Err(deny(StatusCode::NOT_FOUND, "Task not found"));
    };
    let claimed: Option<String> = row.get("claimed_by_agent_id");
    let lease: Option<String> = row.get("claim_lease_id");
    let subject_id = p.subject.strip_prefix("agent:").unwrap_or(&p.subject);
    let owns = claimed.as_deref() == Some(subject_id)
        || (lease_id.is_some() && lease.as_deref() == lease_id);
    if owns {
        Ok(())
    } else {
        Err(deny(StatusCode::FORBIDDEN, "not the task's claimer"))
    }
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

// ── Gate-only combinators ──────────────────────────────────────────────────────
// Resolve the owning domain from a path object, then run the shared default-deny
// `authz_domain`. For the many `/work/...` read handlers that receive only a
// `task_id` / `mission_id` / `agent_id` and do not reuse the resolved domain
// afterward. Because they route through `authz_domain` → `authorized_for`, they
// honor admin, node blanket-trust (until Workstream 2 scopes it), per-agent
// `domain_scope`, and owners/contributors — so the daemon (node or owner
// credential) and scoped agents still pass while cross-domain non-members 403.
// Do NOT hand-roll a local owners/contributors check here: mechanism-(2) style
// checks ignore `domain_scope`/`auth_type` and would lock out the daemon and
// scoped agents (see docs/plans/2026-07-10-authz-hardening.md § O2).

/// Resolve a task's domain and authorize `p` for it.
pub async fn authz_by_task(db: &PgPool, p: &Principal, task_id: &str) -> Result<(), Response> {
    let domain_id = domain_id_for_task(db, task_id).await?;
    authz_domain(db, p, &domain_id).await
}

/// Resolve a mission's domain and authorize `p`. See [`authz_by_task`].
pub async fn authz_by_mission(
    db: &PgPool,
    p: &Principal,
    mission_id: &str,
) -> Result<(), Response> {
    let domain_id = domain_id_for_mission(db, mission_id).await?;
    authz_domain(db, p, &domain_id).await
}

/// Resolve an agent's domain and authorize `p`. See [`authz_by_task`].
pub async fn authz_by_agent(db: &PgPool, p: &Principal, agent_id: &str) -> Result<(), Response> {
    let domain_id = domain_id_for_agent(db, agent_id).await?;
    authz_domain(db, p, &domain_id).await
}

/// Resolve a control-plane `agent` row's domain (integer id) and authorize `p`.
/// Use for `routes/agents.rs` handlers, which key off the `agent` table — NOT
/// [`authz_by_agent`], which targets the separate `meshagent` topology table.
pub async fn authz_by_control_plane_agent(
    db: &PgPool,
    p: &Principal,
    agent_id: i32,
) -> Result<(), Response> {
    let domain_id = domain_id_for_control_plane_agent(db, agent_id).await?;
    authz_domain(db, p, &domain_id).await
}
