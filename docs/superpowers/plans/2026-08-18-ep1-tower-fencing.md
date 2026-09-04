# EP-1 Phase 1: Tower Atomic Fencing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the fencing gap on every mutating task-lifecycle endpoint in `edgeplane-tower` (REST + MCP) — a stale claimer whose lease expired and was reclaimed must never be able to complete/fail/block/progress the task out from under the new claimer.

**Architecture:** Converge every mutating endpoint on the pattern `claim_task` already uses correctly: the ownership/lease/status check moves into the `UPDATE`'s `WHERE` clause (a fenced CAS), and the response is derived from whether the fenced update returned a row, not from an earlier precondition check. A shared `classify_fenced_rejection` helper distinguishes 404 (gone) / 409 (predicate failed but caller showed some ownership proof) / 403 (no proof at all, not full-trust/admin) after a fenced update returns zero rows. `resolve_gate` gets its own bespoke fenced transaction. A new independent periodic sweep (not just `list_tasks`'s side effect) reclaims expired leases tower-wide.

**Tech Stack:** Rust (edition 2024), axum, sqlx (raw `query`, not the `query!` macro — matches this crate's existing convention), Postgres, `axum_test::TestServer` integration tests gated on `TEST_DATABASE_URL`.

**Spec:** `docs/superpowers/specs/2026-08-17-ep1-fenced-dispatch-design.md` §1 only ("Tower: atomic fencing on every mutating task endpoint"). §2 (task_loop.rs), §3 (task_worker.rs unification), and §4 (supervision + drain) are separate, later plans — §1 has no dependency on them and they depend on §1's response-shape changes (409 instead of 403), so this ships first. The one real interlock (§1's `append_progress` lease requirement vs. the daemon's `post_progress` caller) is closed inside this plan's Task 8, not deferred.

## Global Constraints

- `LEASE_TTL_SECS = 120` (`work.rs:467`) — unchanged, do not touch.
- **Never add `kind='claimable'` unconditionally to a query that today serves both kinds.** `complete_task`, `fail_task`, `cancel_task` are deliberately unified across `kind='claimable'` and `kind='assigned'` — express the per-kind precondition as an `OR`-branch inside one `WHERE` clause, never as a blanket kind filter. This was blocker #1 in the spec's first-draft Codex review.
- **Lease freshness, not just token equality:** any predicate that checks `claim_lease_id = $lease` must also assert lease freshness (or rely on `expire_stale_leases` having already cleared the token — but do not assume the sweep ran recently; assert it explicitly). This was blocker #2 in the spec's first-draft Codex review.
- **Bind the freshness check as a parameter — never write SQL `now()` in the predicate.** `task.lease_expires_at`/`updated_at` are `timestamp without time zone`; SQL `now()` returns `timestamptz`. Comparing them forces Postgres to cast the naive column through the connection's session `TimeZone` GUC (`timestamp_ge_timestamptz`), so the fence's correctness silently depends on that session defaulting to UTC — true today (verified live against both the CI/local test Postgres and the production `edgeplane-cnpg` cluster) but asserted nowhere, and any future timezone change (pooler default, a `SET TIME ZONE`, a different base image) flips the predicate: an expired lease can pass, or a live one can be rejected. Fix: bind the already-computed `Utc::now().naive_utc()` value and compare against that placeholder (`lease_expires_at >= $N`), the same pattern `expire_stale_leases` already uses. **Ruling (Task 1 fix loop, adversarial review, retroactively applied to every task below that references `lease_expires_at`):** every occurrence of `lease_expires_at >= now()` in this plan's code blocks was corrected to a bound parameter — if you're implementing from an older copy of this plan, do not transcribe a literal `now()` here.
- **Full-trust/admin bypass is explicit, not implicit:** every fenced predicate that includes an ownership/lease check must also carry `OR $is_bypass`, where `is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin`. Omitting this silently breaks legitimate admin force-operations.
- **403 vs 409 classification rule** (applies everywhere `classify_fenced_rejection` is used): if the caller presented *any* ownership proof — a `claimed_by_agent_id`/`owner` match, or a `claim_lease_id` value at all (even a wrong/stale one) — a failed predicate is `409` (lost a race, not unauthorized). Only a caller who presented zero proof and isn't full-trust/admin gets `403`.
- **Broadcast claims and full-trust/admin bypass are explicitly unfenced by design**, not an oversight — `claim_task`'s broadcast branch (`work.rs:1068`) stays a blind `UPDATE ... WHERE id=$1` after a precheck; this is intentional broadcast semantics and is out of scope for this plan.
- **The broadcast exception applies to every later lifecycle transition, not just the initial claim.** **Ruling (Task 2 review, applies to every task below with a `lease_expires_at` freshness check on the claimable branch — Tasks 1, 2, 3, 8, 9):** `expire_stale_leases` explicitly excludes `claim_policy != 'broadcast'` (`work.rs:549`) and `claim_task` requires `status='ready'` to (re)claim (`work.rs:1100-1107`) — so once a broadcast task's lease lapses, it is never auto-reclaimed and can never be re-claimed either. Pre-fencing, `heartbeat_task`/`complete_task`/`fail_task`/`append_progress` had no freshness check at all, so a broadcast task was always renewable/completable regardless of lease staleness. A bare freshness check with no carve-out therefore wedges every broadcast task that outlives one lease window with **no in-band recovery** — contradicting this same bullet's own principle. ~~Fix: the claimable branch's ownership/lease sub-condition gets `task.claim_policy = 'broadcast'` as a caller-agnostic bypass of the *entire* lease/freshness/lease-id-match clause (not just the lease-id match), mirroring `claim_task`'s own "no single owner for a CAS to protect" reasoning — `authz_domain` (coarse domain membership) still runs first and is unaffected.`~~ **SUPERSEDED (commit 37dca61a, 2026-08-20; doc corrected 2026-08-25 after this exact stale ruling caused Task 8 to reintroduce the bug 37dca61a had already fixed in code — see the SDD ledger's Task 8 second-pass entry):** the "entire clause" framing above was itself the CRITICAL bug — a caller-agnostic bypass of the *entire* clause bypasses OWNERSHIP too, not just freshness, so ANY domain member (no lease, no relation to the task) could complete/fail/heartbeat-hijack/progress-hijack ANY broadcast task. The correct, current fix, in force in every landed task's actual code: ownership (`claim_lease_id = $lease OR $is_bypass`) is **unconditional**; broadcast waives **freshness only** — `(claim_lease_id = $lease OR $is_bypass) AND (claim_policy = 'broadcast' OR lease_expires_at >= $now)`. If you are implementing a task below from this doc's own embedded code blocks, do not transcribe the old "entire clause" shape even where a task's own Step 3 text below still shows it — cross-check against a already-landed sibling endpoint's ACTUAL CURRENT code (e.g. `heartbeat_task`) before writing any broadcast-aware predicate, not just this doc.
- **`complete_task` and `fail_task` specifically also need `claimed_by_agent_id` as a standalone ownership path, unguarded by freshness.** **Ruling (Task 2 review):** `edgeplaned/crates/edgeplaned-bin/src/task_worker.rs` (a real, live, currently-deployed daemon loop, out of scope for this Tower-only plan) calls `/work/tasks/{id}/complete` and `/work/tasks/{id}/fail` with `{"agent_id": agent_id}` only — never a `claim_lease_id`, and never calls `/heartbeat` at all. A lease-only predicate (safe for `heartbeat_task`, whose one real caller — `task_loop.rs` — always carries a lease, independently verified) would reject every completion/failure this live caller performs on a `kind='claimable'` row the moment this plan's Tower changes deploy. `classify_fenced_rejection` already treats a `claimed_by_agent_id` match as valid ownership proof (409, not 403) — the predicate should agree with its own classifier. Fix: `complete_task`'s and `fail_task`'s claimable branches add `OR task.claimed_by_agent_id = $subject_id` as a third, independent path (alongside broadcast and the lease/freshness path), **not gated on `lease_expires_at`** — the state machine already makes this race-safe without an explicit freshness check: a non-broadcast claimable row can't be re-claimed by anyone else until `expire_stale_leases` clears `claimed_by_agent_id` to `NULL`, at which point the identity check stops matching for everyone, including the stale former claimer. This restores exactly the capability `authz_task_owner` already granted before this plan started — no new capability, and `heartbeat_task`/`append_progress`/the MCP mirror do not need this path (their real callers always carry a lease or are unaffected — verify this holds for the MCP mirror specifically when Task 9 is reviewed, since it wasn't directly audited here).
- All new/modified SQL uses `sqlx::query` + `.bind()` + `Row::get`, matching every existing handler in `work.rs`/`mcp.rs` — do not introduce the `sqlx::query!` compile-time-checked macro family, which nothing in this crate uses today.
- Migrations go in `crates/edgeplane-tower/migrations/`, sequential numbering; the next free number is `0015`.
- Test convention for this crate's DB-gated integration tests: match live CI exactly (`.github/workflows/ci.yml:23-37,54`), which uses `cargo nextest`, not plain `cargo test` — a 2026-07-16 plan's precedent documented `cargo test`, but that has since drifted from what CI actually runs; verified directly against the current workflow file, trust that over the older plan. Full-crate form: `TEST_DATABASE_URL=<url> cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml`. Scoped to one test file: add `-E 'binary(<test_binary>)'`; scoped to a name filter within a file: `-E 'test(<substring>)'`. CI's Postgres (`.github/workflows/ci.yml:23-28`) is `postgres:16`, user/pass `postgres`/`postgres`, db `test`, port 5432 — `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test`. Locally, verify a matching Postgres is actually reachable (a real connection attempt — `psql`/`pg_isready` if present, or a raw TCP probe — not just "the env var is set") before running; if none is up, start one ephemerally and matching CI's exact image/creds: `docker run -d --rm --name edgeplane-test-pg -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=test -p 5432:5432 postgres:16`, wait for it to accept connections, then run tests; stop it (`docker stop edgeplane-test-pg`) once the plan's tasks are done, since it's scratch and reversible, not the repo's persistent dev-stack `docker-compose.yml` (a separate compose file, different DB name/creds — do not touch that one for this plan's tests).
- Every task in this plan ends with `cargo clippy --workspace --all-targets -- -D warnings` passing, per this repo's CI gate convention (`.claude/rule-library` / CLAUDE.md "Rust CI gate completeness").
- **Three-test minimum per fenced transition** (ruling: post-Task-5 pattern review, cross-checked with an independent gpt-5.6-terra review of the bug pattern across Tasks 1-5, 2026-08-20). Every task from Task 6 onward writes at least these three, not just the plan's originally-prescribed tests: (1) **stale-actor retry** — the original claimer/actor attempts the operation again after its ownership evidence has been superseded (reclaimed, re-attributed, or cleared by an earlier successful call) and gets the correct classification, not an accidental 403; (2) **concurrent conflicting operation** — two callers race the same transition (or two competing transitions on the same row, e.g. approve vs. reject, unblock vs. cancel) and exactly one wins, atomically, with no window for both to partially apply; (3) **idempotent retry** — the same caller repeats an already-successful call and gets 409, not 403, with attribution/side effects unchanged from the first call. Tasks 1-5 each independently discovered a bug matching one of these three shapes only after a separate adversarial review pass, not from the plan's own originally-prescribed test list — write these proactively instead of waiting for review to find the gap.

---

## File Structure

- Modify `crates/edgeplane-tower/src/routes/work.rs` — `heartbeat_task`, `complete_task`, `fail_task`, `cancel_task`, `block_task`, `unblock_task`, `append_progress`, `resolve_gate`, `expire_stale_leases` (refactored into a shared core), a new `classify_fenced_rejection` helper, a new `run_lease_expiry_sweep` core.
- Modify `crates/edgeplane-tower/src/routes/mcp.rs` — the combined `complete_mesh_task`/`fail_mesh_task`/`block_mesh_task` match arm (`dispatch()`, ~line 791).
- Modify `crates/edgeplane-tower/src/main.rs` — spawn the new periodic sweep task before `build_app` consumes the pool.
- Create `crates/edgeplane-tower/migrations/0015_task_dispatch_covering_index.sql` — replace the `status='claimed'`-only partial index with one covering `claimed` and `running`.
- Modify `crates/edgeplaned/crates/edgeplaned-work/src/task.rs` — `post_progress` gains a required `claim_lease_id: &str` parameter.
- Modify `crates/edgeplaned/crates/edgeplaned-bin/src/task_loop.rs` — the one call site (`task_loop.rs:332`) guards on the lease being present before calling `post_progress`.
- Modify `crates/edgeplane-tower/tests/test_task_kind_unification.rs` — extend with new fencing regression tests; flip one existing assertion from 403→409.

---

### Task 1: Shared fencing-rejection classifier + `heartbeat_task` conversion

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:1004` (add doc comment to `claim_task`'s broadcast branch), `crates/edgeplane-tower/src/routes/work.rs:1175-1258` (`heartbeat_task`)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Produces: `async fn classify_fenced_rejection(db: &sqlx::PgPool, p: &Principal, task_id: &str, lease_id: Option<&str>) -> axum::response::Response` (private fn in `work.rs`, used by Tasks 2-6).
- Consumes: `crate::auth::is_full_trust(p: &Principal) -> bool` (existing, `auth.rs:474`), `not_found`/`conflict` helpers (existing, `work.rs:145-158`).

- [ ] **Step 1: Add the broadcast-exception doc comment (no behavior change)**

In `work.rs`, immediately above the `if claim_policy == "broadcast" {` line inside `claim_task` (`work.rs:1069`):

```rust
    // Broadcast: no locking needed, just update status to running.
    // Intentionally unfenced — broadcast tasks are meant to be claimable by
    // multiple agents simultaneously, so there is no single "owner" for a
    // CAS to protect. This is a deliberate, stated exception to the fencing
    // pattern the rest of this file converges on, not an oversight. See
    // spec §1 "Broadcast claims and full-trust/admin bypass".
    if claim_policy == "broadcast" {
```

- [ ] **Step 2: Write the failing test for the classifier's 409 behavior via `heartbeat_task`**

Add to `crates/edgeplane-tower/tests/test_task_kind_unification.rs`, after `fencing_second_claimer_gets_fresh_lease_original_token_rejected` (after line 518):

```rust
#[tokio::test]
async fn fencing_heartbeat_stale_lease_is_409_not_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        None,
        2,
    )
    .await;
    sqlx::query("UPDATE task SET claim_lease_id='stale-lease', lease_expires_at=now()+interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("seed a live lease");

    // A restricted principal presenting a lease that doesn't match the row's
    // current lease — this is the "wrong owner-with-a-lease-supplied" case
    // from spec §1, which must classify as 409 (a proof was offered, it was
    // just wrong), not 403.
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "not-the-real-lease"}))
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "a caller presenting a (wrong) lease must get 409, not 403: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_heartbeat_no_proof_at_all_is_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-someone-else"),
        2,
    )
    .await;
    sqlx::query("UPDATE task SET claim_lease_id='real-lease', lease_expires_at=now()+interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("seed a live lease");

    // A restricted principal presenting NO lease at all, and not the row's
    // claimed_by_agent_id — zero ownership proof of any kind. Must be 403.
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "a caller with zero ownership proof must get 403: {}",
        res.text()
    );
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_heartbeat)'`
Expected: `fencing_heartbeat_stale_lease_is_409_not_403` FAILs (current code returns 403 for a lease mismatch — see `heartbeat_task`'s existing `StatusCode::FORBIDDEN` branch at `work.rs:1232`). `fencing_heartbeat_no_proof_at_all_is_403` currently PASSes already (existing behavior happens to already be 403 here) — that's fine, it's a regression guard for after the rewrite, not a new-behavior test.

- [ ] **Step 4: Add `classify_fenced_rejection` and rewrite `heartbeat_task`**

In `work.rs`, add the helper immediately after `conflict()` (after line 158, before the `// ── Row helpers` comment at line 167):

```rust
/// After a fenced UPDATE's WHERE clause rejects a caller (zero rows
/// returned), classify why. Mirrors `claim_task`'s `conflict()`-on-`None`
/// pattern but adds the 403 split: a caller who presented no ownership
/// proof at all (no matching `claimed_by_agent_id`/`owner`, no lease
/// supplied) and isn't full-trust/admin gets 403; anyone who presented
/// *some* proof — a stale lease, a real-but-wrong-status claim — gets 409,
/// since from their perspective the request looked legitimate and lost a
/// race, not unauthorized access. See spec §1 "403 vs 409, done correctly".
async fn classify_fenced_rejection(
    db: &sqlx::PgPool,
    p: &Principal,
    task_id: &str,
    lease_id: Option<&str>,
) -> axum::response::Response {
    let row = match sqlx::query("SELECT claimed_by_agent_id, owner FROM task WHERE id=$1")
        .bind(task_id)
        .fetch_optional(db)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("Task not found"),
        Err(e) => {
            tracing::error!("classify_fenced_rejection fetch: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if crate::auth::is_full_trust(p) || p.is_admin {
        return conflict("Task is not in the required state for this transition");
    }
    let claimed: Option<String> = row.get("claimed_by_agent_id");
    let owner: Option<String> = row.get("owner");
    let subject_id = p.subject.strip_prefix("agent:").unwrap_or(&p.subject);
    let owns_directly =
        claimed.as_deref() == Some(subject_id) || owner.as_deref() == Some(subject_id);
    if owns_directly || lease_id.is_some() {
        conflict("Task is not in the required state for this transition")
    } else {
        (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"detail": "not the task's claimer"})),
        )
            .into_response()
    }
}
```

Replace the whole body of `heartbeat_task` (`work.rs:1175-1258`) with:

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

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let now = Utc::now().naive_utc();
    let lease_expires = now + chrono::Duration::seconds(LEASE_TTL_SECS);

    let updated = sqlx::query(
        "UPDATE task SET status='running', lease_expires_at=$2, updated_at=$3 \
         WHERE id=$1 AND kind='claimable' AND status IN ('claimed','running') \
           AND (claim_lease_id = $4 OR $5) \
           AND (claim_policy = 'broadcast' OR lease_expires_at >= $3) \
         RETURNING *",
    )
    .bind(&task_id)
    .bind(lease_expires)
    .bind(now)
    .bind(body.claim_lease_id.as_deref())
    .bind(is_bypass)
    .fetch_optional(&state.db)
    .await;

    match updated {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => {
            classify_fenced_rejection(&state.db, &principal, &task_id, body.claim_lease_id.as_deref())
                .await
        }
        Err(e) => {
            tracing::error!("heartbeat_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

Note: `is_bypass=true` also needs the `lease_expires_at >= now()` check to still pass for a legitimately-live row — an admin heartbeating a task with an *already-expired* lease is a status-precondition failure (the row should have been reclaimed), not something a bypass should paper over silently; this is intentional and matches the spec's "the predicate must also assert `lease_expires_at >= now()` in the same atomic statement" instruction without carving out an exception for `is_bypass`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS, including all pre-existing tests in this file (`assigned_task_cannot_be_heartbeated` must still get 409 — full-trust caller hits `is_full_trust` branch inside `classify_fenced_rejection` since `kind != 'claimable'` fails the WHERE regardless of bypass).

- [ ] **Step 6: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "fix(tower): fence heartbeat_task, add shared 403/409 rejection classifier"
```

---

### Task 2: `complete_task` — fenced CAS + atomic pending-gate transition + 3-field lease clear

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:1359-1499` (`complete_task`)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Consumes: `classify_fenced_rejection` (Task 1).
- Produces: no new public interface; response shape for the terminal-completion path is unchanged (`row_to_task(&r)` + `unblocked_tasks`); the `waiting_review` early-return shape (`{"status": "waiting_review", "pending_gates": [...], "task_id": ...}`) is unchanged.

- [ ] **Step 1: Write the failing tests**

Add to `test_task_kind_unification.rs`:

```rust
#[tokio::test]
async fn fencing_complete_stale_lease_after_reclaim_is_409() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 2)
            .await;
    let claim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"agent_id": "agent-A"}))
        .await;
    let lease_a = claim_res.json::<serde_json::Value>()["claim_lease_id"]
        .as_str()
        .unwrap()
        .to_string();

    sqlx::query("UPDATE task SET lease_expires_at = now() - interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("force lease into the past");
    trigger_reclaim_sweep(&s, &ctx.mission_id, &ctx.owner_session_token).await;

    let complete_res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"claim_lease_id": lease_a}))
        .await;
    assert_eq!(
        complete_res.status_code(),
        409,
        "stale lease A must not complete a reclaimed task: {}",
        complete_res.text()
    );
}

#[tokio::test]
async fn fencing_complete_waiting_review_source_status_still_works() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "waiting_review",
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
    .expect("seed a live lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(
        res.status_code().is_success(),
        "completion from waiting_review must still succeed (full-trust caller): {}",
        res.text()
    );
    assert_eq!(res.json::<serde_json::Value>()["status"], "finished");
}

#[tokio::test]
async fn fencing_complete_kind_assigned_still_works_after_predicate_split() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id =
        common::seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, &ctx.owner_session_subject())
            .await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(
        res.status_code().is_success(),
        "kind='assigned' completion must still work after the predicate split: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_complete_terminal_transition_clears_claimed_by_agent_id() {
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
    .expect("seed a live lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());

    let row = sqlx::query("SELECT claimed_by_agent_id, claim_lease_id, lease_expires_at FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        row.get::<Option<String>, _>("claimed_by_agent_id").is_none(),
        "complete_task must clear claimed_by_agent_id, not just the lease fields (spec §1 third-pass correction)"
    );
    assert!(row.get::<Option<String>, _>("claim_lease_id").is_none());
    assert!(row
        .get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at")
        .is_none());
}

#[tokio::test]
async fn fencing_complete_pending_gate_race_is_closed() {
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
    .expect("seed a live lease");
    // A pending gate exists BEFORE the complete call — proves the CTE sees a
    // gate created concurrently with (here, just before) the completion
    // attempt, not a stale pre-fetched view.
    sqlx::query(
        "INSERT INTO reviewgate (id, owner_subject, mesh_task_id, run_id, gate_type, \
         required_approvals, status, created_at) \
         VALUES ($1, 'harness', $2, NULL, 'manual', 'any', 'pending', now())",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("seed pending gate");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["status"], "waiting_review",
        "a pending gate must route completion to waiting_review, atomically: {body}"
    );
    assert_eq!(body["pending_gates"].as_array().unwrap().len(), 1);

    let row_status: String = sqlx::query_scalar("SELECT status FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row_status, "waiting_review");
}
```

`fencing_complete_kind_assigned_still_works_after_predicate_split` needs `ctx.owner_session_subject()` — `Ctx` doesn't have this today. Add it to `common/mod.rs`'s `Ctx` impl (or inline the known subject string if `setup()` already fixes it — check first): read `crates/edgeplane-tower/tests/common/mod.rs`'s `setup()` (around line 532) to find what subject the owner session was minted with, and either add a `pub owner_subject: String` field to `Ctx` populated from that same value, or bind the assigned task's `owner` column directly to that literal. Prefer adding the field — other future tests will want it too, and duplicating the literal string invites drift.

- [ ] **Step 2: Run to verify failures**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_complete)'`
Expected: `fencing_complete_stale_lease_after_reclaim_is_409` FAILs (current code returns 403 via the `authz_task_owner` precheck at `work.rs:1394-1403`, matching the pre-existing `fencing_second_claimer_gets_fresh_lease_original_token_rejected` test's still-403 assertion at the analogous call — that older test asserts 403 today and will be updated in Step 4 below). `fencing_complete_terminal_transition_clears_claimed_by_agent_id` FAILs (current terminal `UPDATE` at `work.rs:1473-1476` never touches `claimed_by_agent_id`). The other three should currently PASS (regression guards).

- [ ] **Step 3: Rewrite `complete_task`**

Replace `work.rs:1359-1499` with:

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

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let subject_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
    let now = Utc::now().naive_utc();
    let now_tz = Utc::now();
    // result_artifact_id is now `integer` (matches artifact.id) — was varchar.
    let result_artifact_id: Option<i32> = body
        .result_artifact_id
        .as_deref()
        .and_then(|s| s.parse::<i32>().ok());

    // Fences the ownership/lease/status predicate AND the pending-gate check
    // in one statement, closing the race where a gate is created between a
    // separate SELECT and the transition UPDATE (spec §1, complete_task
    // pending-gates subsection). `gate_check.has_pending` is computed fresh
    // inside this same statement, so there is no window for a concurrently
    // created gate to be missed.
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
    .bind(&task_id)
    .bind(result_artifact_id)
    .bind(now_tz)
    .bind(now)
    .bind(body.claim_lease_id.as_deref())
    .bind(is_bypass)
    .bind(subject_id)
    .fetch_optional(&state.db)
    .await;

    let r = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return classify_fenced_rejection(
                &state.db,
                &principal,
                &task_id,
                body.claim_lease_id.as_deref(),
            )
            .await;
        }
        Err(e) => {
            tracing::error!("complete_task update: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let has_pending: bool = r.get("has_pending");
    if has_pending {
        let gate_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM reviewgate WHERE mesh_task_id=$1 AND status='pending'",
        )
        .bind(&task_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        return Json(serde_json::json!({
            "status": "waiting_review",
            "pending_gates": gate_ids,
            "task_id": task_id,
        }))
        .into_response();
    }

    let mission_id: String = r.get("mission_id");
    let domain_id: String = r.get("domain_id");
    let unblocked = unblock_dependents(&state.db, &mission_id, &task_id).await;
    for tid in &unblocked {
        broadcast_task_available(&domain_id, &mission_id, tid).await;
    }
    let mut val = row_to_task(&r);
    val["unblocked_tasks"] = serde_json::json!(unblocked);
    Json(val).into_response()
}
```

Note the two different `now` bindings: `$3` (finalized_at) uses the `DateTime<Utc>` (`now_tz`, matching the column's `timestamptz` type per the original code's `finalized_at=$4` bind of `now_tz`), `$4` (updated_at) uses `NaiveDateTime` (`now`, matching every other `updated_at` bind in this file). Keep them distinct — swapping them is a type mismatch sqlx will catch at runtime (not compile time, since these are untyped `query()` calls), so get it right the first time.

- [ ] **Step 4: Flip the old 403 assertion to 409, per spec's explicit instruction**

In `test_task_kind_unification.rs`, `fencing_second_claimer_gets_fresh_lease_original_token_rejected` (line ~512-517), change:

```rust
    assert_eq!(
        complete_res.status_code(),
        403,
        "stale original lease A must not complete a task now claimed under lease B: {}",
        complete_res.text()
    );
```

to:

```rust
    assert_eq!(
        complete_res.status_code(),
        409,
        "stale original lease A must not complete a task now claimed under lease B \
         (409, not 403 — the caller presented a lease, so this is a lost race, not \
         unauthorized access; spec §1 '403 vs 409, done correctly'): {}",
        complete_res.text()
    );
```

- [ ] **Step 5: Run all tests in the file**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS, all of them.

- [ ] **Step 6: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "fix(tower): fence complete_task, close pending-gate race, clear claimed_by_agent_id on terminal transition"
```

---

### Task 3: `fail_task` — fenced CAS + 3-field lease clear

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:1501-1586` (`fail_task`)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Consumes: `classify_fenced_rejection` (Task 1).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn fencing_fail_terminal_transition_clears_all_three_lease_fields() {
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
    .expect("seed a live lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"error": "boom"}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());

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

#[tokio::test]
async fn fencing_fail_stale_lease_after_reclaim_is_409() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 2)
            .await;
    let claim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"agent_id": "agent-A"}))
        .await;
    let lease_a = claim_res.json::<serde_json::Value>()["claim_lease_id"]
        .as_str()
        .unwrap()
        .to_string();
    sqlx::query("UPDATE task SET lease_expires_at = now() - interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("force lease into the past");
    trigger_reclaim_sweep(&s, &ctx.mission_id, &ctx.owner_session_token).await;

    let fail_res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"claim_lease_id": lease_a, "error": "boom"}))
        .await;
    assert_eq!(fail_res.status_code(), 409, "{}", fail_res.text());
}
```

- [ ] **Step 2: Run to verify failures**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_fail)'`
Expected: both FAIL against current code (current terminal update at `work.rs:1570-1573` doesn't clear `claimed_by_agent_id`; current 403 precheck at `work.rs:1533-1542`).

- [ ] **Step 3: Rewrite `fail_task`**

Replace `work.rs:1501-1586` with:

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

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let subject_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
    let now = Utc::now().naive_utc();
    let now_tz = Utc::now();

    let updated = sqlx::query(
        "UPDATE task SET status='failed', lease_expires_at=NULL, claim_lease_id=NULL, \
         claimed_by_agent_id=NULL, finalized_at=$3, updated_at=$2 \
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
    .bind(&task_id)
    .bind(now)
    .bind(now_tz)
    .bind(body.claim_lease_id.as_deref())
    .bind(is_bypass)
    .bind(subject_id)
    .fetch_optional(&state.db)
    .await;

    match updated {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => {
            classify_fenced_rejection(&state.db, &principal, &task_id, body.claim_lease_id.as_deref())
                .await
        }
        Err(e) => {
            tracing::error!("fail_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

- [ ] **Step 4: Run all tests in the file**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "fix(tower): fence fail_task, clear claimed_by_agent_id on terminal transition"
```

---

### Task 4: `cancel_task` — fenced CAS, dual-kind

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:875-946` (`cancel_task`)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Consumes: `classify_fenced_rejection` (Task 1). Note: `cancel_task` never receives a `claim_lease_id` from its caller (no body extractor today) — its ownership predicate is `claimed_by_agent_id`/`owner` match or bypass only, no lease path. Pass `None` to `classify_fenced_rejection`'s `lease_id` param accordingly.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn fencing_cancel_non_owner_restricted_caller_is_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
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
        "a restricted caller with no claim on the task must get 403 on cancel: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_cancel_already_terminal_is_409() {
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
        .post(&format!("/api/work/tasks/{task_id}/cancel"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "cancelling an already-finished task must be 409: {}",
        res.text()
    );
}
```

- [ ] **Step 2: Run to verify failures**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_cancel)'`
Expected: both currently PASS against the unfenced code too (current `authz_task_owner` precheck already returns 403 for non-owners, and the status check already returns 409 for `finished`/`cancelled`) — these are regression guards proving the fenced rewrite preserves existing correct behavior, not new-behavior tests. Confirm they pass before AND after Step 3; the point of running now is to establish the pre-change baseline.

- [ ] **Step 3: Rewrite `cancel_task`**

Replace `work.rs:875-946` with:

```rust
async fn cancel_task(
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

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let subject_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
    let now = Utc::now().naive_utc();

    // Cancellation is a mode-agnostic terminal transition, same as
    // complete/fail — an assigned task's owner should be able to cancel it
    // too. NOTE (unchanged from pre-fencing code): this is a narrowing
    // behavior change for kind='claimable' rows — cancel_task previously had
    // no per-task ownership check at all (any domain member could cancel any
    // claimable task). It is now claimer-or-full-trust/admin like its
    // siblings.
    let updated = sqlx::query(
        "UPDATE task SET status='cancelled', claimed_by_agent_id=NULL, \
         lease_expires_at=NULL, claim_lease_id=NULL, updated_at=$2 \
         WHERE id=$1 \
           AND ( \
             (kind = 'claimable' AND status NOT IN ('finished','cancelled') \
              AND (claimed_by_agent_id = $3 OR $4)) \
             OR \
             (kind = 'assigned' AND status NOT IN ('done','finished','failed','cancelled') \
              AND (owner = $3 OR $4)) \
           ) \
         RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .bind(subject_id)
    .bind(is_bypass)
    .fetch_optional(&state.db)
    .await;

    match updated {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => classify_fenced_rejection(&state.db, &principal, &task_id, None).await,
        Err(e) => {
            tracing::error!("cancel_task update: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

Note: the original claimable-side status check was `status == "finished" || status == "cancelled"` → conflict (i.e. only those two block cancellation — a `failed` task, for instance, WAS cancellable pre-fencing). Preserve that exact set (`NOT IN ('finished','cancelled')`), not `is_terminal_status`'s broader set — don't silently narrow what's cancellable.

- [ ] **Step 4: Run all tests in the file**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "fix(tower): fence cancel_task"
```

---

### Task 5: `block_task` — net-new fenced precondition + lease release

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:1588-1635` (`block_task`)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Consumes: `classify_fenced_rejection` (Task 1).
- Design decision (stated, not silent): `block_task` is scoped to `kind='claimable'` only in the fenced predicate — mirroring `retry_task`'s existing precedent (`work.rs:974-979`) that `'ready'`/claimable-pool-adjacent status transitions reject `kind='assigned'` rows. Pre-fencing, `block_task` had zero kind-gating (a latent gap, not a deliberate design), so this narrows behavior for any caller that was (incorrectly) calling `block_task` on an assigned-kind row. Per spec: **block releases the lease** — clears `claim_lease_id`/`lease_expires_at`/`claimed_by_agent_id` so the task re-enters the claimable pool rather than pinning it to a claimer that may never return.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn fencing_block_wrong_status_is_409() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 1)
            .await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "block_task must reject a task that's not claimed/running (previously had NO precondition at all): {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_block_releases_the_lease() {
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
    .expect("seed a live lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());

    let row = sqlx::query(
        "SELECT status, claimed_by_agent_id, claim_lease_id, lease_expires_at FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), "blocked");
    assert!(
        row.get::<Option<String>, _>("claimed_by_agent_id").is_none(),
        "block releases the lease so the task re-enters the claimable pool (spec §1 decision)"
    );
    assert!(row.get::<Option<String>, _>("claim_lease_id").is_none());
    assert!(row
        .get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at")
        .is_none());
}
```

- [ ] **Step 2: Run to verify failures**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_block)'`
Expected: both FAIL (current `block_task` has no status precondition at all and never clears lease fields).

- [ ] **Step 3: Rewrite `block_task`**

Replace `work.rs:1588-1635` with:

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

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let subject_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
    let now = Utc::now().naive_utc();

    // Block releases the lease (clears claim_lease_id/lease_expires_at/
    // claimed_by_agent_id), same as a terminal transition — a blocked task
    // re-enters the claimable pool rather than pinning it to a claimer that
    // may never return. Spec §1 "block/unblock: lease retention" decision.
    // kind='claimable' only — mirrors retry_task's precedent for
    // claimable-pool-adjacent status transitions.
    let updated = sqlx::query(
        "UPDATE task SET status='blocked', claimed_by_agent_id=NULL, \
         lease_expires_at=NULL, claim_lease_id=NULL, updated_at=$2 \
         WHERE id=$1 AND kind='claimable' AND status IN ('claimed','running') \
           AND (claimed_by_agent_id = $3 OR $4) \
         RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .bind(subject_id)
    .bind(is_bypass)
    .fetch_optional(&state.db)
    .await;

    match updated {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => classify_fenced_rejection(&state.db, &principal, &task_id, None).await,
        Err(e) => {
            tracing::error!("block_task: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

- [ ] **Step 4: Run all tests in the file**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "fix(tower): add net-new fenced precondition to block_task, release lease on block"
```

---

### Task 6: `unblock_task` — net-new fenced precondition

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:1714-1757` (`unblock_task`)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Consumes: `classify_fenced_rejection` (Task 1).
- Design decision: kind-gated to `claimable` only, same rationale as Task 5 — `unblock_task` writes `status='ready'`, and `retry_task`'s own comment (`work.rs:974`) establishes `'ready'` as claimable-pool-only vocabulary.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn fencing_unblock_wrong_source_status_is_409() {
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

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "unblock_task must reject a task that isn't blocked (previously had NO source-status guard): {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_unblock_assigned_kind_is_409() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id =
        common::seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    sqlx::query("UPDATE task SET status='blocked' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .unwrap();

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "'ready' is claimable-pool-only vocabulary (retry_task precedent) — unblock must reject kind='assigned': {}",
        res.text()
    );
}
```

- [ ] **Step 2: Run to verify failures**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_unblock)'`
Expected: both FAIL (current `unblock_task` has no status or kind precondition).

- [ ] **Step 3: Rewrite `unblock_task`**

Replace `work.rs:1714-1757` with:

```rust
async fn unblock_task(
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

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let subject_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
    let now = Utc::now().naive_utc();

    // kind='claimable' only — 'ready' is claimable-pool-only vocabulary
    // (retry_task precedent, work.rs:974). Symmetric with block_task's
    // ownership predicate (claimed_by_agent_id or bypass; no lease path,
    // this endpoint has never accepted a lease param).
    let updated = sqlx::query(
        "UPDATE task SET status='ready', updated_at=$2 \
         WHERE id=$1 AND kind='claimable' AND status='blocked' \
           AND (claimed_by_agent_id = $3 OR $4) \
         RETURNING *",
    )
    .bind(&task_id)
    .bind(now)
    .bind(subject_id)
    .bind(is_bypass)
    .fetch_optional(&state.db)
    .await;

    match updated {
        Ok(Some(r)) => Json(row_to_task(&r)).into_response(),
        Ok(None) => classify_fenced_rejection(&state.db, &principal, &task_id, None).await,
        Err(e) => {
            tracing::error!("unblock_task: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

- [ ] **Step 4: Run all tests in the file**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "fix(tower): add net-new fenced precondition to unblock_task"
```

---

### Task 7: `resolve_gate` — bespoke fenced transaction

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:1916-2041` (`resolve_gate`)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Does NOT use `classify_fenced_rejection` — the caller resolving the gate is the *gate's* owner, not necessarily the task's lease holder, per spec. Keeps its own existing gate-ownership check, fences the gate `UPDATE` and the task transition instead.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn fencing_resolve_gate_approved_clears_all_three_lease_fields() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "waiting_review",
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
    let gate_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO reviewgate (id, owner_subject, mesh_task_id, run_id, gate_type, \
         required_approvals, status, created_at) \
         VALUES ($1, $2, $3, NULL, 'manual', 'any', 'pending', now())",
    )
    .bind(&gate_id)
    .bind(&ctx.owner_session_subject())
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());

    let row = sqlx::query(
        "SELECT status, claimed_by_agent_id, claim_lease_id, lease_expires_at FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), "finished");
    assert!(
        row.get::<Option<String>, _>("claimed_by_agent_id").is_none(),
        "resolve_gate's approval path must clear claimed_by_agent_id too (spec §1 third-pass correction)"
    );
    assert!(row.get::<Option<String>, _>("claim_lease_id").is_none());
    assert!(row
        .get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at")
        .is_none());
}

#[tokio::test]
async fn fencing_resolve_gate_rejected_clears_all_three_lease_fields() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "waiting_review",
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
    let gate_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO reviewgate (id, owner_subject, mesh_task_id, run_id, gate_type, \
         required_approvals, status, created_at) \
         VALUES ($1, $2, $3, NULL, 'manual', 'any', 'pending', now())",
    )
    .bind(&gate_id)
    .bind(&ctx.owner_session_subject())
    .bind(&task_id)
    .execute(&pool)
    .await
    .unwrap();

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "rejected"}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());

    let row = sqlx::query(
        "SELECT status, claimed_by_agent_id, claim_lease_id, lease_expires_at FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), "failed");
    assert!(
        row.get::<Option<String>, _>("claimed_by_agent_id").is_none(),
        "resolve_gate's rejection path currently clears NOTHING — spec §1 finding"
    );
    assert!(row.get::<Option<String>, _>("claim_lease_id").is_none());
    assert!(row
        .get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at")
        .is_none());
}
```

(`ctx.owner_session_subject()` is the same `Ctx` field/method added in Task 2 Step 1 — if Task 2 landed first, it already exists; if executing tasks out of order, add it here instead.)

- [ ] **Step 2: Run to verify failures**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_resolve_gate)'`
Expected: `fencing_resolve_gate_approved_clears_all_three_lease_fields` FAILs on the `claimed_by_agent_id` assertion (current approved-path UPDATE at `work.rs:2024-2030` clears `lease_expires_at` only). `fencing_resolve_gate_rejected_clears_all_three_lease_fields` FAILs entirely (current rejected-path UPDATE at `work.rs:2018-2022` clears nothing).

- [ ] **Step 3: Rewrite `resolve_gate`'s task-transition section**

`resolve_gate`'s gate-ownership check and `reviewgate` UPDATE (`work.rs:1936-1982`) are unchanged — that part isn't in the fencing table (gate ownership isn't task-lease ownership). Replace only the task-transition block (`work.rs:1998-2038`, from `let task_status: String = task_row.get("status");` through the closing of the `if task_status == "waiting_review"` block) with:

```rust
    let task_status: String = task_row.get("status");
    let mission_id: String = task_row.get("mission_id");

    if task_status == "waiting_review" {
        let all_gates = sqlx::query("SELECT status FROM reviewgate WHERE mesh_task_id=$1")
            .bind(&task_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

        let any_rejected = all_gates
            .iter()
            .any(|r| r.get::<String, _>("status") == "rejected");
        let all_resolved = all_gates.iter().all(|r| {
            let s: String = r.get("status");
            s == "approved" || s == "expired"
        });

        if any_rejected {
            // Fenced: only transition if still waiting_review — a second
            // concurrent resolve_gate call transitioning this same task
            // must not double-fire finalized_at / dependents.
            let _ = sqlx::query(
                "UPDATE task SET status='failed', lease_expires_at=NULL, claim_lease_id=NULL, \
                 claimed_by_agent_id=NULL, updated_at=$2 \
                 WHERE id=$1 AND status='waiting_review' RETURNING id",
            )
            .bind(&task_id)
            .bind(now)
            .fetch_optional(&state.db)
            .await;
        } else if all_resolved {
            let updated = sqlx::query(
                "UPDATE task SET status='finished', lease_expires_at=NULL, claim_lease_id=NULL, \
                 claimed_by_agent_id=NULL, updated_at=$2 \
                 WHERE id=$1 AND status='waiting_review' RETURNING id",
            )
            .bind(&task_id)
            .bind(now)
            .fetch_optional(&state.db)
            .await;
            if matches!(updated, Ok(Some(_))) {
                let domain_id: String = task_row.get("domain_id");
                let unblocked = unblock_dependents(&state.db, &mission_id, &task_id).await;
                for tid in &unblocked {
                    broadcast_task_available(&domain_id, &mission_id, tid).await;
                }
            }
        }
        // else: some still pending, leave as waiting_review
    }

    Json(gate_val).into_response()
}
```

This is deliberately a lighter fence than the CAS-with-CTE pattern used elsewhere: the gate row itself was already atomically transitioned out of `pending` by the earlier `UPDATE reviewgate ... WHERE status='pending'` (unchanged code above this block), so only one caller can ever reach the `any_rejected`/`all_resolved` recompute for a given gate resolution. The `AND status='waiting_review'` guard on the task UPDATE is a defense-in-depth CAS against a second gate's resolution racing the same task (spec's "recomputes remaining-gate state in the same transaction so a second gate created concurrently isn't missed" — the recompute here re-reads `all_gates` fresh each time a gate resolves, and the guard ensures only the resolution that actually observes `all_resolved`/`any_rejected` true while the task is still `waiting_review` gets to transition it).

- [ ] **Step 4: Run all tests in the file**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "fix(tower): fence resolve_gate's task transitions, clear claimed_by_agent_id on both outcomes"
```

---

### Task 8: `append_progress` — required lease, fenced insert + daemon-side interlock fix

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:353-362` (`ProgressCreate` struct), `work.rs:1260-1357` (`append_progress`)
- Modify: `crates/edgeplaned/crates/edgeplaned-work/src/task.rs:106-123` (`post_progress`)
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/task_loop.rs:332` (the one call site)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`, `crates/edgeplaned/crates/edgeplaned-work/src/task.rs`'s existing `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `ProgressCreate.claim_lease_id: String` (new required field — this crate's `ProgressCreate` is already extracted via the strict `Json<ProgressCreate>` extractor, not `Option<Json<...>>`, so a missing field 422s automatically via serde, no new validation code needed for "missing" — only for "present but empty").
- Produces: `pub async fn post_progress(client: &BackendClient, task_id: &str, event: &ProgressEvent, claim_lease_id: &str) -> Result<()>` (signature change — `claim_lease_id` becomes a required 4th param, was absent entirely).
- **This is the one real cross-crate interlock the spec flags**: making the tower's lease required and the daemon's client send one must land together, in this task, not split across plans — otherwise the daemon's only progress-posting call site starts getting rejected the moment tower deploys ahead of the daemon.

- [ ] **Step 1: Write the failing tower-side test**

```rust
#[tokio::test]
async fn fencing_progress_requires_lease_now() {
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

    // No claim_lease_id in the body at all — must be rejected now (was
    // previously accepted with zero lease field).
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "no lease"}))
        .await;
    assert_eq!(
        res.status_code(),
        422,
        "progress without claim_lease_id must be rejected: {}",
        res.text()
    );

    // Correct lease — must succeed.
    let ok_res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "with lease", "claim_lease_id": "lease-a"}))
        .await;
    assert!(ok_res.status_code().is_success(), "{}", ok_res.text());

    // Stale/wrong lease — must be rejected.
    let bad_res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "wrong lease", "claim_lease_id": "not-it"}))
        .await;
    assert!(
        !bad_res.status_code().is_success(),
        "progress with a stale lease must be rejected: {}",
        bad_res.text()
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_progress_requires_lease_now)'`
Expected: FAIL on the first assertion (`res.status_code()` is currently 200/201, not 422 — no lease field exists yet).

- [ ] **Step 3: Add the required field and fence the insert**

In `work.rs`, change the `ProgressCreate` struct (`work.rs:353-362`):

```rust
#[derive(serde::Deserialize)]
struct ProgressCreate {
    event_type: String,
    phase: Option<String>,
    step: Option<String>,
    #[serde(default)]
    summary: String,
    #[serde(default = "default_input_json")]
    payload_json: String,
    agent_run_id: Option<String>,
    claim_lease_id: String,
}
```

Replace the body of `append_progress` (`work.rs:1260-1357`) with:

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

    let is_bypass = crate::auth::is_full_trust(&principal) || principal.is_admin;
    let now = Utc::now().naive_utc();
    let agent_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject).to_string();

    // Fenced insert: verify the lease/status eligibility and insert in one
    // statement — no separate precheck, so there is no TOCTOU window between
    // checking eligibility and writing the event (spec §1 append_progress).
    // The seq computation (COALESCE(MAX(seq),-1)+1) still races two
    // concurrent posts against the same task the same way it did before this
    // change — that pre-existing race is out of EP-1's scope.
    let row = sqlx::query(
        "WITH eligible AS ( \
           SELECT 1 FROM task WHERE id=$1 AND kind='claimable' AND status IN ('claimed','running') \
             AND (claim_lease_id = $2 OR $3) \
             AND (claim_policy = 'broadcast' OR lease_expires_at >= $10) \
         ) \
         INSERT INTO meshprogressevent \
           (task_id, agent_id, seq, event_type, phase, step, summary, payload_json, occurred_at, agent_run_id) \
         SELECT $1, $4, (SELECT COALESCE(MAX(seq), -1) + 1 FROM meshprogressevent WHERE task_id=$1), \
                $5, $6, $7, $8, $9, $10, $11 \
         WHERE EXISTS (SELECT 1 FROM eligible) \
         RETURNING *",
    )
    .bind(&task_id)
    .bind(&body.claim_lease_id)
    .bind(is_bypass)
    .bind(&agent_id)
    .bind(&body.event_type)
    .bind(&body.phase)
    .bind(&body.step)
    .bind(&body.summary)
    .bind(&body.payload_json)
    .bind(now)
    .bind(&body.agent_run_id)
    .fetch_optional(&state.db)
    .await;

    match row {
        Ok(Some(r)) => Json(serde_json::json!({
            "id": r.get::<i32, _>("id"),
            "task_id": r.get::<String, _>("task_id"),
            "agent_id": r.get::<String, _>("agent_id"),
            "seq": r.get::<i32, _>("seq"),
            "event_type": r.get::<String, _>("event_type"),
            "phase": r.get::<Option<String>, _>("phase"),
            "step": r.get::<Option<String>, _>("step"),
            "summary": r.get::<String, _>("summary"),
            "payload_json": serde_json::from_str::<serde_json::Value>(r.get::<&str, _>("payload_json")).unwrap_or(serde_json::json!({})),
            "occurred_at": r.get::<chrono::NaiveDateTime, _>("occurred_at"),
            "agent_run_id": r.get::<Option<String>, _>("agent_run_id"),
        }))
        .into_response(),
        Ok(None) => {
            classify_fenced_rejection(&state.db, &principal, &task_id, Some(&body.claim_lease_id)).await
        }
        Err(e) => {
            tracing::error!("append_progress: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

This drops the old separate `kind` fetch + `if kind != "claimable"` early-return — the fenced `eligible` CTE already requires `kind='claimable'`, and `classify_fenced_rejection` doesn't need `kind` to classify 403 vs 409 (a `kind='assigned'` row simply has no `owner`/`claimed_by_agent_id` match for a restricted caller in the common case, or is full-trust and gets the generic conflict message — either way the response is correct, just without the specific "(kind='assigned')" wording the old `conflict()` call had; that wording loss is acceptable since the test suite only asserts status codes here).

- [ ] **Step 4: Fix the daemon-side interlock — `post_progress`**

In `crates/edgeplaned/crates/edgeplaned-work/src/task.rs`, replace `post_progress` (lines 106-123):

```rust
/// Post a typed progress event. `claim_lease_id` is now required by the
/// tower (spec EP-1 §1) — the caller must hold a live lease to post
/// progress.
pub async fn post_progress(
    client: &BackendClient,
    task_id: &str,
    event: &edgeplaned_core::progress::ProgressEvent,
    claim_lease_id: &str,
) -> Result<()> {
    use serde_json::json;
    let body = json!({
        "event_type": event.event_type.to_string(),
        "phase": event.phase,
        "step": event.step,
        "summary": event.summary,
        "payload_json": event.payload.to_string(),
        "claim_lease_id": claim_lease_id,
    });
    client
        .raw_post(&format!("/work/tasks/{task_id}/progress"), &body)
        .await?;
    Ok(())
}
```

- [ ] **Step 5: Fix the one call site — `task_loop.rs:332`**

In `crates/edgeplaned/crates/edgeplaned-bin/src/task_loop.rs`, replace the progress-posting line inside `stream_and_heartbeat` (line 332):

```rust
        if let Some(lease) = claim_lease_id {
            if let Err(e) = task::post_progress(client, task_id, &event, lease).await {
                tracing::warn!("Progress post failed: {e}");
            }
        } else {
            tracing::warn!(
                "no lease held, skipping progress post for task {task_id} (event {:?})",
                event.event_type
            );
        }
```

This is a defensive guard, not a fix to the broader "task_worker/task_loop should abort before injecting a task with no lease" rule — that rule is §2/§3 scope (a separate, later plan). This guard just means the one call site this plan touches degrades to "log and skip" instead of "send a request the tower will now reject," preserving `stream_and_heartbeat`'s existing best-effort, non-fatal progress-posting semantics (the pre-existing code already only logged a warning on `post_progress` failure, never propagated it).

- [ ] **Step 6: Update daemon-side unit tests for the new signature**

Read `crates/edgeplaned/crates/edgeplaned-work/src/task.rs`'s `#[cfg(test)] mod tests` (starts line 228) for any existing test that calls `post_progress` directly, and update its call site to pass a lease string (e.g. `"test-lease"`) and assert the mock server's captured request body includes `"claim_lease_id": "test-lease"`. If no existing test calls `post_progress` directly, add one modeled on the file's existing `wiremock`-based test pattern (see `mesh_task_json` helper and neighboring tests in the same `mod tests` block for the harness shape) — assert a POST to `/work/tasks/{id}/progress` with a JSON body containing the lease field.

- [ ] **Step 7: Run both crates' test suites**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Run: `cargo nextest run --manifest-path crates/edgeplaned/crates/edgeplaned-work/Cargo.toml`
Expected: both PASS.

- [ ] **Step 8: Clippy across both crates**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 9: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs \
        crates/edgeplaned/crates/edgeplaned-work/src/task.rs \
        crates/edgeplaned/crates/edgeplaned-bin/src/task_loop.rs
git commit -m "fix(tower,daemon): require+fence claim_lease_id on progress events, thread it through the daemon's only caller"
```

---

### Task 9: MCP mirror — `complete_mesh_task` / `fail_mesh_task` / `block_mesh_task`

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs:791-864` (the combined match arm inside `dispatch()`)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Consumes: nothing from `work.rs` (MCP has its own JSON `ok`/`error` response shape, no HTTP status codes, no `classify_fenced_rejection` reuse — per spec, "MCP's error classification needs its own explicit (if parallel) treatment").
- Produces: on rejection, `err_result("task_not_found")` / `err_result("lease_conflict")` / `err_result("forbidden")` — three distinguishable string codes (was a single generic `err_result("mesh_task_not_found")` for every non-success case before).

- [ ] **Step 1: Confirm the existing regression test still asserts the right thing**

`fencing_stale_lease_cannot_complete_task_after_reclaim_via_mcp` (`test_task_kind_unification.rs:339-426`) already asserts `complete_body["ok"] == false` on a stale-lease MCP completion attempt — this must keep passing after the rewrite (it's the exact scenario this task fixes for real, not a new assertion). No new test file changes needed for the core exploit-path regression; add one narrower test for the new error-code granularity:

```rust
#[tokio::test]
async fn fencing_mcp_block_mesh_task_gets_new_precondition() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 1)
            .await;

    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "block_mesh_task",
            "args": { "task_id": task_id }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["ok"], false,
        "block_mesh_task on a 'ready' (not claimed/running) task must now be rejected \
         (previously had no precondition at all, matching REST block_task's pre-fix gap): {body}"
    );
    assert_eq!(body["error"], "lease_conflict");
}
```

- [ ] **Step 2: Run to verify the new test fails**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(fencing_mcp_block)'`
Expected: FAIL (current handler has no precondition for `block_mesh_task`, so this currently succeeds with `ok: true`).

- [ ] **Step 3: Rewrite the combined match arm**

Replace `mcp.rs:791-864` with:

```rust
        "complete_mesh_task" | "fail_mesh_task" | "block_mesh_task" => {
            let task_id = str_arg(args, "task_id");
            if task_id.is_empty() {
                return err_result("task_id is required");
            }
            let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
                Ok(d) => d,
                Err(_) => return err_result("task_not_found"),
            };
            if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
                return e;
            }

            let is_bypass = crate::auth::is_full_trust(principal) || principal.is_admin;
            let subject_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
            let lease_str = str_arg(args, "claim_lease_id");
            let lease_opt: Option<&str> = if lease_str.is_empty() { None } else { Some(lease_str.as_str()) };

            let (new_status, source_statuses): (&str, &[&str]) = match tool {
                "complete_mesh_task" => ("finished", &["claimed", "running", "waiting_review"]),
                "fail_mesh_task" => ("failed", &["claimed", "running", "waiting_review"]),
                // block_mesh_task previously had NO status precondition at all —
                // this is net-new, matching REST block_task's fix (spec §1).
                "block_mesh_task" => ("blocked", &["claimed", "running"]),
                _ => return err_result("unknown_tool"),
            };
            let now_tz = Utc::now();
            let now = Utc::now().naive_utc();

            let row = sqlx::query(
                "UPDATE task SET status=$2, updated_at=NOW(), \
                 claim_lease_id=CASE WHEN $2 IN ('finished','failed','cancelled','blocked') THEN NULL ELSE claim_lease_id END, \
                 claimed_by_agent_id=CASE WHEN $2 IN ('finished','failed','cancelled','blocked') THEN NULL ELSE claimed_by_agent_id END, \
                 finalized_at=CASE WHEN $2 IN ('finished','failed','cancelled') THEN $3 ELSE finalized_at END \
                 WHERE id=$1 AND kind='claimable' AND status = ANY($4) \
                   AND (claim_lease_id = $5 OR $6) \
                   AND (claim_policy = 'broadcast' OR lease_expires_at >= $7) \
                 RETURNING id",
            )
            .bind(&task_id)
            .bind(new_status)
            .bind(now_tz)
            .bind(source_statuses)
            .bind(lease_opt)
            .bind(is_bypass)
            .bind(now)
            .fetch_optional(&state.db)
            .await;

            match row {
                Ok(Some(_)) => ok_result(json!({"task_id": task_id, "status": new_status})),
                Ok(None) => {
                    // Classify without an HTTP status code — MCP's contract is
                    // ok/error JSON only (spec §1 "MCP mirrors the same gap
                    // independently"). Re-fetch to distinguish not-found from a
                    // predicate failure, same 403-vs-409 logic as the REST side,
                    // expressed as distinct error strings the CLI can branch on.
                    let exists: Option<String> = sqlx::query_scalar("SELECT claimed_by_agent_id FROM task WHERE id=$1")
                        .bind(&task_id)
                        .fetch_optional(&state.db)
                        .await
                        .ok()
                        .flatten();
                    match exists {
                        None => err_result("task_not_found"),
                        Some(claimed) => {
                            let owns_directly = claimed.as_deref() == Some(subject_id);
                            if is_bypass || owns_directly || lease_opt.is_some() {
                                err_result("lease_conflict")
                            } else {
                                err_result("forbidden")
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("mcp {tool}: {e}");
                    err_result("database_error")
                }
            }
        }
```

Note: the `exists` re-fetch above binds `claimed_by_agent_id` as `Option<String>` even though the SQL scalar returns `Option<Option<String>>` semantically (row exists but column is NULL vs. row absent) — `sqlx::query_scalar` on a nullable column with `.fetch_optional()` already collapses "row absent" to the outer `None`; a present row with a NULL `claimed_by_agent_id` decodes as `Ok(None)` too, which `.ok().flatten()` would also turn into `None`, indistinguishable from "task not found." Fix by selecting a non-nullable sentinel column instead:

```rust
                    let exists: Option<(Option<String>,)> = sqlx::query_as("SELECT claimed_by_agent_id FROM task WHERE id=$1")
                        .bind(&task_id)
                        .fetch_optional(&state.db)
                        .await
                        .unwrap_or(None);
                    match exists {
                        None => err_result("task_not_found"),
                        Some((claimed,)) => {
                            let owns_directly = claimed.as_deref() == Some(subject_id);
                            if is_bypass || owns_directly || lease_opt.is_some() {
                                err_result("lease_conflict")
                            } else {
                                err_result("forbidden")
                            }
                        }
                    }
```

Use this `query_as` version, not the `query_scalar` draft above — write it correctly the first time rather than writing the buggy version and fixing it in a follow-up step.

- [ ] **Step 4: Run the full test file**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS, including `assigned_task_cannot_be_claimed_via_mcp` (unaffected — different tool) and `fencing_stale_lease_cannot_complete_task_after_reclaim_via_mcp`.

- [ ] **Step 5: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplane-tower/src/routes/mcp.rs crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "fix(tower/mcp): fence complete/fail/block_mesh_task, add block precondition, granular error codes"
```

---

### Task 10: Independent periodic expiry sweep + covering index

**Files:**
- Create: `crates/edgeplane-tower/migrations/0015_task_dispatch_covering_index.sql`
- Modify: `crates/edgeplane-tower/src/routes/work.rs:465-512` (`expire_stale_leases`, refactor into a shared core + a new global entry point)
- Modify: `crates/edgeplane-tower/src/main.rs` (spawn the periodic task)
- Test: `crates/edgeplane-tower/tests/test_task_kind_unification.rs`

**Interfaces:**
- Produces: `pub(crate) async fn run_lease_expiry_sweep(db: &sqlx::PgPool, mission_id: Option<&str>) -> Result<u64, sqlx::Error>` (new shared core in `work.rs`, `pub(crate)` so `main.rs` can call it).
- `expire_stale_leases(db, mission_id)` (existing signature, called from `list_tasks`) becomes a thin wrapper: `let _ = run_lease_expiry_sweep(db, Some(mission_id)).await.inspect_err(|e| tracing::error!("expire_stale_leases: {e}"));` — same call sites, same behavior, now logs instead of swallowing.

- [ ] **Step 1: Write the migration**

Create `crates/edgeplane-tower/migrations/0015_task_dispatch_covering_index.sql`:

```sql
-- EP-1 §1: the sweep's WHERE clause matches status IN ('claimed','running'),
-- but the only supporting index (0014) is a partial index on
-- status='claimed' alone. A 30s all-mission sweep scanning 'running' rows
-- with no matching index becomes a real load source at scale, not a cheap
-- sweep. Replace with a covering index over both statuses.
DROP INDEX IF EXISTS ix_task_lease_expires_at;

CREATE INDEX ix_task_lease_sweep ON public.task
    USING btree (lease_expires_at)
    WHERE status IN ('claimed', 'running') AND lease_expires_at IS NOT NULL;
```

- [ ] **Step 2: Write the failing test for error-logging (not swallowing)**

Add to `test_task_kind_unification.rs`. This test can't easily force a real DB error mid-sweep without a fault-injection harness this crate doesn't have, so instead assert the *index* exists and covers both statuses — the concrete, testable half of the spec's finding — and leave the logging behavior to a code-review-visible diff (the `let _ = ...` → `.inspect_err(tracing::error!)` change is a one-line, self-evidently-correct fix that doesn't need its own DB-fault test):

```rust
#[tokio::test]
async fn sweep_index_covers_both_claimed_and_running() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    let idx_def: Option<String> = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes WHERE tablename='task' AND indexname='ix_task_lease_sweep'",
    )
    .fetch_optional(&pool)
    .await
    .expect("query pg_indexes");
    let idx_def = idx_def.expect("ix_task_lease_sweep must exist after migration 0015");
    assert!(
        idx_def.contains("claimed") && idx_def.contains("running"),
        "sweep index must cover both 'claimed' and 'running' status rows, not just 'claimed': {idx_def}"
    );
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'test(sweep_index_covers)'`
Expected: FAIL — `ix_task_lease_sweep` doesn't exist yet, only the old `ix_task_lease_expires_at`.

- [ ] **Step 4: Apply the migration and refactor `expire_stale_leases`**

The migration from Step 1 runs automatically on the test pool via `sqlx::migrate!` (same mechanism `main.rs` uses in production — confirm the test harness's `setup()` also runs migrations against `TEST_DATABASE_URL`; if it uses a pre-migrated fixture DB instead, check `crates/edgeplane-tower/tests/common/mod.rs`'s `setup()` for how it provisions schema and follow that same path so the new migration is picked up before this test runs).

In `work.rs`, replace `expire_stale_leases` (`work.rs:493-512`) with:

```rust
/// Core sweep: expire leases past due, bounded-retry them per
/// `attempt`/`max_attempts`. `mission_id = None` sweeps every mission in one
/// query (the new independent periodic sweep); `Some(id)` scopes to one
/// mission (the existing `list_tasks`-side-effect call site). Returns the
/// number of rows the sweep touched, or the DB error — callers decide how to
/// log it; this function no longer swallows errors itself (spec §1 finding:
/// `let _ = ...` previously discarded every sweep failure silently).
pub(crate) async fn run_lease_expiry_sweep(
    db: &sqlx::PgPool,
    mission_id: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let now = Utc::now().naive_utc();
    let now_tz = Utc::now();
    let base = "UPDATE task SET \
           attempt = attempt + 1, \
           status = CASE WHEN attempt + 1 >= max_attempts THEN 'failed' ELSE 'ready' END, \
           finalized_at = CASE WHEN attempt + 1 >= max_attempts THEN $2 ELSE finalized_at END, \
           claimed_by_agent_id = NULL, lease_expires_at = NULL, claim_lease_id = NULL, \
           updated_at = $1 \
         WHERE kind='claimable' AND status IN ('claimed','running') \
           AND claim_policy != 'broadcast' \
           AND lease_expires_at IS NOT NULL AND lease_expires_at < $1";

    let result = if let Some(mid) = mission_id {
        sqlx::query(&format!("{base} AND mission_id = $3"))
            .bind(now)
            .bind(now_tz)
            .bind(mid)
            .execute(db)
            .await?
    } else {
        sqlx::query(base).bind(now).bind(now_tz).execute(db).await?
    };
    Ok(result.rows_affected())
}

/// Per-mission wrapper used as a side effect of `list_tasks`. Logs sweep
/// failures instead of swallowing them (spec §1 finding).
async fn expire_stale_leases(db: &sqlx::PgPool, mission_id: &str) {
    if let Err(e) = run_lease_expiry_sweep(db, Some(mission_id)).await {
        tracing::error!("expire_stale_leases (mission {mission_id}): {e}");
    }
}
```

- [ ] **Step 5: Wire the independent periodic sweep in `main.rs`**

In `main.rs`, before `let app = build_app(db, config);` (line 87), insert:

```rust
    // Independent periodic sweep — expire_stale_leases previously only ran
    // as a side effect of list_tasks; if nothing polls a mission, its
    // expired leases never requeue. Spec EP-1 §1 "Background expiry sweep".
    {
        let sweep_db = db.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                match edgeplane_tower::routes::work::run_lease_expiry_sweep(&sweep_db, None).await {
                    Ok(n) if n > 0 => tracing::info!(rows = n, "periodic lease-expiry sweep reclaimed tasks"),
                    Ok(_) => {}
                    Err(e) => tracing::error!("periodic lease-expiry sweep failed: {e}"),
                }
            }
        });
    }

```

This requires `run_lease_expiry_sweep` to be reachable as `edgeplane_tower::routes::work::run_lease_expiry_sweep` from `main.rs` — check whether `routes::work` (and its `pub(crate)` items) is already visible at that path from the binary crate (same crate, so `pub(crate)` should already resolve) by running `cargo check -p edgeplane-tower` after this edit; if `work` isn't a `pub` or `pub(crate)` module from the crate root, add the necessary visibility (`pub(crate) mod work;` in `routes/mod.rs` or wherever modules are declared) rather than widening `run_lease_expiry_sweep` itself to `pub`.

- [ ] **Step 6: Run the full test file and check the sweep doesn't double-fire incorrectly**

Run: `TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/test cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml -E 'binary(test_task_kind_unification)'`
Expected: PASS, including `retry_backoff_default_max_attempts_one_fails_on_first_expiry` and `retry_backoff_max_attempts_two_requeues_then_fails`, which exercise `expire_stale_leases`'s per-mission wrapper path (unchanged behavior, just now going through the shared core).

- [ ] **Step 7: `cargo check` the binary target specifically (the periodic-sweep spawn is in `main.rs`, not covered by the lib-crate test binary)**

Run: `cargo check -p edgeplane-tower --bin edgeplane-tower`
Expected: compiles clean.

- [ ] **Step 8: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

- [ ] **Step 9: Commit**

```bash
git add crates/edgeplane-tower/migrations/0015_task_dispatch_covering_index.sql \
        crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/src/main.rs \
        crates/edgeplane-tower/tests/test_task_kind_unification.rs
git commit -m "feat(tower): independent periodic lease-expiry sweep, covering index, stop swallowing sweep errors"
```

---

## Self-Review

**Spec coverage** (spec §1 subsections → task):
- "The pattern to converge on" → Tasks 1-9 (every endpoint converted).
- "Why the lease-expiry check is required" → `lease_expires_at >= now()` present in every fenced predicate (Tasks 1-4, 8-9).
- "403 vs 409, done correctly" → Task 1 (`classify_fenced_rejection`) + Tasks 2-6, 8 consuming it; Task 9 for MCP's parallel (non-HTTP-status) treatment.
- "The specific transitions and their legal source states" table → each row is one of Tasks 1 (heartbeat), 2 (complete, incl. `waiting_review` source + pending-gate CTE), 3 (fail), 4 (cancel), 5 (block), 6 (unblock), 7 (resolve_gate), 8 (append_progress).
- "Correction: terminal transitions don't fully clear ownership" → Tasks 2, 3, 7 all clear `claimed_by_agent_id` alongside the lease fields now.
- "resolve_gate: bespoke transaction" → Task 7.
- "block/unblock: lease retention decision" → Task 5 (block releases the lease, stated explicitly in code comment).
- "Broadcast claims and full-trust/admin bypass — explicitly unfenced/explicit" → Task 1 Step 1 (doc comment) + every fenced predicate's `$is_bypass` term.
- "MCP mirrors the same gap independently" → Task 9.
- "Background expiry sweep" (independent sweep, error logging, covering index) → Task 10.
- Testing section → covered across Tasks 1-10's own test additions; the one item NOT separately covered is "a test asserting the background sweep logs (not swallows) a DB error" — noted as impractical without a fault-injection harness in Task 10 Step 2, substituted with the index-coverage test (the concrete, testable half of that finding).

**Out of scope, confirmed not silently touched:** `task_loop.rs`'s heartbeat-gap and lease-loss-miscategorization bugs (spec §2), `task_worker.rs` unification (spec §3), supervision + drain (spec §4) — none of these files are modified except the single, narrowly-scoped `post_progress`/`task_loop.rs:332` interlock fix in Task 8, which is explicitly justified as unavoidable (a required-field change on one side without the other side sending it breaks production between deploys) rather than scope creep.

**Placeholder scan:** every step has real SQL/Rust, no "add appropriate error handling"-style stand-ins. The one deliberately-not-written test (sweep DB-error logging) is called out explicitly with its substitute, not silently dropped.

**Type consistency:** `classify_fenced_rejection`'s signature (`db, p, task_id, lease_id: Option<&str>`) is identical across every call site in Tasks 1-6, 8. `run_lease_expiry_sweep`'s signature (`db, mission_id: Option<&str>`) matches its two call sites in Task 10. `post_progress`'s new 4th parameter (`claim_lease_id: &str`) matches its one call site in Task 8 Step 5.

## Roadmap: Phase 2 candidates (surfaced during Phase 1 dual-review, 2026-08-19/20)

Items below are deliberately **not** in this plan's scope (Tower REST only). Listed here so they
don't get lost between this plan's completion and whenever a Phase 2 picks them up — each one
has enough context to start from without re-deriving it.

- **`task_loop.rs`'s event-driven heartbeat gap.** Already fully specified in spec §2 ("Quiet-
  stream heartbeat gap") — a fresh dual-review (rust-reviewer) independently rediscovered the same
  bug, confirming it's still live and still accurately scoped. Spec §2 also covers three related
  bugs found on the same file (lease-loss miscategorized as failure, missing-lease-after-claim not
  treated as fatal, dead watchdog offline-fail wiring) — all still open, none touched by this plan.
- **MCP mirror (Task 9, this plan).** Already scheduled; the spec's "MCP mirrors the same gap
  independently" section now also carries two dual-review specifics worth re-verifying at
  implementation time in case they've drifted: `mcp.rs`'s `updated_at=NOW()` still has the
  timezone-GUC dependency Task 1 fixed on REST, and `heartbeat_mesh_task`'s 300s TTL disagrees with
  REST's 120s `LEASE_TTL_SECS`. **Design question to resolve before implementing, not just noting**
  (gpt-5.6-terra review, 2026-08-20, cross-checked and endorsed independently): the plan as
  currently scoped has MCP hand-derive its *own* fenced predicates, structurally identical to but
  separately written from REST's. That's exactly the "re-derive by analogy" shape that caused every
  real bug in Tasks 1-5 — two independently-verified-but-independently-fallible copies of the same
  authorization logic, guaranteed to drift the moment one side changes without the other. Evaluate
  having MCP's handlers call the same Rust transition function/logic REST's handlers use (one
  fenced UPDATE per transition, shared by both surfaces) instead of writing a second predicate —
  this eliminates the duplication risk structurally rather than relying on review to keep two
  copies in sync. If a shared function turns out to be impractical given MCP's different response
  shape (JSON ok/error, not HTTP status — spec already flags this), that's a legitimate reason to
  keep them separate, but make that a stated decision when Task 9 is dispatched, not a default.
- **`crates/edgeplane/src/solo_supervisor.rs`'s heartbeat gap — not yet tracked anywhere before
  this.** Presents a real `claim_lease_id` on `/complete`/`/fail` (unlike `task_worker.rs`, which
  sends none), but its heartbeat thread only renews the *agent* heartbeat
  (`/work/agents/{id}/heartbeat`), never the *task* lease (`/work/tasks/{id}/heartbeat`) — so a
  task running past `LEASE_TTL_SECS` (120s) correctly, not buggily, gets 409 on completion, and all
  three call sites (`solo_supervisor.rs:404-426`) discard the result with `let _ = ...`, so the
  supervisor can't even observe it. Fix belongs in that crate (add task-lease heartbeating, or
  extend the TTL for that caller specifically) — flagged here rather than fixed inline because it's
  a different crate's daemon loop, out of this plan's Tower-only scope, same reasoning as
  `task_worker.rs`/`task_loop.rs` above.
- **`resolve_gate` (Task 7, this plan) inherits the attribution + idempotent-retry pattern from
  Tasks 2/3's post-dual-review fix, not just the 3-field lease clear the spec's "Correction"
  section already calls for.** When Task 2/3's `finalized_by_subject` column and
  `classify_fenced_rejection`'s "already-reached-target-status → 409 unconditionally" check land,
  Task 7's dispatch brief needs to apply both to `resolve_gate`'s approve/reject transitions too —
  otherwise it repeats the exact non-repudiation-loss and idempotent-retry-breaks-403 regression
  Tasks 2/3 just had fixed for them. Deliberately scoped out of the current design pass (kept to
  complete_task/fail_task, which are already shipped and already broken) rather than designing
  ahead of code that doesn't exist yet — pick this up when Task 7 is actually dispatched.
- **`claim_task`'s broadcast branch (`work.rs:1185-1203`) is genuinely unfenced at the UPDATE
  level — no CAS condition, just an earlier separate `SELECT` for the `status='ready'` precondition
  — and this now interacts with this plan's own broadcast-ownership fix in a way worth tracking.**
  Surfaced investigating (and refuting) a gpt-5.6-terra claim that broadcast tasks support true
  concurrent multi-claimer semantics (they don't — `status != 'ready'` gates every claim policy
  identically, confirmed live at work.rs:1173, before the broadcast branch even runs, so a second
  agent cannot claim a broadcast row while it's already `running`). The real, narrower issue: two
  callers racing to claim the *same* still-`ready` broadcast task at nearly the same instant can
  both pass the precondition and both write, unconditionally — the second silently overwrites the
  first's `claim_lease_id`. Harmless before this plan, because pre-fencing (and pre-Task-1/2's
  broadcast-ownership-bypass bug) any caller could complete/fail a broadcast task regardless of the
  row's current lease state. Since 37dca61a's fix, completing a broadcast task now requires
  lease-match-or-bypass — so a clobbered first-claimer could become legitimately unable to finish
  its own in-progress work, and `expire_stale_leases` still excludes `claim_policy='broadcast'`
  rows (confirmed live, work.rs:613), so nothing auto-recovers it either. Narrow, real, edge-case
  (requires two claims racing within the same request window) — not a rework trigger, but worth a
  look whenever `claim_task`'s broadcast branch is next touched: either accept the race as a known,
  documented cost of "intentionally unfenced," or give broadcast claims a per-claim/per-attempt
  identity instead of overwriting one singleton lease.
  **Severity correction (independent Codex review, 2026-08-28):** the above framed this narrowly as
  "two claims racing" — the actual UPDATE (`work.rs:~1206-1216`, verified live) has **zero predicate
  beyond `WHERE id=$1`**, not just no version/CAS check. It doesn't re-verify `status='ready'` OR
  `claim_policy='broadcast'` at write time, only at the earlier separate read. Concretely, this means
  a stale broadcast-claim request isn't limited to clobbering a *racing claim* — it can overwrite a
  task that has since moved to **any** state, including `finished`/`cancelled`, resetting it to
  `running` with the stale request's `claimed_by_agent_id`/lease. Same "intentionally unfenced by
  design" disposition still applies (this is the claim path, explicitly out of the fenced-transition
  primitive's scope per the Fable investigation below), but the fix-when-touched note should read
  "add a `status='ready' AND claim_policy='broadcast'` predicate to the UPDATE, at minimum" rather
  than treating this as merely a lease-clobber edge case.
- **`claim_task`'s exclusive-claim CAS has an unchecked `i32` overflow** (`work.rs:~1270`,
  `version_counter + 1` with no `checked_add`) — panics in debug builds at `i32::MAX`, wraps to a
  negative version in release and keeps using it for CAS comparisons. Requires an extreme/corrupted
  row state to reach; LOW severity, but a real, newly-flagged (independent Codex review, 2026-08-28)
  correctness nit in the same function as the broadcast-predicate gap above — fix both together
  whenever `claim_task` gets its own dedicated pass.
- **`create_gate` (`work.rs:1987-2032`) is check-then-insert with no fencing on the insert itself —
  never part of EP-1's 8-endpoint scope, newly surfaced by independent review (2026-08-28).** Calls
  `authz_task_owner` as a precheck, then an unconditional `INSERT INTO reviewgate` with no
  task-status, ownership, or lease predicate on the insert. Confirmed live. A caller who owned the
  task at check-time but has since lost ownership (reclaimed, completed, cancelled) can still attach
  a pending gate to it — and since `complete_task`'s and `resolve_gate`'s gate-aggregate CTEs read
  `reviewgate` fresh each time but don't lock the task row against a concurrent `create_gate` insert,
  a gate created in that exact window can be either missed (task finishes despite an attacker's gate,
  narrow) or unexpectedly attached to a task the *new*, legitimate claimer didn't ask to be
  gate-reviewed (the more likely practical impact — an authz gap, not primarily a race). Fix, when
  picked up: fence the INSERT the same way the rest of this plan fences mutations — `INSERT ... SELECT
  ... WHERE EXISTS (task still owned by caller AND still in a gate-attachable status)`.
- **CLOSED by `docs/superpowers/plans/2026-08-28-shared-fenced-transition-primitive.md`.** `append_progress`'s fenced CTE lacks a row lock, unlike its `UPDATE...WHERE` siblings — confirmed
  by two independent reviews now (rust-reviewer's Task 8 fix-round re-review, live 3-connection
  reproduction; and an independent Codex review, 2026-08-28, same conclusion via SQL-semantics
  reasoning). `WITH eligible AS (SELECT 1 FROM task WHERE ...)` takes the statement's snapshot with
  no `FOR UPDATE`, so a concurrent write to the same task row (reclaim, completion) that commits
  after the snapshot is taken but before the INSERT completes isn't observed. This is now a firm
  design requirement for the shared fenced-transition primitive (`docs/superpowers/specs/` — see the
  new spec once written): the "live-lease" fence family (heartbeat + progress) must use a row-locked
  `UPDATE...WHERE`-shaped statement, not a lock-free CTE, closing this structurally rather than
  patching the old code.
- **`retry_task` (`work.rs:1083-1125`) — severity correction (independent Codex review,
  2026-08-28).** Already logged (2026-08-26) as "any domain member can retry any failed/cancelled
  task, no ownership check." The check-then-act window is worse than that framing suggested: because
  the write is a blind `UPDATE ... WHERE id=$1` with no status re-check, a caller who read the task
  as `failed`/`cancelled` can still apply the retry-reset even if the task has since been legitimately
  retried and re-claimed by someone else — ripping an **actively claimed, in-progress** task back to
  `ready` and nulling the new claimer's lease, not merely restarting dead work. Same disposition
  (out of EP-1's 8-endpoint scope, log only, per Merlin's 2026-08-26 call) — noted here so the
  severity is accurate when this is eventually picked up: needs `AND status IN ('failed','cancelled')`
  on the UPDATE itself, not just the earlier read.
- **CLOSED by `docs/superpowers/plans/2026-08-28-shared-fenced-transition-primitive.md`.** `progress_mesh_task` (MCP, `mcp.rs`) is unfenced and outside even Task 9's stated scope —
  found during Task 8's independent review, not previously tracked anywhere. Task 9's own scope
  (below) is `complete_mesh_task`/`fail_mesh_task`/`block_mesh_task` only (`mcp.rs:791-864`);
  `progress_mesh_task` (`mcp.rs:731-786`) and `heartbeat_mesh_task` are in NO task's stated scope,
  despite this plan's own Goal statement (line 5) explicitly listing "complete/fail/block/**progress**"
  as the transitions a stale claimer must never be able to perform on a reclaimed task. Concretely:
  `progress_mesh_task`'s `claim_lease_id` is OPTIONAL (empty string → `None`), and `authz_task_owner`
  passes on a bare `claimed_by_agent_id == subject` match with no lease check and no freshness check
  at all — a stale claimer whose lease expired and was reclaimed can still post progress via MCP,
  the exact threat this whole plan exists to close, now that Task 8 closes it on the REST side.
  Decision needed when Task 9 is dispatched: fold `progress_mesh_task`/`heartbeat_mesh_task` into
  Task 9's scope, or split them into their own task — but don't leave them unscoped again.
- **Deploy-ordering hazard for Task 8's tower/daemon interlock — not a code bug, an operational
  note.** Landing tower (`edgeplane-tower`, a K8s Deployment) and daemon (`edgeplaned`, runs on fleet
  nodes) changes in one commit fixes source-tree ordering, not deploy ordering — they ship as
  separate artifacts. A tower with Task 8's change talking to a not-yet-upgraded `edgeplaned` 422s
  every progress post from that node, and `post_progress` failures are only `tracing::warn!`'d
  (`task_loop.rs`), so it fails silently: progress events/SSE just go dark for un-upgraded nodes
  until they update. Worth an explicit rollout note (upgrade `edgeplaned` nodes before or with the
  tower deploy) whenever this branch actually ships — not a blocker for continuing Phase 1's
  remaining tasks.
- **`append_progress`'s `seq = COALESCE(MAX(seq),-1)+1` computation races two concurrent posts
  against the same task — verified live (Task 8), not just Terra's flagged-but-unconfirmed claim
  from the handoff.** No `UNIQUE(task_id, seq)` constraint exists on `meshprogressevent` (confirmed
  against the live schema), so this isn't a retryable conflict — it's a silent duplicate `seq` value
  across two rows, an ordering/data-quality issue for any consumer that treats `seq` as a strict
  per-task cursor. Pre-existing (unrelated to fencing, present before this plan started), explicitly
  scoped out by the spec's own text ("that pre-existing race is out of EP-1's scope"). Fix, if ever
  picked up: either a `UNIQUE(task_id, seq)` constraint (turns silent duplication into a retryable
  23505 the caller can react to) or a per-task Postgres sequence/serial allocation instead of a
  `MAX+1` read. **Considered and NOT adopted (Task 8 independent review's suggestion): adding `FOR
  UPDATE` to the CTE's `SELECT 1 FROM task WHERE id=$1 ...`**, on the theory that locking the task
  row serializes concurrent posts to the same task and closes the race for free, no migration. Real
  open question, not dismissed lightly: this is ONE round-tripped SQL statement (CTE + the
  `meshprogressevent` MAX(seq) subquery + the INSERT), not a multi-statement transaction — under
  READ COMMITTED, a statement's snapshot is fixed at that statement's own start, and `FOR UPDATE`'s
  documented EvalPlanQual re-check only refreshes the specific LOCKED row's own data for further
  qual evaluation, not the snapshot used by an unrelated table read (the `meshprogressevent`
  subquery) elsewhere in the same statement. If that reasoning holds, a second caller blocked on the
  lock could still compute `MAX(seq)` from its pre-wait snapshot after the lock releases, seeing the
  same value the first caller did — i.e. `FOR UPDATE` might only serialize the WRITES, not actually
  freshen the READ that decides `seq`, and the duplicate-seq race could survive it. This needs a
  genuine two-connection concurrent probe to settle, not statement-validity testing (both this
  session and the reviewing session confirmed the SQL is valid and takes a lock; neither confirmed
  the deeper correctness property). Don't adopt `FOR UPDATE` here as a fix without that probe.
- **`dispatch_task` (work.rs) is another unfenced terminal `→'finished'` transition, missed by this
  plan's own original §1 endpoint table — not yet independently verified beyond a single review's
  read.** Flagged by an independent rust-reviewer pass during Task 7 (SDD ledger, Task 7 review).
  App-level `status != "ready"` precheck then a blind `UPDATE task SET status='finished' ... WHERE
  id=$1` — same TOCTOU shape every endpoint in this plan exists to close, and it stamps neither
  `finalized_at` nor `finalized_by_subject`, so a `claim_task` landing in the precheck-to-write
  window would leave a `'finished'` row still carrying a live `claim_lease_id`/`claimed_by_agent_id`/
  `lease_expires_at` — the exact ownership-carryover hole the spec's "Correction: terminal
  transitions don't fully clear ownership" section exists to close for every OTHER terminal
  transition. Out of this plan's Task 7 scope (a different endpoint entirely); worth its own
  dispatch/review cycle alongside the `claim_task` broadcast-branch item above once Phase 1 wraps.
- **Four Minor findings from Task 6's review, all pre-existing or narrow-scope, not fixed:**
  (1) `unblock_dependents` (`work.rs:680`) sweeps `status IN ('pending','blocked')` → `'ready'` with
  no check that the block was dependency-caused — a task an agent deliberately paused via
  `block_task` could in principle get auto-unblocked by an unrelated dependency finishing.
  Reachability is narrow today (a claimable task only becomes claimable once its deps are already
  finished), but MCP `block_mesh_task` has no status precondition at all, so it can move a
  `'pending'` row into `'blocked'` and reopen the window — check when Task 9 touches MCP block.
  (2) `'pending'` tasks are no longer force-readyable through `/unblock` (the old blind UPDATE had
  no status precondition; the new `status='blocked'` conjunct 409s a `'pending'` row). Matches the
  plan's own prescription, no real caller of `/unblock` exists at all today — a known, intentional
  narrowing, not a surprise, but worth remembering if an operator ever needs that escape hatch.
  (3) MCP `block_mesh_task` (`mcp.rs:830`) sets `status='blocked'` for any `kind` with no status
  precondition — it can create an assigned-kind row in `'blocked'`, which REST `/unblock`'s new
  `kind='claimable'` gate can never recover. Task 9's `block_mesh_task` note already covers the
  proof-shape difference (Roadmap item above); add the kind gate to that same fix.
  (4) `unblock_task` moves a task to `'ready'` without calling `broadcast_task_available`, unlike
  every other `'ready'`-producing path. Currently harmless (the daemon discovers work by polling
  `list_tasks?status=ready`, not the notify registry) — latency, not loss. Consistency-only note.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-08-18-ep1-tower-fencing.md`. Two execution options:

1. **Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks. Given this touches auth/fencing logic (security-sensitive per this profile's CLAUDE.md — "Security-sensitive changes: No — Claude directly"), route each task's implementation through `rust-engineer` and its review through `rust-reviewer` per this profile's delegation table, rather than doing the edits inline.

2. **Inline Execution** — execute tasks in this session using `executing-plans`, batch execution with checkpoints.

Which approach?
