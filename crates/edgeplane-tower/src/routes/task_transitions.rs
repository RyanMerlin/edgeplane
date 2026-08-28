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

/// After a fenced UPDATE's WHERE clause rejects a caller (zero rows
/// returned), classify why. Mirrors `claim_task`'s `conflict()`-on-`None`
/// pattern but adds the 403 split: a caller who presented no ownership
/// proof at all (no matching `claimed_by_agent_id`/`owner`, no lease
/// supplied) and isn't full-trust/admin gets 403; anyone who presented
/// *some* proof — a stale lease, a real-but-wrong-status claim — gets 409,
/// since from their perspective the request looked legitimate and lost a
/// race, not unauthorized access. See spec §1 "403 vs 409, done correctly".
///
/// This function is diagnostic-only: by the time it runs, the fenced
/// UPDATE has already atomically decided the request is rejected — nothing
/// here grants or withholds access, it only picks the response shape.
///
/// Deliberate: `lease_id.is_some()` alone is enough for 409, without
/// checking whether it matches the row's *current* `claim_lease_id`.
/// Dual-review (2026-08-19/20) flagged this as letting a caller suppress
/// the 403 signal by attaching any string. Verified the stricter
/// alternative (require an exact match) before rejecting it: it breaks
/// the reclaim-race case this design exists to serve —
/// `expire_stale_leases` clears a reclaimed row's `claim_lease_id` to
/// `NULL`, so a legitimate agent presenting its own, once-real, since-
/// reclaimed lease would get 403 instead of 409 under strict matching
/// (see `fencing_complete_stale_lease_after_reclaim_is_409` and
/// `fencing_heartbeat_stale_lease_is_409_not_403`, both of which encode
/// this exact scenario). `edgeplaned-work` maps 409 to a graceful
/// lease-mismatch/abandon path and 403 to a hard error, so this isn't
/// cosmetic — strict matching would hard-error a caller that did nothing
/// wrong. Abuse detection belongs on the `fenced_rejection` tracing event
/// below (which does distinguish a never-matching lease from a real one),
/// not on the HTTP status code.
///
/// `already_done_statuses`: statuses that mean *this specific transition*
/// already succeeded (e.g. `["finished"]` for `complete_task`) — checked
/// before ownership, unconditionally, for every caller. complete_task/
/// fail_task null `claimed_by_agent_id` on their terminal transition (to
/// close a real ownership-carryover gap — see the plan's "Correction:
/// terminal transitions don't fully clear ownership" section), which
/// otherwise turns a legitimate idempotent retry into a 403 instead of a
/// 409 once that evidence is gone (`edgeplaned-work` maps only 409 to a
/// graceful lease-mismatch path). A row already at the target status is a
/// state fact, not an authorization fact — true for the original owner
/// retrying AND for a caller that never had any relationship to the task,
/// so this check doesn't distinguish by identity at all; it doesn't leak
/// anything a domain-gated `GET /work/tasks/{id}` doesn't already reveal.
/// Empty slice for endpoints with no idempotent-retry concern (e.g.
/// `heartbeat_task`, whose target status isn't terminal).
///
/// Moved from `work.rs`'s `classify_fenced_rejection` — same behavior,
/// retyped to return `TransitionError` instead of an
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
    // finalized_by_subject: an earlier terminal/attribution-writing call
    // (complete/fail/cancel) can erase claimed_by_agent_id while preserving
    // the actor's identity in this separate column (see 3f8c262a/79d0c493).
    // A caller racing against that earlier call — e.g. self-cancel, then a
    // retried unblock — must not be misclassified as "zero ownership proof
    // ever" just because the specific column this predicate reads got
    // cleared by a DIFFERENT operation. Not attacker-controllable: this
    // column is only ever written to a legitimate claimer's or bypass
    // caller's own subject, so checking it here grants no new capability,
    // it just stops losing a signal that already exists on the row.
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
