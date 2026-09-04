# retry_task / create_gate Fencing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the two remaining unfenced mutating task-lifecycle gaps flagged in the EP-1 Roadmap —
`retry_task`'s blind `UPDATE ... WHERE id=$1` (can rip an actively-claimed, in-progress task back to
`ready` out from under its new claimer) and `create_gate`'s check-then-insert (a caller who owned the
task at check-time but has since lost ownership can still attach a pending review gate).

**Architecture:** Both fixes follow the exact "Family B" single-statement CAS pattern
`task_transitions.rs`'s `Fail`/`Block` arms already establish: fold the precondition (status, and for
`create_gate`, ownership) into the mutating statement's own `WHERE`/`WHERE EXISTS` clause, so the
response is derived from whether the statement affected a row, not from an earlier separate `SELECT`.
Both endpoints are single-surface (REST only, no MCP mirror — confirmed via grep), so neither needs a
shared `task_transitions.rs` addition; the fenced statement lives directly in each handler in
`work.rs`, matching that file's own pre-shared-primitive convention for single-surface endpoints.

**Tech Stack:** Rust (edition 2024), axum, sqlx (raw `query`, not `query!`), Postgres,
`axum_test::TestServer` integration tests gated on `TEST_DATABASE_URL`.

**Spec:** `docs/superpowers/plans/2026-08-18-ep1-tower-fencing.md`, Roadmap section (lines 2335-2344 for
`retry_task`, lines 2313-2324 for `create_gate`) — these two Roadmap entries are the spec; no separate
design doc exists for this follow-up, the entries already carry full problem statements and the fix
shape was independently re-derived and confirmed against the current code in this planning pass.

## Global Constraints

- Branch fresh from current `main` (`git checkout main && git pull && git checkout -b
  fix/retry-and-gate-fencing`) — do NOT attempt to resume `feat/ep1-fenced-dispatch`, it no longer
  exists (deleted after PR #120 merged as `b857e92c`).
- All new/modified SQL uses `sqlx::query` + `.bind()` + `Row::get`, matching every existing handler in
  `work.rs` — do not introduce `sqlx::query!`.
- `retry_task` has **no ownership dimension by design** — any domain member may retry a
  `failed`/`cancelled` task today; this was explicitly ruled out of scope (log-only) by Merlin on
  2026-08-26 per the Roadmap entry. This plan does **not** add an ownership check to `retry_task` — it
  only closes the TOCTOU on the status/kind precondition. Do not conflate the two.
- `create_gate`'s ownership check (Change 10: only the task's claimer, or full-trust/admin, may attach
  a gate) is preserved exactly — this plan folds it into the fenced INSERT's `WHERE EXISTS`, it does
  not change who is allowed to create a gate, only when the check runs.
- "Gate-attachable status" for `create_gate`'s new status fence mirrors `complete_task`'s own
  non-terminal predicate (`task_transitions.rs`'s `Complete` arm): `kind='claimable' AND status IN
  ('claimed','running','waiting_review')`, or `kind='assigned' AND status NOT IN
  ('done','finished','failed','cancelled')`. A gate only makes sense before the task reaches a status
  `complete_task` itself would treat as terminal.
- Test DB: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test`, `cargo nextest run`
  (not plain `cargo test`) per this crate's CI convention. Container `edgeplane-test-pg` — verify with
  `docker ps --filter name=edgeplane-test-pg` before running; confirmed up as of this plan's writing.
- Every task ends with `cargo clippy --workspace --all-targets -- -D warnings` passing.
- Per this profile's CLAUDE.md: seam-level/authz changes require a dedicated security-reviewer
  red-team pass before merge, in addition to a whole-branch rust-reviewer pass — do both after Task 2,
  before opening the PR (not part of the per-task steps below, called out here so it isn't dropped).

---

## File Structure

- Modify `crates/edgeplane-tower/src/routes/work.rs` — `retry_task` (994-1047), `create_gate`
  (1682-1738); adds two new private helpers, `classify_retry_rejection` and
  `classify_create_gate_rejection`, following the same "diagnostic read after a failed fenced write"
  shape `task_transitions.rs`'s `classify_fenced_rejection` already established.
- Modify `crates/edgeplane-tower/tests/test_task_kind_unification.rs` — new `retry_task` fencing tests,
  inserted before the `// ── Bounded retry / backoff` section header (line 755).
- Modify `crates/edgeplane-tower/tests/test_authz.rs` — new `create_gate` tests, inserted after
  `domain_peer_cannot_create_gate_on_foreign_task` (ends line 719), before the `// ── send_mesh_message
  sender anti-spoof` header (line 721).

---

### Task 1: Fence `retry_task`'s blind UPDATE

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:994-1047` (`retry_task`)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Produces: `async fn classify_retry_rejection(db: &sqlx::PgPool, task_id: &str) ->
  axum::response::Response` (private fn in `work.rs`, used only by `retry_task`).
- Consumes: `crate::routes::authz::domain_id_for_task` (existing, `authz.rs:174`),
  `crate::routes::authz::authz_domain` (existing), `not_found`/`conflict` (existing, `work.rs:145-158`),
  `row_to_task` (existing).

- [ ] **Step 1: Write the failing tests**

Insert into `crates/edgeplane-tower/tests/test_task_kind_unification.rs`, between the closing `}` of
`fencing_heartbeat_real_agent_own_live_lease_succeeds` at line 753 and the `// ── Bounded
retry / backoff` header at line 755 (i.e. replace the blank line 754 with the new section + tests
below, keeping one blank line before the existing header):

```rust
// ── retry_task — fenced CAS, closes blind-UPDATE TOCTOU ─────────────────────

#[tokio::test]
async fn fencing_retry_task_stale_read_after_reclaim_is_409() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Seed a task already back in 'ready' (as if a concurrent retry/reclaim
    // already happened after this caller's hypothetical earlier read saw
    // 'failed') — the old code's blind UPDATE would apply the retry-reset
    // unconditionally regardless of current status; the fenced version must
    // reject it as a status conflict.
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-new-claimer"),
        1,
    )
    .await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/retry"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "retrying a task that is not failed/cancelled must be 409, not silently reset: {}",
        res.text()
    );

    let row = sqlx::query(
        "SELECT status, claimed_by_agent_id, claim_lease_id FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row.get::<String, _>("status"),
        "running",
        "rejected retry must not have touched the row's status"
    );
    assert_eq!(
        row.get::<Option<String>, _>("claimed_by_agent_id").as_deref(),
        Some("agent-new-claimer"),
        "rejected retry must not have cleared the current claimer's ownership"
    );
}

#[tokio::test]
async fn fencing_retry_task_wrong_kind_is_409() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id =
        common::seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/retry"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "retry on a kind='assigned' row must be 409: {}",
        res.text()
    );
    assert!(
        res.text().contains("not claimable"),
        "must preserve the existing kind-specific error message: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_retry_task_from_failed_still_succeeds() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "failed",
        Some("agent-old-claimer"),
        1,
    )
    .await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/retry"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "retry from failed must still succeed: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "ready");

    let row = sqlx::query(
        "SELECT claimed_by_agent_id, claim_lease_id, lease_expires_at FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.get::<Option<String>, _>("claimed_by_agent_id").is_none());
    assert!(row.get::<Option<String>, _>("claim_lease_id").is_none());
    assert!(row
        .get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at")
        .is_none());
}

```

- [ ] **Step 2: Run the tests to verify failures**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_retry_task)'`
Expected: `fencing_retry_task_stale_read_after_reclaim_is_409` FAILs — current code's blind `UPDATE ...
WHERE id=$1` has no status/kind predicate, so it unconditionally resets the seeded `running` row to
`ready`, returning 200 instead of 409 (and clearing `claimed_by_agent_id`, breaking the second
assertion too). `fencing_retry_task_wrong_kind_is_409` and `fencing_retry_task_from_failed_still_succeeds`
should currently PASS (regression guards for existing precondition checks and the happy path) —
confirm this before proceeding.

- [ ] **Step 3: Rewrite `retry_task`**

Replace `work.rs:994-1047` with:

```rust
async fn retry_task(
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

    let now = Utc::now().naive_utc();
    let updated = sqlx::query(
        "UPDATE task SET status='ready', claimed_by_agent_id=NULL, result_artifact_id=NULL, \
         lease_expires_at=NULL, claim_lease_id=NULL, finalized_at=NULL, \
         finalized_by_subject=NULL, updated_at=$2 \
         WHERE id=$1 AND kind='claimable' AND status IN ('failed','cancelled') \
         RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .fetch_optional(&state.db)
    .await;

    match updated {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => classify_retry_rejection(&state.db, &task_id).await,
        Err(e) => {
            tracing::error!("retry_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// After the fenced retry UPDATE rejects (zero rows), classify why with a
/// fresh read — preserves retry_task's existing precondition-specific error
/// messages (kind vs. status) while ensuring the message reflects the
/// task's CURRENT state, not the pre-UPDATE snapshot the old check-then-act
/// code read (which could already be stale by the time the blind UPDATE
/// ran). No ownership/lease dimension here: retry_task performs no
/// per-caller ownership check by design (any domain member may retry a
/// failed/cancelled task — ruled by Merlin 2026-08-26, see
/// docs/superpowers/plans/2026-08-18-ep1-tower-fencing.md Roadmap,
/// "retry_task — severity correction"). This function only closes the
/// TOCTOU on the status/kind precondition, it does not add authorization.
async fn classify_retry_rejection(db: &sqlx::PgPool, task_id: &str) -> axum::response::Response {
    let row = match sqlx::query("SELECT kind, status FROM task WHERE id=$1")
        .bind(task_id)
        .fetch_optional(db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("classify_retry_rejection fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let kind: String = row.get("kind");
    if kind != "claimable" {
        return conflict("Task is not claimable (kind='assigned'); retry does not apply");
    }
    let status: String = row.get("status");
    conflict(&format!("Task cannot be retried from status: {status}"))
}
```

- [ ] **Step 4: Run all tests in the file**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS, including `claimable_task_claim_heartbeat_progress_retry_still_work` (the pre-existing
end-to-end retry usage test) and all three new tests.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "fix(tower): fence retry_task's blind UPDATE against status/kind TOCTOU"
```

---

### Task 2: Fence `create_gate`'s check-then-insert

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:1682-1738` (`create_gate`)
- Test: `crates/edgeplane-tower/tests/test_authz.rs`

**Interfaces:**
- Produces: `async fn classify_create_gate_rejection(db: &sqlx::PgPool, principal: &Principal, task_id:
  &str) -> axum::response::Response` (private fn in `work.rs`, used only by `create_gate`).
- Consumes: `crate::routes::authz::domain_id_for_task`, `crate::routes::authz::authz_domain`,
  `crate::routes::authz::authz_task_owner` (existing, `authz.rs:293`, reused unchanged for its
  404/403 semantics), `crate::auth::is_full_trust`, `conflict` (existing), `row_to_gate` (existing).

- [ ] **Step 1: Write the failing tests**

Insert into `crates/edgeplane-tower/tests/test_authz.rs`, between the closing `}` of
`domain_peer_cannot_create_gate_on_foreign_task` at line 719 and the `// ── send_mesh_message sender
anti-spoof` header at line 721 (keep one blank line before the existing header):

```rust

#[tokio::test]
async fn create_gate_succeeds_for_current_claimer_on_running_task() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id =
        common::seed_claimed_task(&pool, &ctx.mission_id, &ctx.domain_id, "agent-A").await;
    let s = server(pool.clone());

    // owner_session_token acts as a full-trust/bypass caller elsewhere in
    // this suite (see fencing_complete_waiting_review_source_status_still_works
    // in test_task_kind_unification.rs) — exercises the bypass arm of the
    // new fenced INSERT's ownership check, not just the claimed_by_agent_id
    // match, since no HTTP-level test previously covered create_gate's
    // success path at all (grep confirms this was the only caller of
    // POST .../gates in the whole suite before this plan).
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/gates"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "gate_type": "review",
            "required_approvals": "1"
        }))
        .await;
    assert_eq!(
        res.status_code(),
        201,
        "creating a gate on a running, gate-attachable task must succeed: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "pending");
    assert_eq!(body["mesh_task_id"], task_id);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reviewgate WHERE mesh_task_id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn create_gate_rejected_when_task_no_longer_gate_attachable() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // A finished task: complete_task's own terminal transition clears
    // claimed_by_agent_id, so seed that same end state directly (status +
    // no claimer) rather than driving it through /complete, matching how
    // sibling fencing tests in test_task_kind_unification.rs seed
    // post-terminal-transition state.
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "finished",
        None,
        1,
    )
    .await;
    let s = server(pool.clone());

    // Bypass caller (owner_session_token) — satisfies the ownership half of
    // the fence unconditionally, isolating this test to the STATUS half:
    // proves the new fence rejects based on current task status even when
    // ownership is not the blocking factor, which the old code (a plain
    // authz_task_owner precheck with no status check at all) could not do
    // — that precheck would have let this request proceed straight to an
    // unconditional INSERT.
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/gates"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "gate_type": "review",
            "required_approvals": "1"
        }))
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "attaching a gate to an already-finished task must be rejected: {}",
        res.text()
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reviewgate WHERE mesh_task_id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "no gate row must have been inserted by the rejected attempt"
    );
}
```

- [ ] **Step 2: Run the tests to verify failures**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(create_gate)'`
Expected: `create_gate_rejected_when_task_no_longer_gate_attachable` FAILs — current code's
`authz_task_owner` precheck only checks identity, never task status, and `owner_session_token` is
bypass, so the precheck passes and the unconditional INSERT succeeds (201 instead of the expected
409). `create_gate_succeeds_for_current_claimer_on_running_task` and the pre-existing
`domain_peer_cannot_create_gate_on_foreign_task` should currently PASS — confirm before proceeding.

- [ ] **Step 3: Rewrite `create_gate`**

Replace `work.rs:1682-1738` with:

```rust
async fn create_gate(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
    Json(body): Json<GateCreate>,
) -> impl IntoResponse {
    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let subject_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
    let gate_id = Uuid::new_v4().to_string();
    let now = Utc::now().naive_utc();

    // Fences the ownership check (Change 10: only the task's claimer, or
    // full-trust/admin, may attach a gate) AND a "still gate-attachable"
    // status check into the INSERT itself, closing the check-then-insert
    // TOCTOU the old separate authz_task_owner precheck left open: a caller
    // who owned the task at check-time but has since lost ownership
    // (reclaimed, completed, cancelled) could otherwise still attach a
    // pending gate. "Gate-attachable" mirrors complete_task's own
    // non-terminal predicate (task_transitions.rs's Complete arm) — a gate
    // only makes sense before the task reaches a status complete_task
    // itself would treat as terminal. See
    // docs/superpowers/plans/2026-08-18-ep1-tower-fencing.md Roadmap,
    // "create_gate is check-then-insert with no fencing on the insert
    // itself".
    let row = sqlx::query(
        "INSERT INTO reviewgate (id, owner_subject, mesh_task_id, run_id, gate_type, \
         required_approvals, status, approval_request_id, ai_pending_action_id, policy_rule_id, \
         created_at, resolved_at) \
         SELECT $1,$2,$3,$4,$5,$6,'pending',$7,NULL,NULL,$8,NULL \
         WHERE EXISTS ( \
           SELECT 1 FROM task \
           WHERE task.id = $3 \
             AND ( \
               (task.kind = 'claimable' AND task.status IN ('claimed','running','waiting_review') \
                AND (task.claimed_by_agent_id = $9 OR $10)) \
               OR \
               (task.kind = 'assigned' AND task.status NOT IN ('done','finished','failed','cancelled') \
                AND (task.owner = $9 OR $10)) \
             ) \
         ) \
         RETURNING *",
    )
    .bind(&gate_id)
    .bind(&principal.subject)
    .bind(&task_id)
    .bind(&body.run_id)
    .bind(&body.gate_type)
    .bind(&body.required_approvals)
    .bind(&body.approval_request_id)
    .bind(now)
    .bind(subject_id)
    .bind(is_bypass)
    .fetch_optional(&state.db)
    .await;

    match row {
        Ok(Some(r)) => (StatusCode::CREATED, Json(row_to_gate(&r))).into_response(),
        Ok(None) => classify_create_gate_rejection(&state.db, &principal, &task_id).await,
        Err(e) => {
            tracing::error!("create_gate: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// After the fenced INSERT rejects (zero rows), classify why with a fresh
/// read: reuses `authz_task_owner` unchanged for "task missing" (404) /
/// "caller isn't the claimer" (403) — those two conditions are exactly what
/// it already checks. A caller who reaches here AND passes
/// `authz_task_owner` must have failed the status half of the fence (the
/// only remaining reason the INSERT's WHERE EXISTS could be false), i.e.
/// the task is no longer in a gate-attachable status.
async fn classify_create_gate_rejection(
    db: &sqlx::PgPool,
    principal: &Principal,
    task_id: &str,
) -> axum::response::Response {
    if let Err(resp) = crate::routes::authz::authz_task_owner(db, principal, task_id, None).await
    {
        return resp;
    }
    conflict("Task is not in a gate-attachable status")
}
```

- [ ] **Step 4: Run all tests in the file**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_authz)'`
Expected: PASS, all of them (including the pre-existing `domain_peer_cannot_create_gate_on_foreign_task`
unchanged).

- [ ] **Step 5: Run the full tower suite (both test files touched by this plan)**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml`
Expected: PASS, full suite, no regressions.

- [ ] **Step 6: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "fix(tower): fence create_gate's INSERT against ownership/status TOCTOU"
```

---

## Self-Review

**1. Spec coverage:**
- Roadmap's `retry_task` entry ("needs `AND status IN ('failed','cancelled')` on the UPDATE itself, not
  just the earlier read") → Task 1, Step 3. Covered.
- Roadmap's `create_gate` entry ("fence the INSERT ... `INSERT ... SELECT ... WHERE EXISTS (task still
  owned by caller AND still in a gate-attachable status)`") → Task 2, Step 3, using exactly that
  `INSERT ... SELECT ... WHERE EXISTS` shape. Covered.
- Global Constraints' explicit scope boundary (no ownership check added to `retry_task`) → stated in
  Global Constraints and in `classify_retry_rejection`'s doc comment. Covered.

**2. Placeholder scan:** No TBD/TODO markers; every step has literal, runnable code and exact run
commands; no "similar to Task N" references (each task's SQL/Rust is written in full).

**3. Type consistency:** `classify_retry_rejection`/`classify_create_gate_rejection` both return
`axum::response::Response`, matching every `match` arm's `.into_response()` calls in their respective
handlers — consistent with `task_transitions.rs`'s `classify_fenced_rejection` naming/shape convention.
`row_to_task`, `row_to_gate`, `not_found`, `conflict`, `domain_id_for_task`, `authz_domain`,
`authz_task_owner` are all pre-existing functions used with their current, verified signatures — none
invented.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-09-04-retry-task-create-gate-fencing.md`. Two
execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast
iteration

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with
checkpoints

**Which approach?**
