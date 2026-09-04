# Shared Fenced-Transition Primitive Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the REST/MCP fencing duplication that caused a CRITICAL regression (commit `82507f12`) and a HIGH-severity MCP lease-revival gap, by giving `heartbeat`/`complete`/`fail`/`block`/`progress` one shared, atomically-fenced Rust implementation that both `routes/work.rs` (REST) and `routes/mcp.rs` (MCP) call into.

**Architecture:** A new `crates/edgeplane-tower/src/routes/task_transitions.rs` module owns the fence predicates (as visible, per-family SQL — not a macro), the mutating statements, and rejection classification, returning a typed `Result<TransitionOutcome, TransitionError>`. REST and MCP become thin adapters: REST's five existing hand-rolled handlers are refactored to call the service (regression risk mitigated by the existing ~300-test suite staying green throughout); MCP's five arms — currently either missing freshness checks entirely (`heartbeat_mesh_task`, `progress_mesh_task`) or check-then-act with no fencing at all (`complete_mesh_task`/`fail_mesh_task`/`block_mesh_task`) — get real fencing for the first time.

**Tech Stack:** Rust (edition 2024), axum, sqlx (raw `query`, not `query!`), Postgres, `axum_test::TestServer` integration tests gated on `TEST_DATABASE_URL`.

**Spec:** `docs/superpowers/specs/2026-08-28-shared-fenced-transition-primitive-design.md`

## Global Constraints

- **No macro, no fully-generic function.** Each fence family (A: live claimable lease — heartbeat/progress; B: claimable-or-assigned lifecycle — complete/fail; D: claimable pause — block) is its own small, visible helper. This is deliberate per the spec: a predicate hidden behind generic machinery is exactly what let the CRITICAL bug hide from review once already.
- **`TransitionOutcome`'s task/progress payloads are `serde_json::Value`, produced by (refactored, reused) `row_to_task`/a new `row_to_progress`helper — not new typed `TaskRecord`/`ProgressRecord` structs.** The spec left this open; `serde_json::Value` matches the established pattern this entire codebase already uses everywhere else and avoids a parallel typed-DTO layer for no behavioral benefit. If you're implementing from an earlier read of the spec that shows `TaskRecord`, that was explicitly flagged there as an implementation-plan-level decision — this plan's resolution is `serde_json::Value`.
- **Family A (heartbeat, progress) uses an explicit `db.begin()` transaction: `SELECT ... FOR UPDATE` first (taking the row lock), then the actual mutation as a separate statement in the same transaction, then commit.** Not a single CTE-shaped statement. This is required, not stylistic: `append_progress`'s current single-statement CTE (`WITH eligible AS (SELECT ...) INSERT ... WHERE EXISTS (...)`) has an independently-confirmed cross-table TOCTOU (a live 3-connection reproduction by one review, matching SQL-semantics reasoning by a second, independent review) — a concurrent write to `task` that commits after the CTE's snapshot is taken but before the INSERT completes isn't observed, because the CTE never locks or writes the row it reads. Splitting into `SELECT ... FOR UPDATE` (statement 1) then the mutation (statement 2) closes this: under READ COMMITTED, each *statement* within an explicit transaction gets its own fresh snapshot at that statement's own start, so a caller that was blocked on the row lock and then proceeds sees data committed by whoever it was waiting on. This same technique, applied to `progress`'s `SELECT COALESCE(MAX(seq),-1)+1` as its own statement issued *after* the lock is held, also closes the previously-open `seq` duplication race (documented in the plan's roadmap as "considered, not adopted" for a single-statement CTE shape — that conclusion doesn't carry over to this multi-statement-transaction shape; verify this understanding with a live concurrent test in Task 2, don't just trust this paragraph).
- **Family B (complete, fail) keeps the existing single-statement `UPDATE task ... WHERE <fence> RETURNING *` shape** (with `complete`'s `gate_check` CTE folded into the same statement, matching current code) — this already writes directly to the row it fences, so it already gets Postgres's EvalPlanQual re-check for free (this is *why* `heartbeat_task`'s existing single-statement shape was never flagged as vulnerable the way `append_progress`'s was — the vulnerable case is specifically "fence reads one table, write lands in a different table without a lock").
- **Family B's on-behalf-of `effective_id` derivation (Ruling C2) is preserved exactly**: `if is_bypass { agent_id.unwrap_or(subject_id) } else { subject_id }`. This exists because the real, deployed `edgeplaned-bin/task_worker.rs` caller authenticates as a full-trust node and never carries a lease — removing this path breaks that caller. Do not "clean this up."
- **`classify_fenced_rejection` moves into `task_transitions.rs` and returns `TransitionError` instead of `axum::response::Response`.** `cancel_task` and `unblock_task` (out of this plan's 5-transition scope, but existing callers of the old function) get updated to call the new typed version through the same `rest_transition_error` adapter every migrated handler uses — this keeps exactly one classifier in the codebase instead of two. Their own fence predicates are untouched.
- **`heartbeat_mesh_task`'s lease TTL unifies to `LEASE_TTL_SECS` (120s), not its current 300s.** The original EP-1 plan's own roadmap flagged this exact divergence as an unresolved decision ("heartbeat_mesh_task's lease TTL is 300s against REST's LEASE_TTL_SECS = 120s — the two paths can put the same row in different freshness states depending on which one last touched it; needs a decision... not left as an unstated inconsistency"). Routing MCP's heartbeat through the same `execute_task_transition` Heartbeat arm REST uses resolves it: one constant, one TTL, both surfaces. This is a deliberate, tested behavior change for MCP callers (Task 6 verifies it explicitly) — not a silent side effect of the refactor.
- Every task ends with `cargo clippy --workspace --all-targets -- -D warnings` passing and the full `edgeplane-tower` suite green except the one known pre-existing, unrelated failure: `test_global_sse_null_summary_decode::meshprogressevent_summary_decodes_when_null`.
- Test command: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E '<filter>'` — a local Postgres matching this shape must be reachable before running (an ephemeral `edgeplane-test-pg` container has been kept running for this branch's whole history; verify with `docker ps` before assuming you need to start one).
- All new/modified SQL uses `sqlx::query` + `.bind()` + `Row::get` — never `sqlx::query!`.

---

## File Structure

- Create: `crates/edgeplane-tower/src/routes/task_transitions.rs` — `TransitionActor`, `TaskTransition`, `TransitionOutcome`, `TransitionError`, `execute_task_transition`, `classify_fenced_rejection` (moved from `work.rs`), `fence_claimable_live`, `row_to_progress`, `rest_transition_error`.
- Modify: `crates/edgeplane-tower/src/routes/mod.rs` — register the new module.
- Modify: `crates/edgeplane-tower/src/routes/work.rs` — `LEASE_TTL_SECS` and `unblock_dependents` become `pub(crate)`; `classify_fenced_rejection` removed (moved out); `cancel_task`/`unblock_task` updated to call the moved+retyped classifier; `heartbeat_task`/`append_progress`/`complete_task`/`fail_task`/`block_task` refactored to call `execute_task_transition`.
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs` — `heartbeat_mesh_task`/`progress_mesh_task`/the combined `complete_mesh_task | fail_mesh_task | block_mesh_task` arm refactored to call `execute_task_transition`.
- Modify: `crates/edgeplane-tower/tests/test_mcp_progress_mesh_task.rs` — the existing `progress_mesh_task_inserts_sequential_events` test seeds an unclaimed `'ready'` task with no lease; once fenced, this no longer qualifies (`status IN ('claimed','running')` required) — update the seed to a claimed/running task with a live lease, and thread the lease through each of the 3 calls.
- Create: `crates/edgeplane-tower/tests/test_mcp_fenced_transitions.rs` — new MCP-side fencing tests for all five transitions.

---

### Task 1: Core types, moved+retyped classifier, module wiring

**Files:**
- Create: `crates/edgeplane-tower/src/routes/task_transitions.rs`
- Modify: `crates/edgeplane-tower/src/routes/mod.rs`
- Modify: `crates/edgeplane-tower/src/routes/work.rs:590` (`LEASE_TTL_SECS` → `pub(crate)`), `work.rs:213-277` (remove `classify_fenced_rejection`, it moves), `work.rs:692` (`unblock_dependents` → `pub(crate)`), `cancel_task`'s and `unblock_task`'s `classify_fenced_rejection` call sites
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs` (existing `fencing_cancel_*`/`fencing_unblock_*` tests must still pass unchanged — this task must not alter their observable behavior)

**Interfaces:**
- Produces: `pub struct TransitionActor<'a> { pub subject: &'a str, pub subject_id: &'a str, pub is_bypass: bool, pub is_admin: bool }`, `pub fn task_actor(principal: &Principal) -> TransitionActor<'_>`, `pub enum TransitionError { NotFound, Forbidden, Conflict, Invalid(String), Database { operation: &'static str, source: sqlx::Error } }`, `pub(crate) async fn classify_fenced_rejection(db: &sqlx::PgPool, actor: &TransitionActor<'_>, task_id: &str, lease_id: Option<&str>, already_done_statuses: &[&str]) -> TransitionError`, `pub(crate) fn rest_transition_error(error: TransitionError) -> axum::response::Response`.
- Consumes: `crate::auth::{Principal, is_full_trust}` (existing), `crate::routes::work::{not_found, conflict}` (existing helpers, stay in `work.rs` — `rest_transition_error` calls them via `crate::routes::work::`).

- [ ] **Step 1: Write the failing test proving `classify_fenced_rejection`'s behavior is unchanged after the move**

Add to `test_task_kind_unification.rs`, right after the existing `fencing_cancel_*` tests (the exact insertion point doesn't matter — this is a pure regression guard, not new behavior):

```rust
#[tokio::test]
async fn fencing_classify_rejection_survives_the_move_to_task_transitions() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    // A restricted, unrelated caller with zero ownership proof on a running
    // task must still get 403 — the exact classify_fenced_rejection behavior
    // this task moves into task_transitions.rs, unchanged.
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-someone-else"),
        1,
    )
    .await;
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/cancel"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cancel_task's classify_fenced_rejection call, now routed through \
         task_transitions::classify_fenced_rejection + rest_transition_error, \
         must still classify zero-proof restricted callers as 403: {}",
        res.text()
    );
}
```

- [ ] **Step 2: Run to verify it currently passes (regression guard, not new behavior)**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_classify_rejection_survives_the_move_to_task_transitions)'`
Expected: PASS against the current, pre-move code — this confirms the baseline before you touch anything.

- [ ] **Step 3: Create `task_transitions.rs` with the core types and the moved, retyped classifier**

Create `crates/edgeplane-tower/src/routes/task_transitions.rs`:

```rust
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
```

- [ ] **Step 4: Confirm `not_found`/`conflict`/`bad_request` are visible to the new module**

Read `work.rs` around its `not_found`/`conflict`/`bad_request` helper definitions (search `fn not_found`, `fn conflict`, `fn bad_request`). If any is not already `pub(crate)`, add `pub(crate)` to its signature — `task_transitions.rs` calls them via `crate::routes::work::`.

- [ ] **Step 5: Remove the old `classify_fenced_rejection` from `work.rs`, delete its now-orphaned doc comment**

In `work.rs`, delete the entire `classify_fenced_rejection` function (currently lines 213-277 — re-locate by searching `async fn classify_fenced_rejection` first, since earlier tasks this session have shifted line numbers repeatedly; delete the whole function body and its preceding doc comment block).

- [ ] **Step 6: Update `cancel_task` and `unblock_task` to call the moved, retyped classifier**

In `work.rs`, find `cancel_task`'s call site (search `classify_fenced_rejection(&state.db, &principal, &task_id, None, &["cancelled"])`). Replace:

```rust
        Ok(None) => classify_fenced_rejection(&state.db, &principal, &task_id, None, &["cancelled"]).await,
```

with:

```rust
        Ok(None) => {
            let actor = crate::routes::task_transitions::task_actor(&principal);
            crate::routes::task_transitions::rest_transition_error(
                crate::routes::task_transitions::classify_fenced_rejection(
                    &state.db, &actor, &task_id, None, &["cancelled"],
                )
                .await,
            )
        }
```

Find `unblock_task`'s call site (search `classify_fenced_rejection(&state.db, &principal, &task_id, None, &[]).await`) and apply the identical transformation, keeping its own `already_done_statuses` argument (`&[]`) unchanged.

- [ ] **Step 7: Make `LEASE_TTL_SECS` and `unblock_dependents` visible to the new module**

In `work.rs`, change `const LEASE_TTL_SECS: i64 = 120;` to `pub(crate) const LEASE_TTL_SECS: i64 = 120;`.
Change `async fn unblock_dependents(` to `pub(crate) async fn unblock_dependents(`.

- [ ] **Step 8: Register the new module**

In `crates/edgeplane-tower/src/routes/mod.rs`, add `pub mod task_transitions;` — insert it alphabetically among the existing `pub mod` lines (after `pub mod slack_integrations;`, before whatever comes next alphabetically — check the file's actual ordering and match it).

- [ ] **Step 9: Run the regression test and the full existing suite**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_classify_rejection_survives_the_move_to_task_transitions)'`
Expected: PASS.

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml --no-fail-fast`
Expected: same pass count as before this task (one known pre-existing failure, everything else green) — this task changes zero observable behavior, it only moves and retypes code.

- [ ] **Step 10: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add crates/edgeplane-tower/src/routes/task_transitions.rs \
        crates/edgeplane-tower/src/routes/mod.rs \
        crates/edgeplane-tower/src/routes/work.rs \
        crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "refactor(tower): move classify_fenced_rejection into task_transitions, retype to TransitionError"
```

---

### Task 2: Family A — `fence_claimable_live` + Heartbeat + AppendProgress, REST wiring

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/task_transitions.rs` (add `fence_claimable_live`, `row_to_progress`, `TaskTransition`/`TransitionOutcome` enums, `execute_task_transition` with `Heartbeat`/`AppendProgress` arms)
- Modify: `crates/edgeplane-tower/src/routes/work.rs` — `heartbeat_task` (search `async fn heartbeat_task`) and `append_progress` (search `async fn append_progress`) refactored to call the service
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Produces: `pub enum TaskTransition<'a> { Heartbeat { claim_lease_id: Option<&'a str> }, AppendProgress { claim_lease_id: &'a str, event_type: &'a str, phase: Option<&'a str>, step: Option<&'a str>, summary: &'a str, payload_json: &'a str, agent_run_id: Option<&'a str> } }` (more variants added in Tasks 3-5), `pub enum TransitionOutcome { Task { task: serde_json::Value, unblocked_task_ids: Vec<String> }, Progress(serde_json::Value) }` (more variants added in Task 3), `pub(crate) async fn execute_task_transition(db: &sqlx::PgPool, actor: &TransitionActor<'_>, task_id: &str, transition: TaskTransition<'_>) -> Result<TransitionOutcome, TransitionError>`.
- Consumes: `crate::routes::work::{row_to_task, LEASE_TTL_SECS}` (the latter now `pub(crate)` per Task 1), `TransitionActor`/`TransitionError`/`classify_fenced_rejection` (Task 1).

- [ ] **Step 1: Write the failing tests — behavioral parity for heartbeat and progress, plus the seq-race closure**

Add to `test_task_kind_unification.rs`:

```rust
#[tokio::test]
async fn fencing_heartbeat_still_works_after_family_a_refactor() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-a', lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "heartbeat must still succeed after the family-A refactor: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "running");
}

/// The specific race this task's row-locked, multi-statement transaction
/// design closes: two genuinely concurrent progress posts to the SAME task
/// must never both compute the same `seq` value. A single-statement CTE
/// version of this fence (the pre-refactor code) could not guarantee this —
/// see this plan's Global Constraints for why a multi-statement transaction
/// does.
#[tokio::test]
async fn fencing_progress_concurrent_posts_get_sequential_seq_after_family_a_refactor() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-a', lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    let mut handles = Vec::new();
    for i in 0..8 {
        let s = s.clone();
        let task_id = task_id.clone();
        let token = ctx.owner_session_token.clone();
        handles.push(tokio::spawn(async move {
            s.post(&format!("/api/work/tasks/{task_id}/progress"))
                .add_header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {token}"),
                )
                .json(&serde_json::json!({
                    "event_type": "status",
                    "summary": format!("concurrent {i}"),
                    "claim_lease_id": "lease-a",
                }))
                .await
                .status_code()
        }));
    }
    for h in handles {
        assert!(h.await.unwrap().is_success());
    }

    let seqs: Vec<i32> = sqlx::query_scalar("SELECT seq FROM meshprogressevent WHERE task_id=$1 ORDER BY seq")
        .bind(&task_id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        seqs,
        (0..8).collect::<Vec<i32>>(),
        "8 genuinely concurrent posts must get 8 distinct, sequential seq values, no duplicates: {seqs:?}"
    );
}
```

Note: `TestServer` must be `Clone` for the second test's `s.clone()` inside each spawned task — `axum_test::TestServer` implements `Clone` already (it's an `Arc`-backed handle); if this doesn't compile, check the `axum_test` version in `Cargo.toml` and adjust to `Arc::new(s)` + per-task `Arc::clone` instead, but try the direct `.clone()` first since it's the simpler form.

- [ ] **Step 2: Run to verify current behavior**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_heartbeat_still_works_after_family_a_refactor) or test(fencing_progress_concurrent_posts_get_sequential_seq_after_family_a_refactor)'`
Expected: `fencing_heartbeat_still_works_after_family_a_refactor` PASSes already (pre-refactor `heartbeat_task` already does this correctly — this is a regression guard). `fencing_progress_concurrent_posts_get_sequential_seq_after_family_a_refactor` is expected to be FLAKY-TO-FAILING against the current single-statement-CTE `append_progress` (the pre-existing race this task fixes) — if it happens to pass on a given run, that's the race not manifesting that particular time, not proof it's closed; re-run it 3-4 times against the pre-refactor code and note whether duplicates ever appear, then proceed to the fix regardless.

- [ ] **Step 3: Add `fence_claimable_live`, `row_to_progress`, the enums, and `execute_task_transition`'s Heartbeat/AppendProgress arms**

Append to `task_transitions.rs`:

```rust
use chrono::Utc;

/// Family A fence: live claimable-lease ownership+freshness check, shared by
/// Heartbeat and AppendProgress. Takes the row lock (`FOR UPDATE`) so the
/// caller's subsequent write — whether to `task` itself (Heartbeat) or a
/// different table (AppendProgress → `meshprogressevent`) — is atomic with
/// respect to any concurrent writer of this same task row. Returns the
/// locked row on success (unused by callers today beyond existence, but
/// available for future fence families that need to read fields off it
/// without a second round-trip).
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
}

pub enum TransitionOutcome {
    Task {
        task: serde_json::Value,
        unblocked_task_ids: Vec<String>,
    },
    Progress(serde_json::Value),
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
            let lease_expires = now + chrono::Duration::seconds(crate::routes::work::LEASE_TTL_SECS);
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
            let locked = fence_claimable_live(&mut tx, task_id, Some(claim_lease_id), actor.is_bypass, now)
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
            // Constraints).
            let seq: i32 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq), -1) + 1 FROM meshprogressevent WHERE task_id=$1",
            )
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
    }
}
```

- [ ] **Step 4: Refactor REST's `heartbeat_task` to call the service**

In `work.rs`, replace the whole body of `heartbeat_task` (from `let body = body.map(|b| b.0).unwrap_or_default();` through its closing brace) with:

```rust
async fn heartbeat_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<HeartbeatBody>>,
) -> impl IntoResponse {
    let body = body.map(|b| b.0).unwrap_or_default();

    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await
    {
        return resp;
    }

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::Heartbeat {
            claim_lease_id: body.claim_lease_id.as_deref(),
        },
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Task { task, .. }) => {
            Json(task).into_response()
        }
        Ok(_) => unreachable!("Heartbeat always yields TransitionOutcome::Task"),
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}
```

- [ ] **Step 5: Refactor REST's `append_progress` to call the service**

In `work.rs`, replace the whole body of `append_progress` (from `if body.claim_lease_id.is_empty()` through its closing brace) with:

```rust
async fn append_progress(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    Json(body): Json<ProgressCreate>,
) -> impl IntoResponse {
    if body.claim_lease_id.is_empty() {
        return bad_request("claim_lease_id is required");
    }

    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await
    {
        return resp;
    }

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::AppendProgress {
            claim_lease_id: &body.claim_lease_id,
            event_type: &body.event_type,
            phase: body.phase.as_deref(),
            step: body.step.as_deref(),
            summary: &body.summary,
            payload_json: &body.payload_json,
            agent_run_id: body.agent_run_id.as_deref(),
        },
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Progress(event)) => {
            Json(event).into_response()
        }
        Ok(_) => unreachable!("AppendProgress always yields TransitionOutcome::Progress"),
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}
```

- [ ] **Step 6: Run all tests in the fencing files**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification) or binary(test_work_progress_seq) or binary(test_authz)'`
Expected: PASS, including every pre-existing `fencing_heartbeat_*`/`fencing_progress_*` test (heartbeat's and progress's broadcast/stale-lease/idempotent-retry tests from Tasks 1 and 8 must all still pass unchanged — this is the regression-risk-bearing part of this task, confirm each one by name in the output, don't just check the aggregate count) and the two new tests from Step 1.

- [ ] **Step 7: Run the full suite and clippy**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml --no-fail-fast`
Expected: same pass count as Task 1's baseline plus the 2 new tests, one known unrelated failure.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/edgeplane-tower/src/routes/task_transitions.rs \
        crates/edgeplane-tower/src/routes/work.rs \
        crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "refactor(tower): heartbeat_task + append_progress via shared task_transitions (family A), close append_progress's row-lock TOCTOU"
```

---

### Task 3: Family B — Complete, REST wiring

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/task_transitions.rs` (add `Complete` to `TaskTransition`, `WaitingReview` to `TransitionOutcome`, the `Complete` arm of `execute_task_transition`)
- Modify: `crates/edgeplane-tower/src/routes/work.rs` — `complete_task` (search `async fn complete_task`) refactored
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Produces: `TaskTransition::Complete { claim_lease_id: Option<&'a str>, agent_id: Option<&'a str>, result_artifact_id: Option<i32> }`, `TransitionOutcome::WaitingReview { task: serde_json::Value, pending_gate_ids: Vec<String> }`.
- Consumes: `crate::routes::work::{unblock_dependents, broadcast_task_available}` (the former now `pub(crate)` per Task 1, the latter already `pub`).

- [ ] **Step 1: Write the failing test — behavioral parity across every one of `complete_task`'s distinct paths**

Add to `test_task_kind_unification.rs` (this is a parity check across paths already individually covered by Tasks 1-2/7's own tests — one test per path, not exhaustive re-coverage):

```rust
#[tokio::test]
async fn fencing_complete_still_routes_to_waiting_review_after_family_b_refactor() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-a', lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO reviewgate (id, owner_subject, mesh_task_id, run_id, gate_type, \
         required_approvals, status, created_at) \
         VALUES ($1, 'harness', $2, NULL, 'manual', 'any', 'pending', now())",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["status"], "waiting_review",
        "the pending-gate CTE path must still route to waiting_review after the family-B refactor: {body}"
    );
    assert_eq!(body["pending_gates"].as_array().unwrap().len(), 1);
    // The response shape stays exactly what it was pre-refactor — task_id +
    // pending_gates only, not a full task row (parity with the pre-refactor
    // handler, not a TransitionOutcome::WaitingReview implementation detail
    // leaking through).
    assert_eq!(body["task_id"], task_id);
    assert_eq!(body.as_object().unwrap().len(), 3, "response shape must be exactly {{status, pending_gates, task_id}}: {body}");
}

#[tokio::test]
async fn fencing_complete_still_unblocks_dependents_after_family_b_refactor() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-a', lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();
    let dependent_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "pending",
        None,
        1,
    )
    .await;
    sqlx::query("UPDATE task SET depends_on=$2 WHERE id=$1")
        .bind(&dependent_id)
        .bind(serde_json::json!([task_id]).to_string())
        .execute(&pool)
        .await
        .unwrap();

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "finished");
    assert_eq!(
        body["unblocked_tasks"].as_array().unwrap(),
        &vec![serde_json::json!(dependent_id)],
        "dependent-unblocking must still happen after the family-B refactor: {body}"
    );
}
```

- [ ] **Step 2: Run to verify current behavior**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_complete_still_routes_to_waiting_review_after_family_b_refactor) or test(fencing_complete_still_unblocks_dependents_after_family_b_refactor)'`
Expected: both PASS against the current, pre-refactor `complete_task` — these are pure regression guards.

- [ ] **Step 3: Add `Complete` to the enums and `execute_task_transition`**

Add to `TaskTransition` in `task_transitions.rs`:

```rust
    Complete {
        claim_lease_id: Option<&'a str>,
        agent_id: Option<&'a str>,
        result_artifact_id: Option<i32>,
    },
```

Add to `TransitionOutcome`:

```rust
    WaitingReview {
        task: serde_json::Value,
        pending_gate_ids: Vec<String>,
    },
```

Add a new match arm to `execute_task_transition`:

```rust
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
```

Note the argument to `task_id` in this arm: `execute_task_transition`'s own `task_id: &str` parameter shadows-in cleanly since the arm is inside the same function — no rebinding needed, use it directly as shown.

- [ ] **Step 4: Refactor REST's `complete_task` to call the service**

In `work.rs`, replace the whole body of `complete_task` (from `let body = body.map(|b| b.0).unwrap_or_default();` through its closing brace) with:

```rust
async fn complete_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<CompleteBody>>,
) -> impl IntoResponse {
    let body = body.map(|b| b.0).unwrap_or_default();

    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await
    {
        return resp;
    }

    let result_artifact_id: Option<i32> = body
        .result_artifact_id
        .as_deref()
        .and_then(|s| s.parse::<i32>().ok());

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::Complete {
            claim_lease_id: body.claim_lease_id.as_deref(),
            agent_id: body.agent_id.as_deref(),
            result_artifact_id,
        },
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Task { task, unblocked_task_ids }) => {
            let mut val = task;
            val["unblocked_tasks"] = serde_json::json!(unblocked_task_ids);
            Json(val).into_response()
        }
        Ok(crate::routes::task_transitions::TransitionOutcome::WaitingReview { pending_gate_ids, .. }) => {
            Json(serde_json::json!({
                "status": "waiting_review",
                "pending_gates": pending_gate_ids,
                "task_id": task_id,
            }))
            .into_response()
        }
        Ok(_) => unreachable!("Complete only yields Task or WaitingReview"),
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}
```

- [ ] **Step 5: Run all tests in the fencing files**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification) or binary(test_authz)'`
Expected: PASS, including every pre-existing `fencing_complete_*` test from Task 2's original implementation (broadcast, on-behalf-of, attribution, idempotent-retry) and the 2 new tests from Step 1.

- [ ] **Step 6: Full suite + clippy**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml --no-fail-fast`
Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: both clean, one known unrelated failure.

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplane-tower/src/routes/task_transitions.rs crates/edgeplane-tower/src/routes/work.rs \
        crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "refactor(tower): complete_task via shared task_transitions (family B)"
```

---

### Task 4: Family B — Fail, REST wiring

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/task_transitions.rs` (add `Fail` to `TaskTransition`, its arm)
- Modify: `crates/edgeplane-tower/src/routes/work.rs` — `fail_task` (search `async fn fail_task`) refactored
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Produces: `TaskTransition::Fail { claim_lease_id: Option<&'a str>, agent_id: Option<&'a str> }`.

Note: `FailBody.error` is `#[allow(dead_code)]` in the current code — it's accepted in the request body for API compatibility but never stored anywhere. This transition does not carry it through; `fail_task`'s handler keeps deserializing `FailBody` as-is (so the field stays API-compatible) but does not pass `body.error` into the transition call, matching current behavior exactly.

- [ ] **Step 1: Write the failing test**

Add to `test_task_kind_unification.rs`:

```rust
#[tokio::test]
async fn fencing_fail_still_works_after_family_b_refactor() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-a', lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a", "error": "boom"}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "failed");

    let row = sqlx::query(
        "SELECT claimed_by_agent_id, claim_lease_id, lease_expires_at, finalized_by_subject FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.get::<Option<String>, _>("claimed_by_agent_id").is_none());
    assert!(row.get::<Option<String>, _>("claim_lease_id").is_none());
    assert!(row.get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at").is_none());
}
```

- [ ] **Step 2: Run to verify current behavior**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_fail_still_works_after_family_b_refactor)'`
Expected: PASS against the current, pre-refactor `fail_task` — regression guard.

- [ ] **Step 3: Add `Fail` to the enum and `execute_task_transition`**

Add to `TaskTransition`:

```rust
    Fail {
        claim_lease_id: Option<&'a str>,
        agent_id: Option<&'a str>,
    },
```

Add a new match arm:

```rust
        TaskTransition::Fail {
            claim_lease_id,
            agent_id,
        } => {
            let effective_id = if actor.is_bypass {
                agent_id.unwrap_or(actor.subject_id)
            } else {
                actor.subject_id
            };
            let now = Utc::now().naive_utc();
            let now_tz = Utc::now();
            let row = sqlx::query(
                "UPDATE task SET status='failed', lease_expires_at=NULL, claim_lease_id=NULL, \
                 claimed_by_agent_id=NULL, finalized_at=$3, updated_at=$2, \
                 finalized_by_subject=COALESCE(claimed_by_agent_id, $6) \
                 WHERE id=$1 \
                   AND ( \
                     (kind = 'claimable' AND status IN ('claimed','running','waiting_review') \
                      AND (claimed_by_agent_id = $6 \
                           OR ((claim_lease_id = $4 OR $5) \
                               AND (claim_policy = 'broadcast' OR lease_expires_at >= $2)))) \
                     OR \
                     (kind = 'assigned' AND status NOT IN ('done','finished','failed','cancelled') \
                      AND (owner = $6 OR claim_lease_id = $4 OR $5)) \
                   ) \
                 RETURNING *",
            )
            .bind(task_id)
            .bind(now)
            .bind(now_tz)
            .bind(claim_lease_id)
            .bind(actor.is_bypass)
            .bind(effective_id)
            .fetch_optional(db)
            .await
            .map_err(|e| TransitionError::Database {
                operation: "fail update",
                source: e,
            })?;

            match row {
                Some(r) => Ok(TransitionOutcome::Task {
                    task: crate::routes::work::row_to_task(&r),
                    unblocked_task_ids: vec![],
                }),
                None => Err(classify_fenced_rejection(db, actor, task_id, claim_lease_id, &["failed"]).await),
            }
        }
```

- [ ] **Step 4: Refactor REST's `fail_task` to call the service**

In `work.rs`, replace the whole body of `fail_task` with:

```rust
async fn fail_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    body: Option<Json<FailBody>>,
) -> impl IntoResponse {
    let body = body.map(|b| b.0).unwrap_or_default();

    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await
    {
        return resp;
    }

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::Fail {
            claim_lease_id: body.claim_lease_id.as_deref(),
            agent_id: body.agent_id.as_deref(),
        },
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Task { task, .. }) => {
            Json(task).into_response()
        }
        Ok(_) => unreachable!("Fail always yields TransitionOutcome::Task"),
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}
```

- [ ] **Step 5: Run all tests in the fencing files**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification) or binary(test_authz)'`
Expected: PASS, including every pre-existing `fencing_fail_*` test and the new test from Step 1.

- [ ] **Step 6: Full suite + clippy**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml --no-fail-fast`
Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplane-tower/src/routes/task_transitions.rs crates/edgeplane-tower/src/routes/work.rs \
        crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "refactor(tower): fail_task via shared task_transitions (family B)"
```

---

### Task 5: Family D — Block, REST wiring

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/task_transitions.rs` (add `Block` to `TaskTransition`, its arm)
- Modify: `crates/edgeplane-tower/src/routes/work.rs` — `block_task` (search `async fn block_task`) refactored
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Produces: `TaskTransition::Block` (no fields — block takes no body today).

- [ ] **Step 1: Write the failing test**

Add to `test_task_kind_unification.rs`:

```rust
#[tokio::test]
async fn fencing_block_still_works_after_family_d_refactor() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-a', lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "blocked");

    let row = sqlx::query("SELECT claimed_by_agent_id, claim_lease_id, lease_expires_at FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        row.get::<Option<String>, _>("claimed_by_agent_id").is_some(),
        "block must still PRESERVE claimed_by_agent_id (deliberate, see work.rs's original comment) after the family-D refactor"
    );
    assert!(row.get::<Option<String>, _>("claim_lease_id").is_none());
    assert!(row.get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at").is_none());
}
```

- [ ] **Step 2: Run to verify current behavior**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_block_still_works_after_family_d_refactor)'`
Expected: PASS against the current, pre-refactor `block_task` — regression guard.

- [ ] **Step 3: Add `Block` to the enum and `execute_task_transition`**

Add to `TaskTransition`:

```rust
    Block,
```

Add a new match arm:

```rust
        TaskTransition::Block => {
            let now = Utc::now().naive_utc();
            let row = sqlx::query(
                "UPDATE task SET status='blocked', \
                 lease_expires_at=NULL, claim_lease_id=NULL, updated_at=$2 \
                 WHERE id=$1 AND kind='claimable' AND status IN ('claimed','running') \
                   AND (claimed_by_agent_id = $3 OR $4) \
                 RETURNING *",
            )
            .bind(task_id)
            .bind(now)
            .bind(actor.subject_id)
            .bind(actor.is_bypass)
            .fetch_optional(db)
            .await
            .map_err(|e| TransitionError::Database {
                operation: "block update",
                source: e,
            })?;

            match row {
                Some(r) => Ok(TransitionOutcome::Task {
                    task: crate::routes::work::row_to_task(&r),
                    unblocked_task_ids: vec![],
                }),
                None => Err(classify_fenced_rejection(db, actor, task_id, None, &["blocked"]).await),
            }
        }
```

- [ ] **Step 4: Refactor REST's `block_task` to call the service**

In `work.rs`, replace the whole body of `block_task` with:

```rust
async fn block_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) =
        crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await
    {
        return resp;
    }

    let actor = crate::routes::task_transitions::task_actor(&principal);
    let outcome = crate::routes::task_transitions::execute_task_transition(
        &state.db,
        &actor,
        &task_id,
        crate::routes::task_transitions::TaskTransition::Block,
    )
    .await;

    match outcome {
        Ok(crate::routes::task_transitions::TransitionOutcome::Task { task, .. }) => {
            Json(task).into_response()
        }
        Ok(_) => unreachable!("Block always yields TransitionOutcome::Task"),
        Err(e) => crate::routes::task_transitions::rest_transition_error(e),
    }
}
```

- [ ] **Step 5: Run all tests in the fencing files**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification) or binary(test_authz)'`
Expected: PASS, including every pre-existing `fencing_block_*` test and the new test from Step 1.

- [ ] **Step 6: Full suite + clippy**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml --no-fail-fast`
Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplane-tower/src/routes/task_transitions.rs crates/edgeplane-tower/src/routes/work.rs \
        crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "refactor(tower): block_task via shared task_transitions (family D) — REST side of the primitive complete"
```

At this point every REST endpoint's behavior is unchanged (proven by the full existing suite staying green through 5 refactors) and the service is complete and proven for all 5 transitions. Task 6 is where the actual security fix lands — wiring MCP to the same service.

---

### Task 6: Wire MCP's five arms to the shared service

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs` — `heartbeat_mesh_task`, `progress_mesh_task`, the combined `complete_mesh_task | fail_mesh_task | block_mesh_task` arm (search each tool name string to locate)
- Modify: `crates/edgeplane-tower/tests/test_mcp_progress_mesh_task.rs` — fix the pre-existing test that predates fencing

**Interfaces:**
- Consumes: everything from Tasks 1-5 (`task_actor`, `execute_task_transition`, `TaskTransition`, `TransitionOutcome`, `TransitionError`).
- Produces: `pub(crate) fn mcp_transition_error(error: crate::routes::task_transitions::TransitionError) -> serde_json::Value` (new, in `mcp.rs` near `ok_result`/`err_result`) — the MCP-side adapter, mirroring `rest_transition_error`.

- [ ] **Step 1: Write the failing tests proving the actual security fix — this is the core deliverable of this whole plan**

Create `crates/edgeplane-tower/tests/test_mcp_fenced_transitions.rs`:

```rust
//! MCP-side fencing tests for heartbeat/complete/fail/block/progress —
//! these five previously either had no freshness check at all
//! (heartbeat_mesh_task, progress_mesh_task) or were check-then-act with no
//! fencing whatsoever (complete_mesh_task/fail_mesh_task/block_mesh_task).
//! Mirrors the REST-side fencing coverage this same suite already has for
//! the equivalent endpoints in test_task_kind_unification.rs.

mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::{PgPool, Row};

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

/// The exact exploit chain an independent security review reproduced live
/// against the pre-fix code: a caller whose lease has genuinely expired
/// (REST correctly rejects it) revives it through MCP's heartbeat_mesh_task
/// — which had zero freshness checking — and REST access is then restored.
/// This test proves that chain is closed.
#[tokio::test]
async fn mcp_heartbeat_on_expired_lease_is_rejected_and_cannot_revive_rest_access() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-a', lease_expires_at = now() - interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    // REST correctly rejects the expired lease first (baseline, unchanged).
    let rest_res = s
        .post(&format!("/api/work/tasks/{task_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert_eq!(rest_res.status_code(), 409, "sanity: REST must reject the expired lease first: {}", rest_res.text());

    // The exploit attempt: revive the same expired lease through MCP.
    let mcp_res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "heartbeat_mesh_task",
            "args": {"task_id": task_id, "claim_lease_id": "lease-a"}
        }))
        .await;
    let mcp_body: serde_json::Value = mcp_res.json();
    assert_eq!(
        mcp_body["ok"], false,
        "MCP heartbeat on a genuinely expired lease must fail, not silently revive it: {mcp_body}"
    );

    // Confirm the revival didn't happen even partially — the row's lease
    // must still be expired, not pushed into the future.
    let lease_expires_at: chrono::NaiveDateTime = sqlx::query_scalar("SELECT lease_expires_at FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        lease_expires_at < chrono::Utc::now().naive_utc(),
        "the lease must still be expired in the database — no partial revival"
    );

    // REST access must still be rejected — the whole point of this test.
    let rest_res_again = s
        .post(&format!("/api/work/tasks/{task_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert_eq!(
        rest_res_again.status_code(),
        409,
        "REST access must NOT have been restored by the failed MCP revival attempt: {}",
        rest_res_again.text()
    );
}

/// Deliberate behavior change (see this plan's Global Constraints): MCP's
/// heartbeat previously granted a 300s window; routing through the shared
/// service unifies it to REST's LEASE_TTL_SECS (120s). This test locks in
/// the new value so a future change to either surface's TTL has to touch
/// this test, not drift unnoticed again.
#[tokio::test]
async fn mcp_heartbeat_grants_120s_not_300s() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-a', lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    let before = chrono::Utc::now();
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "heartbeat_mesh_task",
            "args": {"task_id": task_id, "claim_lease_id": "lease-a"}
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(body["ok"], true, "{body}");

    let lease_expires_at: chrono::NaiveDateTime = sqlx::query_scalar("SELECT lease_expires_at FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let granted_secs = (lease_expires_at.and_utc() - before).num_seconds();
    assert!(
        (115..=125).contains(&granted_secs),
        "MCP heartbeat must now grant ~120s (LEASE_TTL_SECS), not the old 300s: got {granted_secs}s"
    );
}

#[tokio::test]
async fn mcp_progress_requires_lease_and_freshness_now() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-a', lease_expires_at = now() - interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    // No lease at all — previously accepted (lease was optional), must be
    // rejected now.
    let res_no_lease = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "progress_mesh_task",
            "args": {"task_id": task_id, "event_type": "status"}
        }))
        .await;
    let body_no_lease: serde_json::Value = res_no_lease.json();
    assert_eq!(body_no_lease["ok"], false, "progress without a lease must be rejected: {body_no_lease}");

    // Expired lease presented — must be rejected too.
    let res_expired = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "progress_mesh_task",
            "args": {"task_id": task_id, "event_type": "status", "claim_lease_id": "lease-a"}
        }))
        .await;
    let body_expired: serde_json::Value = res_expired.json();
    assert_eq!(body_expired["ok"], false, "progress with an expired lease must be rejected: {body_expired}");

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM meshprogressevent WHERE task_id=$1")
            .bind(&task_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "no progress event must have been inserted by either rejected attempt"
    );
}

#[tokio::test]
async fn mcp_complete_stale_lease_after_reclaim_is_rejected() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 2).await;

    let claim_res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "claim_mesh_task",
            "args": {"task_id": task_id, "agent_id": "agent-A"}
        }))
        .await;
    let claim_body: serde_json::Value = claim_res.json();
    let lease_a = claim_body["result"]["claim_lease_id"].as_str().unwrap().to_string();

    // Force the lease into the past and let the reclaim sweep run via a
    // list_tasks call (the existing trigger_reclaim_sweep pattern from
    // test_task_kind_unification.rs — reimplemented here since MCP tests
    // don't share that file's helpers).
    sqlx::query("UPDATE task SET lease_expires_at = now() - interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .unwrap();
    let _ = s
        .get(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;

    let complete_res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "complete_mesh_task",
            "args": {"task_id": task_id, "claim_lease_id": lease_a}
        }))
        .await;
    let complete_body: serde_json::Value = complete_res.json();
    assert_eq!(
        complete_body["ok"], false,
        "a stale lease from before the reclaim sweep must not complete a task now reclaimed: {complete_body}"
    );
}

#[tokio::test]
async fn mcp_block_has_a_status_precondition_now() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "finished",
        Some("agent-A"),
        1,
    )
    .await;

    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "block_mesh_task",
            "args": {"task_id": task_id}
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["ok"], false,
        "block_mesh_task must reject an already-finished task now (previously had no status precondition at all): {body}"
    );
}

#[tokio::test]
async fn mcp_fail_broadcast_task_without_matching_lease_is_rejected() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_policy='broadcast', claim_lease_id='lease-a', \
         lease_expires_at = now() - interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "fail_mesh_task",
            "args": {"task_id": task_id, "claim_lease_id": "not-the-real-lease"}
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["ok"], false,
        "an unrelated caller with a non-matching lease must not fail someone else's broadcast task via MCP: {body}"
    );

    let status: String = sqlx::query_scalar("SELECT status FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "running", "the task must not have been failed");
}
```

- [ ] **Step 2: Run to verify failures**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_mcp_fenced_transitions)'`
Expected: every test FAILs against the current, pre-refactor MCP code — this is the whole point. `mcp_heartbeat_on_expired_lease_is_rejected_and_cannot_revive_rest_access` fails specifically because the MCP call currently returns `ok: true` (the live-reproduced exploit). Confirm each failure message matches what you'd expect from the currently-unfenced code before proceeding — don't just see red and move on.

- [ ] **Step 3: Add `mcp_transition_error` and refactor `heartbeat_mesh_task`**

In `mcp.rs`, add near `ok_result`/`err_result` (search `fn err_result`):

```rust
fn mcp_transition_error(error: crate::routes::task_transitions::TransitionError) -> Value {
    use crate::routes::task_transitions::TransitionError;
    match error {
        TransitionError::NotFound => err_result("task not found"),
        TransitionError::Forbidden => err_result("not the task's claimer"),
        TransitionError::Conflict => err_result("task is not in the required state for this transition"),
        TransitionError::Invalid(detail) => err_result(&detail),
        TransitionError::Database { operation, source } => {
            tracing::error!("{operation}: {source}");
            err_result("database_error")
        }
    }
}
```

Replace the whole `"heartbeat_mesh_task" => { ... }` arm (search `"heartbeat_mesh_task" => {`) with:

```rust
        "heartbeat_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            let claim_lease_id = str_arg(args, "claim_lease_id");
            if task_id.is_empty() || claim_lease_id.is_empty() {
                return err_result("task_id and claim_lease_id are required");
            }
            let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
                Ok(d) => d,
                Err(_) => return err_result("task not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let actor = crate::routes::task_transitions::task_actor(principal);
            let outcome = crate::routes::task_transitions::execute_task_transition(
                &state.db,
                &actor,
                &task_id,
                crate::routes::task_transitions::TaskTransition::Heartbeat {
                    claim_lease_id: Some(&claim_lease_id),
                },
            )
            .await;
            match outcome {
                Ok(crate::routes::task_transitions::TransitionOutcome::Task { task, .. }) => {
                    ok_result(json!({
                        "task_id": task_id,
                        "lease_expires_at": task.get("lease_expires_at").cloned().unwrap_or(Value::Null),
                    }))
                }
                Ok(_) => err_result("database_error"),
                Err(e) => mcp_transition_error(e),
            }
        }
```

Note this drops the `kind != "claimable"` precheck the old code had — `execute_task_transition`'s Heartbeat arm's fence already requires `kind='claimable'` in its own `WHERE`, so a `kind='assigned'` task now correctly classifies through `classify_fenced_rejection` (404/409/403 per its existing rules) instead of a bespoke `"task is not claimable"` string. This is an intentional, minor MCP-side error-message change — not a behavior regression, since MCP has no test pinning the exact old string (confirmed by grep before this step: search the whole `tests/` directory for `"task is not claimable"` and verify no test asserts on it before proceeding; if one exists, update its assertion to check `body["ok"] == false` instead of the exact message).

Also note the TTL change this same code causes: the old arm computed `now + chrono::Duration::seconds(300)` itself; the shared Heartbeat arm uses `LEASE_TTL_SECS` (120s) instead. This is the deliberate unification documented in Global Constraints, not an accident — `mcp_heartbeat_grants_120s_not_300s` in Step 1 locks it in.

- [ ] **Step 4: Refactor `progress_mesh_task`**

Replace the whole `"progress_mesh_task" => { ... }` arm with:

```rust
        "progress_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            let event_type = str_arg(args, "event_type");
            let claim_lease_id = str_arg(args, "claim_lease_id");
            if task_id.is_empty() || event_type.is_empty() || claim_lease_id.is_empty() {
                return err_result("task_id, event_type, and claim_lease_id are required");
            }
            let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
                Ok(d) => d,
                Err(_) => return err_result("task not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let payload_json = args.get("payload_json").cloned().unwrap_or(json!({})).to_string();
            let phase = args.get("phase").and_then(|v| v.as_str());
            let step = args.get("step").and_then(|v| v.as_str());
            let actor = crate::routes::task_transitions::task_actor(principal);
            let outcome = crate::routes::task_transitions::execute_task_transition(
                &state.db,
                &actor,
                &task_id,
                crate::routes::task_transitions::TaskTransition::AppendProgress {
                    claim_lease_id: &claim_lease_id,
                    event_type: &event_type,
                    phase,
                    step,
                    summary: "",
                    payload_json: &payload_json,
                    agent_run_id: None,
                },
            )
            .await;
            match outcome {
                Ok(crate::routes::task_transitions::TransitionOutcome::Progress(event)) => {
                    ok_result(json!({
                        "event_id": event.get("id").cloned().unwrap_or(Value::Null),
                        "task_id": task_id,
                        "event_type": event_type,
                    }))
                }
                Ok(_) => err_result("database_error"),
                Err(e) => mcp_transition_error(e),
            }
        }
```

Note `claim_lease_id` is now REQUIRED (was optional) — this is the actual security fix for this arm, matching REST's `append_progress` requirement. Note also `summary: ""` — the MCP tool never accepted a `summary` argument before this refactor either (confirmed: the old arm's INSERT never bound one); passing an empty string through the shared service reproduces that exact prior behavior rather than silently inventing new MCP surface.

- [ ] **Step 5: Refactor the combined `complete_mesh_task | fail_mesh_task | block_mesh_task` arm**

Replace the whole arm (search `"complete_mesh_task" | "fail_mesh_task" | "block_mesh_task" => {`) with:

```rust
        "complete_mesh_task" | "fail_mesh_task" | "block_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() {
                return err_result("task_id is required");
            }
            let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
                Ok(d) => d,
                Err(_) => return err_result("task not found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }
            let lease_str = str_arg(args, "claim_lease_id");
            let lease_opt = if lease_str.is_empty() { None } else { Some(lease_str.as_str()) };
            let actor = crate::routes::task_transitions::task_actor(principal);
            let transition = match tool {
                "complete_mesh_task" => crate::routes::task_transitions::TaskTransition::Complete {
                    claim_lease_id: lease_opt,
                    agent_id: None,
                    result_artifact_id: None,
                },
                "fail_mesh_task" => crate::routes::task_transitions::TaskTransition::Fail {
                    claim_lease_id: lease_opt,
                    agent_id: None,
                },
                "block_mesh_task" => crate::routes::task_transitions::TaskTransition::Block,
                _ => return err_result("unknown_tool"),
            };
            let outcome =
                crate::routes::task_transitions::execute_task_transition(&state.db, &actor, &task_id, transition)
                    .await;
            match outcome {
                Ok(crate::routes::task_transitions::TransitionOutcome::Task { task, .. }) => {
                    ok_result(json!({"task_id": task_id, "status": task.get("status").cloned().unwrap_or(Value::Null)}))
                }
                Ok(crate::routes::task_transitions::TransitionOutcome::WaitingReview { pending_gate_ids, .. }) => {
                    ok_result(json!({"task_id": task_id, "status": "waiting_review", "pending_gates": pending_gate_ids}))
                }
                Ok(_) => err_result("database_error"),
                Err(e) => mcp_transition_error(e),
            }
        }
```

Note `block_mesh_task` gains a real status precondition for the first time (previously had none at all — `if tool != "block_mesh_task"` skipped the old precondition check entirely for it) and `complete_mesh_task`/`fail_mesh_task` now stamp `finalized_by_subject` (via the shared service) where the old combined UPDATE never did.

- [ ] **Step 6: Fix the pre-existing `progress_mesh_task_inserts_sequential_events` test**

In `test_mcp_progress_mesh_task.rs`, replace the body of `progress_mesh_task_inserts_sequential_events`'s setup and loop:

```rust
#[tokio::test]
async fn progress_mesh_task_inserts_sequential_events() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = seed_ready_task(&pool, &ctx.mission_id, &ctx.domain_id).await;
    sqlx::query(
        "UPDATE task SET status='running', claim_lease_id='lease-seq-test', \
         lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("seed a live lease");
    let s = server(pool.clone());

    for i in 0..3 {
        let res = s
            .post("/api/mcp/call")
            .add_header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", ctx.owner_session_token),
            )
            .json(&serde_json::json!({
                "tool": "progress_mesh_task",
                "args": {
                    "task_id": task_id,
                    "event_type": "phase_finished",
                    "claim_lease_id": "lease-seq-test",
                    "payload_json": {"iteration": i},
                }
            }))
            .await;
        res.assert_status_ok();
        let body: serde_json::Value = res.json();
        assert_eq!(body["ok"], true, "iteration {i}: response body: {body}");
    }

    let seqs: Vec<i32> = sqlx::query_scalar(
        "SELECT seq FROM meshprogressevent WHERE task_id = $1 ORDER BY seq",
    )
    .bind(&task_id)
    .fetch_all(&pool)
    .await
    .expect("fetch progress events");
    assert_eq!(seqs, vec![0, 1, 2], "seq must be sequential per task, not null/duplicated");
}
```

(`Row` import for `.get` — confirm `use sqlx::Row;` is already present at the top of this file from the earlier `sqlx::query_scalar` usage; if not, add it.)

- [ ] **Step 7: Run all MCP and fencing tests**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_mcp_fenced_transitions) or binary(test_mcp_progress_mesh_task) or binary(test_mcp_submit_mesh_task) or binary(mcp_parity)'`
Expected: PASS, all of it — including `mcp_heartbeat_on_expired_lease_is_rejected_and_cannot_revive_rest_access`, which is the direct proof the HIGH-severity finding is closed.

- [ ] **Step 8: Full suite + clippy**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml --no-fail-fast`
Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: one known unrelated failure, everything else green; clippy clean.

- [ ] **Step 9: Commit**

```bash
git add crates/edgeplane-tower/src/routes/mcp.rs \
        crates/edgeplane-tower/tests/test_mcp_fenced_transitions.rs \
        crates/edgeplane-tower/tests/test_mcp_progress_mesh_task.rs
git commit -m "fix(tower): MCP mesh-task heartbeat/progress/complete/fail/block via shared task_transitions — closes the lease-revival gap"
```

---

### Task 7: Final verification — dedicated exploit-closure confidence pass, docs

**Files:**
- Modify: `docs/superpowers/plans/2026-08-18-ep1-tower-fencing.md` (roadmap: mark the MCP progress/heartbeat gap and the append_progress row-lock finding as closed by this plan, cross-reference this plan's completion)
- No code changes — this task is verification + documentation only.

- [ ] **Step 1: Run the complete `edgeplane-tower` suite one more time, from a clean state**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml --no-fail-fast 2>&1 | tail -20`
Expected: every test passes except the one known pre-existing `meshprogressevent_summary_decodes_when_null` failure. Count the total tests run and compare against Task 6's own count — it must match (this task adds no new tests, it's purely a final confirmation pass).

- [ ] **Step 2: Run `cargo nextest run` for the whole workspace, not just edgeplane-tower**

Run: `cargo nextest run --workspace --no-fail-fast 2>&1 | tail -30`
Expected: no new failures anywhere else in the workspace (confirms `edgeplaned-work`/`edgeplaned-bin`/`edgeplane` CLI code — none of which this plan touches — are unaffected).

- [ ] **Step 3: Full workspace clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Update the EP-1 plan's roadmap to mark closed items**

In `docs/superpowers/plans/2026-08-18-ep1-tower-fencing.md`, find the roadmap entry beginning `**progress_mesh_task (MCP) is unfenced and outside even Task 9's stated scope**` and the one beginning `**append_progress's fenced CTE lacks a row lock**`. Prepend each with a bolded closure note, e.g.:

```markdown
- **CLOSED by `docs/superpowers/plans/2026-08-28-shared-fenced-transition-primitive.md`.** `progress_mesh_task` (MCP) is unfenced...
```

and

```markdown
- **CLOSED by `docs/superpowers/plans/2026-08-28-shared-fenced-transition-primitive.md`.** `append_progress`'s fenced CTE lacks a row lock...
```

Do not delete the original text — the historical record of what was found and why stays; only the closure note is added.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/plans/2026-08-18-ep1-tower-fencing.md
git commit -m "docs(specs): mark MCP lease-revival + append_progress row-lock roadmap items closed"
```

- [ ] **Step 6: Push**

```bash
GIT_SSH_COMMAND="ssh -F /dev/null" git push origin feat/ep1-fenced-dispatch
```

(The `GIT_SSH_COMMAND` override is this branch's established workaround for a broken `ssh_config.d` on the excalibur host — see the SDD ledger and `.learnings/ERRORS.md` for context if it's no longer needed by the time this runs.)

---

## Self-Review Notes

- **Spec coverage:** every fence family (A/B/D) has a task; the "not a macro" constraint is honored (`fence_claimable_live`, the Complete/Fail SQL, and the Block SQL are all separately visible, not generated); REST-and-MCP-both-call-the-service is honored (Tasks 2-5 do REST, Task 6 does MCP against the now-complete service); the CLI-first/thin-MCP principle is honored structurally (Task 6's MCP arms do only arg-parsing + one service call + response formatting, matching the spec's explicit non-goal of not building new MCP surface — `progress_mesh_task`'s `summary` field is deliberately NOT added as new MCP surface, for example).
- **Placeholder scan:** no task contains "TBD"/"handle errors appropriately"/unshown code — every SQL statement and Rust function body is complete and copied from (or directly adapted from) verified current source.
- **Type consistency:** `TransitionOutcome::Task`'s `unblocked_task_ids` field is populated by every arm that can produce it (Complete: real value from `unblock_dependents`; Heartbeat/Fail/Block: always `vec![]`) — checked across Tasks 2, 4, 5 for consistency with Task 3's definition.
- **Caught on this pass:** the first draft of Task 6 silently changed `heartbeat_mesh_task`'s TTL from 300s to 120s (an inevitable consequence of routing it through the same Heartbeat arm REST uses, which reads `LEASE_TTL_SECS`) without ever calling it out or testing it — exactly the "undocumented behavior change" class of bug this whole plan's parent branch has repeatedly found in review. Fixed: promoted to an explicit Global Constraint, with its own test (`mcp_heartbeat_grants_120s_not_300s`) and a code comment at the call site. This also happens to resolve a TTL-divergence question the original EP-1 plan's own roadmap had left explicitly open.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-08-28-shared-fenced-transition-primitive.md`. Given this plan touches security-sensitive fencing/auth logic across 5 already-shipped REST endpoints and 5 previously-unfenced MCP endpoints — the same class of code where independent review has found a real, actionable issue on every single task of the parent EP-1 plan — **every task in this plan should get an independent adversarial review (rust-reviewer, matching this whole branch's established discipline) before moving to the next task**, not just at the end.

Two execution options:

1. **Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks.
2. **Inline Execution** — execute tasks in this session using `executing-plans`, batch execution with checkpoints.

Which approach?
