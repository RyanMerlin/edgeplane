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
use chrono::{Duration, Utc};
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


/// Family A fence: live claimable-lease ownership+freshness check, shared by
/// Heartbeat and AppendProgress. Takes the row lock (`FOR UPDATE`) so the
/// caller's subsequent write — whether to `task` itself (Heartbeat) or a
/// different table (AppendProgress → `meshprogressevent`) — is atomic with
/// respect to any concurrent writer of this same task row. Returns the
/// locked row on success (unused by callers today beyond existence, but
/// available for future fence families that need to read fields off it
/// without a second round-trip).
///
/// Security invariant — do not change this predicate's parenthesization:
/// `(claim_lease_id = $2 OR $3) AND (claim_policy = 'broadcast' OR
/// lease_expires_at >= $4)`. Ownership (a matching `claim_lease_id`, or the
/// bypass/full-trust escape hatch) is required UNCONDITIONALLY, for every
/// caller, regardless of `claim_policy`; `claim_policy = 'broadcast'` waives
/// ONLY the freshness sub-check. The first version of this predicate (before
/// it was unified into this single shared helper) had `claim_policy =
/// 'broadcast'` as a bare top-level `OR` disjunct spanning the ENTIRE
/// ownership+lease clause — a CRITICAL bug, fixed in commit 37dca61a, that
/// let any caller who merely knew a broadcast task's id bypass ownership
/// entirely, not just staleness. That same bug shape was independently
/// reintroduced once more in an early draft of `append_progress`'s own copy
/// of this predicate and caught by a second review pass before merge — it
/// keeps recurring under refactor pressure whenever the predicate is
/// hand-derived per call site, which is exactly why it now lives in this one
/// named function instead.
async fn fence_claimable_live(
    tx: &mut sqlx::PgConnection,
    task_id: &str,
    lease_id: Option<&str>,
    is_bypass: bool,
    now: chrono::NaiveDateTime,
) -> Result<Option<sqlx::postgres::PgRow>, sqlx::Error> {
    sqlx::query(
        "SELECT * FROM task WHERE id=$1 AND kind='claimable' AND status IN ('claimed','running') \
         AND (claim_lease_id = $2 OR $3) \
         AND (claim_policy = 'broadcast' OR lease_expires_at >= $4) \
         FOR UPDATE",
    )
    .bind(task_id)
    .bind(lease_id)
    .bind(is_bypass)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await
}

fn row_to_progress(row: &sqlx::postgres::PgRow) -> serde_json::Value {
    serde_json::json!({
        "id": row.get::<i32, _>("id"),
        "task_id": row.get::<String, _>("task_id"),
        "agent_id": row.get::<String, _>("agent_id"),
        "seq": row.get::<i32, _>("seq"),
        "event_type": row.get::<String, _>("event_type"),
        "phase": row.get::<Option<String>, _>("phase"),
        "step": row.get::<Option<String>, _>("step"),
        "summary": row.get::<String, _>("summary"),
        "payload_json": serde_json::from_str::<serde_json::Value>(row.get::<&str, _>("payload_json")).unwrap_or(serde_json::json!({})),
        "occurred_at": row.get::<chrono::NaiveDateTime, _>("occurred_at"),
        "agent_run_id": row.get::<Option<String>, _>("agent_run_id"),
    })
}

pub enum TaskTransition<'a> {
    Heartbeat {
        claim_lease_id: Option<&'a str>,
    },
    AppendProgress {
        claim_lease_id: &'a str,
        event_type: &'a str,
        phase: Option<&'a str>,
        step: Option<&'a str>,
        summary: &'a str,
        payload_json: &'a str,
        agent_run_id: Option<&'a str>,
    },
    Complete {
        claim_lease_id: Option<&'a str>,
        agent_id: Option<&'a str>,
        result_artifact_id: Option<i32>,
    },
}

pub enum TransitionOutcome {
    Task {
        task: serde_json::Value,
        unblocked_task_ids: Vec<String>,
    },
    Progress(serde_json::Value),
    WaitingReview {
        task: serde_json::Value,
        pending_gate_ids: Vec<String>,
    },
}

pub(crate) async fn execute_task_transition(
    db: &sqlx::PgPool,
    actor: &TransitionActor<'_>,
    task_id: &str,
    transition: TaskTransition<'_>,
) -> Result<TransitionOutcome, TransitionError> {
    match transition {
        TaskTransition::Heartbeat { claim_lease_id } => {
            let now = Utc::now().naive_utc();
            let mut tx = db.begin().await.map_err(|e| TransitionError::Database {
                operation: "heartbeat begin tx",
                source: e,
            })?;
            let locked = fence_claimable_live(&mut tx, task_id, claim_lease_id, actor.is_bypass, now)
                .await
                .map_err(|e| TransitionError::Database {
                    operation: "heartbeat fence",
                    source: e,
                })?;
            if locked.is_none() {
                let _ = tx.rollback().await;
                return Err(classify_fenced_rejection(db, actor, task_id, claim_lease_id, &[]).await);
            }
            let lease_expires = now + Duration::seconds(crate::routes::work::LEASE_TTL_SECS);
            let row = sqlx::query(
                "UPDATE task SET status='running', lease_expires_at=$2, updated_at=$3 \
                 WHERE id=$1 RETURNING *",
            )
            .bind(task_id)
            .bind(lease_expires)
            .bind(now)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| TransitionError::Database {
                operation: "heartbeat update",
                source: e,
            })?;
            tx.commit().await.map_err(|e| TransitionError::Database {
                operation: "heartbeat commit",
                source: e,
            })?;
            Ok(TransitionOutcome::Task {
                task: crate::routes::work::row_to_task(&row),
                unblocked_task_ids: vec![],
            })
        }
        TaskTransition::AppendProgress {
            claim_lease_id,
            event_type,
            phase,
            step,
            summary,
            payload_json,
            agent_run_id,
        } => {
            let now = Utc::now().naive_utc();
            let mut tx = db.begin().await.map_err(|e| TransitionError::Database {
                operation: "progress begin tx",
                source: e,
            })?;
            let locked =
                fence_claimable_live(&mut tx, task_id, Some(claim_lease_id), actor.is_bypass, now)
                    .await
                    .map_err(|e| TransitionError::Database {
                        operation: "progress fence",
                        source: e,
                    })?;
            if locked.is_none() {
                let _ = tx.rollback().await;
                return Err(
                    classify_fenced_rejection(db, actor, task_id, Some(claim_lease_id), &[]).await,
                );
            }
            // Issued as its own statement, AFTER the row lock above is held —
            // under READ COMMITTED this gets a fresh snapshot as of *now*,
            // not the transaction's start, so a concurrent poster that was
            // blocked on the same lock and just committed is visible here.
            // This is what closes the seq-duplication race a single-
            // statement CTE version of this fence could not (see Global
            // Constraints) — closed for the REST path only. MCP's
            // `progress_mesh_task` (mcp.rs) still writes via the raw pool
            // with no lock and remains unlocked until Task 6 migrates it
            // onto this same `execute_task_transition` service; until then a
            // REST post and an MCP post to the same task can still collide.
            let seq: i32 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq), -1) + 1 FROM meshprogressevent WHERE task_id=$1",
            )
            .bind(task_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| TransitionError::Database {
                operation: "progress seq",
                source: e,
            })?;
            let agent_id = actor.subject_id.to_string();
            let row = sqlx::query(
                "INSERT INTO meshprogressevent \
                 (task_id, agent_id, seq, event_type, phase, step, summary, payload_json, occurred_at, agent_run_id) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING *",
            )
            .bind(task_id)
            .bind(&agent_id)
            .bind(seq)
            .bind(event_type)
            .bind(phase)
            .bind(step)
            .bind(summary)
            .bind(payload_json)
            .bind(now)
            .bind(agent_run_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| TransitionError::Database {
                operation: "progress insert",
                source: e,
            })?;
            tx.commit().await.map_err(|e| TransitionError::Database {
                operation: "progress commit",
                source: e,
            })?;
            Ok(TransitionOutcome::Progress(row_to_progress(&row)))
        }
        TaskTransition::Complete {
            claim_lease_id,
            agent_id,
            result_artifact_id,
        } => {
            // See work.rs's original comment (still true here): the real
            // edgeplaned-bin/task_worker.rs caller authenticates as a
            // full-trust node and always sends {"agent_id": ...}; its own
            // subject can never match claimed_by_agent_id, so ownership is
            // read back the same on-behalf-of way claim_task wrote it,
            // bypass-gated so a restricted caller can't spoof another
            // agent's id via this field (Ruling C2).
            let effective_id = if actor.is_bypass {
                agent_id.unwrap_or(actor.subject_id)
            } else {
                actor.subject_id
            };
            let now = Utc::now().naive_utc();
            let now_tz = Utc::now();
            let row = sqlx::query(
                "WITH gate_check AS ( \
                   SELECT EXISTS ( \
                     SELECT 1 FROM reviewgate WHERE mesh_task_id=$1 AND status='pending' \
                   ) AS has_pending \
                 ) \
                 UPDATE task SET \
                   status = CASE WHEN gate_check.has_pending THEN 'waiting_review' ELSE 'finished' END, \
                   result_artifact_id = CASE WHEN gate_check.has_pending THEN task.result_artifact_id ELSE $2 END, \
                   lease_expires_at = CASE WHEN gate_check.has_pending THEN task.lease_expires_at ELSE NULL END, \
                   claim_lease_id = CASE WHEN gate_check.has_pending THEN task.claim_lease_id ELSE NULL END, \
                   claimed_by_agent_id = CASE WHEN gate_check.has_pending THEN task.claimed_by_agent_id ELSE NULL END, \
                   finalized_at = CASE WHEN gate_check.has_pending THEN task.finalized_at ELSE $3 END, \
                   finalized_by_subject = CASE WHEN gate_check.has_pending THEN task.finalized_by_subject \
                                                ELSE COALESCE(task.claimed_by_agent_id, $7) END, \
                   updated_at = $4 \
                 FROM gate_check \
                 WHERE task.id = $1 \
                   AND ( \
                     (task.kind = 'claimable' AND task.status IN ('claimed','running','waiting_review') \
                      AND (task.claimed_by_agent_id = $7 \
                           OR ((task.claim_lease_id = $5 OR $6) \
                               AND (task.claim_policy = 'broadcast' OR task.lease_expires_at >= $4)))) \
                     OR \
                     (task.kind = 'assigned' AND task.status NOT IN ('done','finished','failed','cancelled') \
                      AND (task.owner = $7 OR task.claim_lease_id = $5 OR $6)) \
                   ) \
                 RETURNING task.*, gate_check.has_pending",
            )
            .bind(task_id)
            .bind(result_artifact_id)
            .bind(now_tz)
            .bind(now)
            .bind(claim_lease_id)
            .bind(actor.is_bypass)
            .bind(effective_id)
            .fetch_optional(db)
            .await
            .map_err(|e| TransitionError::Database {
                operation: "complete update",
                source: e,
            })?;

            let Some(r) = row else {
                return Err(classify_fenced_rejection(db, actor, task_id, claim_lease_id, &["finished"]).await);
            };

            let has_pending: bool = r.get("has_pending");
            if has_pending {
                let gate_ids: Vec<String> = sqlx::query_scalar(
                    "SELECT id FROM reviewgate WHERE mesh_task_id=$1 AND status='pending'",
                )
                .bind(task_id)
                .fetch_all(db)
                .await
                .unwrap_or_default();
                return Ok(TransitionOutcome::WaitingReview {
                    task: crate::routes::work::row_to_task(&r),
                    pending_gate_ids: gate_ids,
                });
            }

            let mission_id: String = r.get("mission_id");
            let domain_id: String = r.get("domain_id");
            let unblocked = crate::routes::work::unblock_dependents(db, &mission_id, task_id).await;
            for tid in &unblocked {
                crate::routes::work::broadcast_task_available(&domain_id, &mission_id, tid).await;
            }
            Ok(TransitionOutcome::Task {
                task: crate::routes::work::row_to_task(&r),
                unblocked_task_ids: unblocked,
            })
        }
    }
}
