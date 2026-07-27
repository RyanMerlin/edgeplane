//! Shared domain-authorization guard for privileged dispatch/ledger/stream handlers.
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::auth::{authorized_for, authorized_for_owner, Principal};

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

/// Public-read-aware variant of [`authz_domain`], for READ paths that were
/// previously gated by a local mechanism-(2) check honoring domain
/// `visibility=='public'` (e.g. `tasks.rs::domain_access` with
/// `require_write=false, require_owner=false`). Grants if the domain is
/// public, OR [`authorized_for`] (admin / `domain_scope` / owners /
/// contributors). The public-visibility bypass is checked first.
///
/// The comparison is case-insensitive (`visibility` is never normalized or
/// constrained at write time — `routes/domains.rs::create_domain` binds the
/// caller-supplied string as-is, no CHECK constraint on the column) — this
/// matches the dominant convention used at five other read-path call sites
/// (`explorer.rs`, `artifacts.rs`, `missions.rs`, `docs.rs`, `domains.rs`,
/// all `.to_lowercase() == "public"`) and restores the exact behavior of the
/// `domain_access()` this replaces (`vis.to_lowercase() != "public"`).
/// `routes/search.rs` uses a case-sensitive `Some("public")` exact match —
/// that is the minority/outlier convention in this codebase (a pre-existing,
/// separately-tracked issue) and is deliberately NOT mirrored here.
///
/// Returns `Ok(())` on success, or an `Err(Response)` with the appropriate
/// status code: 422 on empty domain_id, 404 if absent, 403 if unauthorized,
/// 500 on DB error.
pub async fn authz_domain_readable(
    db: &PgPool,
    p: &Principal,
    domain_id: &str,
) -> Result<(), Response> {
    if domain_id.is_empty() {
        return Err(deny(
            StatusCode::UNPROCESSABLE_ENTITY,
            "target has no domain",
        ));
    }
    let row = sqlx::query("SELECT visibility, owners, contributors FROM domain WHERE id = $1")
        .bind(domain_id)
        .fetch_optional(db)
        .await
        .map_err(|e| {
            tracing::error!("authz_domain_readable load {domain_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
    let Some(row) = row else {
        return Err(deny(StatusCode::NOT_FOUND, "Domain not found"));
    };
    let visibility: Option<String> = row.get("visibility");
    if visibility.is_some_and(|v| v.eq_ignore_ascii_case("public")) {
        return Ok(());
    }
    let owners: String = row.get("owners");
    let contributors: String = row.get("contributors");
    if authorized_for(domain_id, &owners, &contributors, p) {
        Ok(())
    } else {
        Err(deny(StatusCode::FORBIDDEN, "not authorized for domain"))
    }
}

/// Owner-only variant of [`authz_domain`]: grants if admin, `domain_scope`
/// membership, or `owners` CSV membership — contributors do NOT count. See
/// [`crate::auth::authorized_for_owner`]. For actions stricter than a normal
/// domain write (e.g. `tasks.rs::delete_task`, migrated from
/// `domain_access(..., require_owner=true)`).
///
/// Returns `Ok(())` on success, or an `Err(Response)` with the appropriate
/// status code: 422 on empty domain_id, 404 if absent, 403 if unauthorized,
/// 500 on DB error.
pub async fn authz_domain_owner(
    db: &PgPool,
    p: &Principal,
    domain_id: &str,
) -> Result<(), Response> {
    if domain_id.is_empty() {
        return Err(deny(
            StatusCode::UNPROCESSABLE_ENTITY,
            "target has no domain",
        ));
    }
    let row = sqlx::query("SELECT owners FROM domain WHERE id = $1")
        .bind(domain_id)
        .fetch_optional(db)
        .await
        .map_err(|e| {
            tracing::error!("authz_domain_owner load {domain_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
    let Some(row) = row else {
        return Err(deny(StatusCode::NOT_FOUND, "Domain not found"));
    };
    let owners: String = row.get("owners");
    if authorized_for_owner(domain_id, &owners, p) {
        Ok(())
    } else {
        Err(deny(
            StatusCode::FORBIDDEN,
            "not authorized (owner-only) for domain",
        ))
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
        "SELECT domain_id FROM task WHERE id=$1",
        task_id,
        "Task not found",
    )
    .await
}

/// Resolve both `domain_id` and `kind` for a task in one round trip. Used by
/// callers (routes/mcp.rs's mesh-task tool handlers) that need to authorize
/// on domain AND reject claim/lease operations against a `kind='assigned'`
/// row — an assigned task is never claimable, leased, or heartbeat-able.
pub async fn domain_and_kind_for_task(
    db: &PgPool,
    task_id: &str,
) -> Result<(String, String), Response> {
    let row = sqlx::query("SELECT domain_id, kind FROM task WHERE id=$1")
        .bind(task_id)
        .fetch_optional(db)
        .await
        .map_err(|e| {
            tracing::error!("domain_and_kind_for_task {task_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
    let Some(row) = row else {
        return Err(deny(StatusCode::NOT_FOUND, "Task not found"));
    };
    Ok((row.get("domain_id"), row.get("kind")))
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

/// True iff `principal` IS the control-plane `agent` identified by `agent_id`
/// (i.e. the agent acting on itself). Agent JWTs carry `sub = "agent:{meshagent.id}"`,
/// so we bridge meshagent.id -> meshagent.agent_public_id and compare to the target
/// agent.public_id. Only `auth_type == "agent"` principals can be "self"; admin /
/// session / node are handled separately by callers. Fail-closed: any DB error,
/// missing row, or NULL bridge column returns false.
pub async fn is_self_control_plane_agent(db: &PgPool, p: &Principal, agent_id: i32) -> bool {
    if p.auth_type != "agent" {
        return false;
    }

    let mesh_id = p.subject.strip_prefix("agent:").unwrap_or(&p.subject);
    let agent_public_id: Option<Option<String>> =
        match sqlx::query_scalar("SELECT agent_public_id FROM meshagent WHERE id = $1")
            .bind(mesh_id)
            .fetch_optional(db)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("is_self_control_plane_agent meshagent {mesh_id}: {e}");
                return false;
            }
        };
    let Some(agent_public_id) = agent_public_id.flatten() else {
        return false;
    };

    let target_public_id: Option<String> =
        match sqlx::query_scalar("SELECT public_id FROM agent WHERE id = $1")
            .bind(agent_id)
            .fetch_optional(db)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("is_self_control_plane_agent agent {agent_id}: {e}");
                return false;
            }
        };

    target_public_id.as_deref() == Some(agent_public_id.as_str())
}

/// After domain authz: a non-full-trust caller may only act on a task it holds.
/// Full-trust (session/node) and admin bypass. `lease_id` is the caller-presented
/// claim_lease_id (None for endpoints that don't take one).
///
/// Kind-agnostic: works for both `kind='claimable'` rows (ownership via
/// `claimed_by_agent_id`, set by `claim_task`) and `kind='assigned'` rows
/// (ownership via `owner`, set at creation/update) — either kind's row is
/// also ownable by presenting the matching `claim_lease_id`, the completion
/// token minted for both kinds. See docs/plans (task/meshtask unification)
/// for why one primitive validates ownership the same way regardless of mode.
pub async fn authz_task_owner(
    db: &PgPool,
    p: &Principal,
    task_id: &str,
    lease_id: Option<&str>,
) -> Result<(), Response> {
    if crate::auth::is_full_trust(p) || p.is_admin {
        return Ok(());
    }
    let row = sqlx::query(
        "SELECT claimed_by_agent_id, claim_lease_id, owner FROM task WHERE id=$1",
    )
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
    let owner: Option<String> = row.get("owner");
    let subject_id = p.subject.strip_prefix("agent:").unwrap_or(&p.subject);
    let owns = claimed.as_deref() == Some(subject_id)
        || owner.as_deref() == Some(subject_id)
        || (lease_id.is_some() && lease.as_deref() == lease_id);
    if owns {
        Ok(())
    } else {
        Err(deny(StatusCode::FORBIDDEN, "not the task's claimer"))
    }
}

pub async fn domain_id_for_gate(db: &PgPool, gate_id: &str) -> Result<String, Response> {
    // reviewgate.mesh_task_id → task.domain_id
    // (schema: reviewgate FK column is `mesh_task_id`, not `task_id`)
    resolve(
        db,
        "SELECT t.domain_id FROM reviewgate g JOIN task t ON t.id = g.mesh_task_id WHERE g.id=$1",
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
// honor admin, per-node dynamic `domain_scope` (Workstream 2 — the domains a
// node currently hosts assigned agents in), per-agent `domain_scope`, and
// owners/contributors — so the daemon (node or owner credential) and scoped
// agents still pass while cross-domain non-members 403.
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
