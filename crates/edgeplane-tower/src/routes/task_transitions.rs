//! Shared fenced-transition primitive for heartbeat/complete/fail/block/
//! progress — the five task-lifecycle mutations with both a REST
//! (`routes/work.rs`) and an MCP (`routes/mcp.rs`) surface. Both call into
//! this module instead of each hand-deriving their own copy of the same
//! fence predicate — see `docs/superpowers/specs/
//! 2026-08-28-shared-fenced-transition-primitive-design.md` for why.

use crate::auth::Principal;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sqlx::Row;

/// Derived once per request from the authenticated `Principal`. `subject_id`
/// is the `agent:`-prefix-stripped form every fence predicate compares
/// against; `is_bypass` is the full-trust/admin escape hatch every fenced
/// predicate's `OR $bypass` arm reads.
pub struct TransitionActor<'a> {
    pub subject: &'a str,
    pub subject_id: &'a str,
    pub is_bypass: bool,
    pub is_admin: bool,
}

pub fn task_actor(principal: &Principal) -> TransitionActor<'_> {
    TransitionActor {
        subject: &principal.subject,
        subject_id: principal
            .subject
            .strip_prefix("agent:")
            .unwrap_or(&principal.subject),
        is_bypass: crate::auth::is_full_trust(principal) || principal.is_admin,
        is_admin: principal.is_admin,
    }
}

#[derive(Debug)]
pub enum TransitionError {
    NotFound,
    Forbidden,
    Conflict,
    Invalid(String),
    Database {
        operation: &'static str,
        source: sqlx::Error,
    },
}

/// After a fenced write rejects a caller (zero rows returned), classify why.
/// Moved verbatim from `work.rs`'s `classify_fenced_rejection` — same
/// behavior, retyped to return `TransitionError` instead of an
/// `axum::response::Response` so MCP callers (which have no use for an Axum
/// response type) can use it too. `rest_transition_error` is the REST-side
/// adapter back to the exact status codes/bodies this function used to
/// build directly.
pub(crate) async fn classify_fenced_rejection(
    db: &sqlx::PgPool,
    actor: &TransitionActor<'_>,
    task_id: &str,
    lease_id: Option<&str>,
    already_done_statuses: &[&str],
) -> TransitionError {
    let row = match sqlx::query(
        "SELECT claimed_by_agent_id, owner, claim_lease_id, status, finalized_by_subject \
         FROM task WHERE id=$1",
    )
    .bind(task_id)
    .fetch_optional(db)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return TransitionError::NotFound,
        Err(e) => {
            tracing::error!("classify_fenced_rejection fetch: {e}");
            return TransitionError::Database {
                operation: "classify_fenced_rejection fetch",
                source: e,
            };
        }
    };
    let status: String = row.get("status");
    let already_done = already_done_statuses.contains(&status.as_str());
    if already_done || actor.is_bypass {
        return TransitionError::Conflict;
    }
    let claimed: Option<String> = row.get("claimed_by_agent_id");
    let owner: Option<String> = row.get("owner");
    let current_lease: Option<String> = row.get("claim_lease_id");
    let finalized_by: Option<String> = row.get("finalized_by_subject");
    let owns_directly = claimed.as_deref() == Some(actor.subject_id)
        || owner.as_deref() == Some(actor.subject_id)
        || finalized_by.as_deref() == Some(actor.subject_id);
    let lease_matches_current = lease_id.is_some() && lease_id == current_lease.as_deref();
    tracing::warn!(
        %task_id,
        subject = %actor.subject,
        lease_presented = lease_id.is_some(),
        lease_matches_current,
        owns_directly,
        already_done,
        "fenced_rejection"
    );
    if owns_directly || lease_id.is_some() {
        TransitionError::Conflict
    } else {
        TransitionError::Forbidden
    }
}

/// REST adapter: converts a `TransitionError` into the exact
/// `axum::response::Response` shapes `work.rs`'s handlers built directly
/// before this refactor — status codes and body shapes are unchanged.
pub(crate) fn rest_transition_error(error: TransitionError) -> Response {
    match error {
        TransitionError::NotFound => crate::routes::work::not_found("Task not found"),
        TransitionError::Forbidden => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "not the task's claimer"})),
        )
            .into_response(),
        TransitionError::Conflict => {
            crate::routes::work::conflict("Task is not in the required state for this transition")
        }
        TransitionError::Invalid(detail) => crate::routes::work::bad_request(&detail),
        TransitionError::Database { operation, source } => {
            tracing::error!("{operation}: {source}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
