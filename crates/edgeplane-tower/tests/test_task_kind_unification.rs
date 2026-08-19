//! Migration 0014 (task/meshtask unification) regression tests: kind-gating,
//! reclaim fencing, and bounded-retry backoff.
//!
//! These exercise the unified `task` table's `kind` discriminator
//! ('assigned' | 'claimable') and the reclaim-sweep behavior in
//! `routes/work.rs::expire_stale_leases`, which now (a) clears the stale
//! `claim_lease_id` on reclaim (the fencing fix — previously left in place,
//! letting an agent whose lease expired keep acting on the task with its old
//! token) and (b) counts `attempt`/`max_attempts` instead of unconditionally
//! re-readying a timed-out task forever.

mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::Row;

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

/// Enroll an agent (as owner session) and return the enrolled agent_id +
/// agent_token from the response. Mirrors `test_authz.rs`'s helper of the
/// same name (each integration-test binary is a separate crate, so it can't
/// be imported — duplicated here per this crate's existing convention, see
/// e.g. `test_authz_search.rs`'s duplicated `bearer`/`server`).
async fn enroll_and_get_token(
    s: &TestServer,
    domain_id: &str,
    session_token: &str,
) -> (String, String) {
    let res = s
        .post(&format!("/api/work/domains/{domain_id}/agents/enroll"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {session_token}"),
        )
        .json(&serde_json::json!({"runtime_kind": "test"}))
        .await;
    assert_eq!(res.status_code(), 201, "enroll failed: {}", res.text());
    let body: serde_json::Value = res.json();
    let agent_id = body["id"].as_str().unwrap().to_string();
    let agent_token = body["agent_token"].as_str().unwrap().to_string();
    (agent_id, agent_token)
}

/// Trigger the reclaim sweep (`expire_stale_leases`, private to `routes::work`
/// and only reachable as a side effect of `GET /work/missions/{id}/tasks`)
/// for `mission_id`.
async fn trigger_reclaim_sweep(s: &TestServer, mission_id: &str, session_token: &str) {
    let res = s
        .get(&format!("/api/work/missions/{mission_id}/tasks"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {session_token}"),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "reclaim-sweep trigger (list_tasks) must succeed: {}",
        res.text()
    );
}

// ── Kind-gating: an assigned task is never claim/lease-scoped ───────────────

#[tokio::test]
async fn assigned_task_cannot_be_claimed() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id =
        common::seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "assigned task must reject claim: {}",
        res.text()
    );
}

#[tokio::test]
async fn assigned_task_cannot_be_heartbeated() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id =
        common::seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "assigned task must reject heartbeat: {}",
        res.text()
    );
}

#[tokio::test]
async fn assigned_task_cannot_be_progressed() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id =
        common::seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "nope"}))
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "assigned task must reject progress: {}",
        res.text()
    );
}

#[tokio::test]
async fn assigned_task_cannot_be_retried() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id =
        common::seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    let s = server(pool);
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
        "assigned task must reject retry: {}",
        res.text()
    );
}

#[tokio::test]
async fn assigned_task_cannot_be_claimed_via_mcp() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id =
        common::seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, "harness").await;
    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "claim_mesh_task",
            "args": { "task_id": task_id, "agent_id": "agent-x" }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["ok"], false,
        "assigned task must reject claim_mesh_task: {body}"
    );
}

// ── Claimable tasks: unchanged behavior ──────────────────────────────────────

#[tokio::test]
async fn claimable_task_claim_heartbeat_progress_retry_still_work() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = common::seed_ready_task(&pool, &ctx.mission_id, &ctx.domain_id).await;
    let s = server(pool);
    let owner = &ctx.owner_session_token;

    // Claim.
    let claim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(axum::http::header::AUTHORIZATION, format!("Bearer {owner}"))
        .json(&serde_json::json!({"agent_id": "agent-still-works"}))
        .await;
    assert!(
        claim_res.status_code().is_success(),
        "claim should succeed: {}",
        claim_res.text()
    );
    let claim_body: serde_json::Value = claim_res.json();
    let lease_id = claim_body["claim_lease_id"].as_str().unwrap().to_string();

    // Heartbeat.
    let hb_res = s
        .post(&format!("/api/work/tasks/{task_id}/heartbeat"))
        .add_header(axum::http::header::AUTHORIZATION, format!("Bearer {owner}"))
        .json(&serde_json::json!({"claim_lease_id": lease_id}))
        .await;
    assert!(
        hb_res.status_code().is_success(),
        "heartbeat should succeed: {}",
        hb_res.text()
    );

    // Progress.
    let progress_res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(axum::http::header::AUTHORIZATION, format!("Bearer {owner}"))
        .json(&serde_json::json!({"event_type": "status", "summary": "still working"}))
        .await;
    assert!(
        progress_res.status_code().is_success(),
        "progress should succeed: {}",
        progress_res.text()
    );

    // Fail it, then retry — proves the claimable retry path (independent of
    // kind-gating) is unaffected.
    let fail_res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(axum::http::header::AUTHORIZATION, format!("Bearer {owner}"))
        .json(&serde_json::json!({}))
        .await;
    assert!(
        fail_res.status_code().is_success(),
        "fail should succeed: {}",
        fail_res.text()
    );

    let retry_res = s
        .post(&format!("/api/work/tasks/{task_id}/retry"))
        .add_header(axum::http::header::AUTHORIZATION, format!("Bearer {owner}"))
        .await;
    assert!(
        retry_res.status_code().is_success(),
        "retry should succeed: {}",
        retry_res.text()
    );
    let retry_body: serde_json::Value = retry_res.json();
    assert_eq!(
        retry_body["status"], "ready",
        "retry must reset status to ready"
    );
}

// ── Fencing regression: reclaim must clear the stale claim_lease_id ─────────
//
// The bug fixed this session: the reclaim sweep cleared claimed_by_agent_id
// and lease_expires_at on expiry but left the stale claim_lease_id in place.
// A slow-but-alive agent presenting its old (pre-expiry) lease token could
// therefore still pass authz_task_owner's lease-match check after its lease
// had already been reclaimed. These tests prove the sweep now clears
// claim_lease_id too, and that the specific end-to-end exploit path (the
// combined MCP complete/fail/block handler, which — unlike the REST
// complete/fail/heartbeat siblings — has no independent status precondition
// or secondary lease-mismatch recheck, so authz_task_owner's lease match is
// its ONLY defense) is closed.

#[tokio::test]
async fn fencing_reclaim_clears_stale_lease_id() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // max_attempts=2 so a single expiry re-readies the row instead of
    // finalizing it to 'failed' — isolates the fencing behavior (this test)
    // from the bounded-retry behavior (covered separately below).
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "claimed",
        Some("agent-A"),
        2,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='stale-lease-a', lease_expires_at = now() - interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("force lease into the past");

    let s = server(pool.clone());
    trigger_reclaim_sweep(&s, &ctx.mission_id, &ctx.owner_session_token).await;

    let row = sqlx::query(
        "SELECT status, attempt, claimed_by_agent_id, claim_lease_id, lease_expires_at FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .expect("fetch reclaimed task");

    assert_eq!(
        row.get::<String, _>("status"),
        "ready",
        "row must be re-readied (attempt 1 < max_attempts 2)"
    );
    assert_eq!(row.get::<i16, _>("attempt"), 1);
    assert!(
        row.get::<Option<String>, _>("claimed_by_agent_id")
            .is_none(),
        "claimed_by_agent_id must be cleared"
    );
    assert!(
        row.get::<Option<String>, _>("claim_lease_id").is_none(),
        "claim_lease_id must be cleared on reclaim — this is the fencing fix; \
         pre-fix this stayed 'stale-lease-a'"
    );
    assert!(
        row.get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at")
            .is_none(),
        "lease_expires_at must be cleared"
    );
}

#[tokio::test]
async fn fencing_stale_lease_cannot_complete_task_after_reclaim_via_mcp() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // A real agent identity (non-full-trust — the lease-fencing path only
    // matters for restricted principals; full-trust/admin bypass ownership
    // checks entirely via authz_task_owner's early return).
    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 2).await;

    // Agent A claims for real, via the REST endpoint, to get a real lease.
    let claim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(
        claim_res.status_code().is_success(),
        "claim failed: {}",
        claim_res.text()
    );
    let claim_body: serde_json::Value = claim_res.json();
    let lease_a = claim_body["claim_lease_id"].as_str().unwrap().to_string();
    assert_eq!(claim_body["claimed_by_agent_id"], agent_a_id);

    // Force the lease into the past and sweep — task goes back to 'ready',
    // nobody has re-claimed it yet (the vulnerability window).
    sqlx::query("UPDATE task SET lease_expires_at = now() - interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("force lease into the past");
    trigger_reclaim_sweep(&s, &ctx.mission_id, &ctx.owner_session_token).await;

    let post_sweep_status: String = sqlx::query_scalar("SELECT status FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .expect("fetch post-sweep status");
    assert_eq!(
        post_sweep_status, "ready",
        "task must be back in the ready pool, unclaimed"
    );

    // Agent A — whose claim was reclaimed — tries to complete the task via
    // the MCP tool using its now-stale lease A. This handler
    // (complete_mesh_task/fail_mesh_task/block_mesh_task) has no status
    // precondition and no secondary lease-mismatch recheck (unlike the REST
    // complete_task/fail_task/heartbeat_task siblings) — authz_task_owner's
    // lease comparison against the row's CURRENT claim_lease_id is the only
    // thing standing between this call and an illegitimate completion.
    let complete_res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .json(&serde_json::json!({
            "tool": "complete_mesh_task",
            "args": { "task_id": task_id, "claim_lease_id": lease_a }
        }))
        .await;
    let complete_body: serde_json::Value = complete_res.json();
    assert_eq!(
        complete_body["ok"], false,
        "stale lease A must not be able to complete a reclaimed, unclaimed task: {complete_body}"
    );

    // Confirm the task was NOT illegitimately finished by the stale-token call.
    let final_status: String = sqlx::query_scalar("SELECT status FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .expect("fetch final status");
    assert_eq!(
        final_status, "ready",
        "task must remain unclaimed/ready — stale lease must not have completed it"
    );
}

/// Matches the exact scenario described in this migration's plan: claim as A,
/// force reclaim, have a second agent B claim it (getting a fresh, different
/// lease), then confirm A's original token is rejected. Note: because
/// `claim_task`/`claim_mesh_task` always overwrite `claim_lease_id` to a
/// fresh value on a successful claim, this scenario is ALSO defended by the
/// independent "Lease ID mismatch" recheck in the REST complete/fail/
/// heartbeat handlers, so — unlike
/// `fencing_stale_lease_cannot_complete_task_after_reclaim_via_mcp` above —
/// it would still pass even without this session's reclaim-clearing fix.
/// Kept as an end-to-end/defense-in-depth check of the desired system
/// behavior; the MCP gap-window test above is the one that specifically
/// isolates the fencing fix.
#[tokio::test]
async fn fencing_second_claimer_gets_fresh_lease_original_token_rejected() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 2).await;

    // Agent A claims (full-trust session claiming on A's behalf).
    let claim_a = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"agent_id": "agent-A"}))
        .await;
    assert!(
        claim_a.status_code().is_success(),
        "claim A failed: {}",
        claim_a.text()
    );
    let lease_a = claim_a.json::<serde_json::Value>()["claim_lease_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Force expiry + reclaim.
    sqlx::query("UPDATE task SET lease_expires_at = now() - interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("force lease into the past");
    trigger_reclaim_sweep(&s, &ctx.mission_id, &ctx.owner_session_token).await;

    // Agent B claims.
    let claim_b = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"agent_id": "agent-B"}))
        .await;
    assert!(
        claim_b.status_code().is_success(),
        "claim B failed: {}",
        claim_b.text()
    );
    let lease_b = claim_b.json::<serde_json::Value>()["claim_lease_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(
        lease_a, lease_b,
        "second claim must mint a different lease id"
    );

    // A restricted (non-full-trust) caller presenting A's original lease
    // must be rejected — full-trust/admin bypass authz_task_owner entirely,
    // so this must go through a principal that actually needs the lease
    // match (the domain-contributor service account).
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
        "stale original lease A must not complete a task now claimed under lease B \
         (409, not 403 — the caller presented a lease, so this is a lost race, not \
         unauthorized access; spec §1 '403 vs 409, done correctly'): {}",
        complete_res.text()
    );
}

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

/// The real production success path: a non-full-trust `agent`-type principal
/// heartbeats a task it genuinely claimed itself, presenting its own live,
/// correct `claim_lease_id`. This is the `(claim_lease_id = $4 OR $5)` branch
/// with `$4` actually matching — every other test in this file either uses a
/// full-trust session token (which always takes the `$5` bypass branch) or
/// deliberately presents a wrong/missing lease to probe the 403/409 split.
/// Without this test, the branch real workers depend on
/// (`edgeplaned-work`'s `task_loop.rs`, which always threads its own
/// `claim_lease_id` through every heartbeat) had zero passing coverage.
#[tokio::test]
async fn fencing_heartbeat_real_agent_own_live_lease_succeeds() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // A real agent identity (non-full-trust — enrolled agent tokens are the
    // restricted principal type that actually exercises the lease-match
    // branch; full-trust/admin bypass ownership checks entirely via `$5`).
    let (agent_id, agent_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 2).await;

    // Claim for real, via the REST endpoint, as the agent itself — this
    // mints a genuine, live claim_lease_id owned by this exact principal.
    let claim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(
        claim_res.status_code().is_success(),
        "claim failed: {}",
        claim_res.text()
    );
    let claim_body: serde_json::Value = claim_res.json();
    assert_eq!(claim_body["claimed_by_agent_id"], agent_id);
    let lease_id = claim_body["claim_lease_id"].as_str().unwrap().to_string();

    // Heartbeat with the agent's own token and its own genuinely live lease.
    let hb_res = s
        .post(&format!("/api/work/tasks/{task_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"claim_lease_id": lease_id}))
        .await;
    assert!(
        hb_res.status_code().is_success(),
        "a real agent heartbeating its own live lease must succeed: {}",
        hb_res.text()
    );
    let hb_body: serde_json::Value = hb_res.json();
    assert_eq!(
        hb_body["status"], "running",
        "heartbeat must (re)set status to running"
    );
}

// ── Bounded retry / backoff (attempt vs. max_attempts) ───────────────────────

#[tokio::test]
async fn retry_backoff_default_max_attempts_one_fails_on_first_expiry() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // max_attempts defaults to 1 (the migration column default) — don't pass
    // it explicitly, to also prove the default itself.
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "claimed",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-1', lease_expires_at = now() - interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("force lease into the past");

    let s = server(pool.clone());
    trigger_reclaim_sweep(&s, &ctx.mission_id, &ctx.owner_session_token).await;

    let row = sqlx::query(
        "SELECT status, attempt, max_attempts, finalized_at, claim_lease_id FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .expect("fetch task after first expiry");
    assert_eq!(row.get::<i16, _>("max_attempts"), 1);
    assert_eq!(
        row.get::<i16, _>("attempt"),
        1,
        "attempt must increment on expiry"
    );
    assert_eq!(
        row.get::<String, _>("status"),
        "failed",
        "with max_attempts=1, the FIRST expiry must finalize to failed, not re-ready \
         (bounded-retry replaces the old unconditional reclaim-to-ready behavior)"
    );
    assert!(
        row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("finalized_at")
            .is_some(),
        "finalized_at must be stamped on the failing expiry"
    );
    assert!(
        row.get::<Option<String>, _>("claim_lease_id").is_none(),
        "claim_lease_id must be cleared even on the finalize-to-failed path"
    );
}

#[tokio::test]
async fn retry_backoff_max_attempts_two_requeues_then_fails() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "claimed",
        Some("agent-A"),
        2,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='lease-1', lease_expires_at = now() - interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("force lease into the past (1st expiry)");

    let s = server(pool.clone());
    trigger_reclaim_sweep(&s, &ctx.mission_id, &ctx.owner_session_token).await;

    let row = sqlx::query("SELECT status, attempt, finalized_at FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .expect("fetch task after first expiry");
    assert_eq!(
        row.get::<String, _>("status"),
        "ready",
        "1st expiry (attempt 1 < max_attempts 2) must re-ready"
    );
    assert_eq!(row.get::<i16, _>("attempt"), 1);
    assert!(
        row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("finalized_at")
            .is_none(),
        "finalized_at must stay unset while attempts remain"
    );

    // Simulate a second claim + timeout (bypassing the claim endpoint — only
    // the sweep's counting behavior is under test here).
    sqlx::query(
        "UPDATE task SET status='claimed', claimed_by_agent_id='agent-B', claim_lease_id='lease-2', \
         lease_expires_at = now() - interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("force 2nd claim + expiry");

    trigger_reclaim_sweep(&s, &ctx.mission_id, &ctx.owner_session_token).await;

    let row2 = sqlx::query("SELECT status, attempt, finalized_at FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .expect("fetch task after second expiry");
    assert_eq!(
        row2.get::<String, _>("status"),
        "failed",
        "2nd expiry (attempt 2 >= max_attempts 2) must finalize to failed"
    );
    assert_eq!(row2.get::<i16, _>("attempt"), 2);
    assert!(
        row2.get::<Option<chrono::DateTime<chrono::Utc>>, _>("finalized_at")
            .is_some(),
        "finalized_at must be stamped once attempts are exhausted"
    );
}

// ── Task 2: complete_task — fenced CAS + atomic pending-gate transition ─────

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
        common::seed_assigned_task(&pool, &ctx.mission_id, &ctx.domain_id, ctx.owner_session_subject())
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

// ── Task 2 review fix round: broadcast-wedge (Ruling C1) + restored
// claimed_by_agent_id identity path (Ruling C2) + carried-forward coverage
// gaps (Important #3/#4). See progress.md for the full rulings.

#[tokio::test]
async fn fencing_heartbeat_broadcast_task_bypasses_expired_lease() {
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
    // Broadcast + an already-expired lease that will never be swept
    // (expire_stale_leases skips claim_policy='broadcast') and can never be
    // re-claimed (claim_task requires status='ready'). Ruling C1: broadcast
    // bypasses the entire lease/freshness sub-condition, not just the
    // lease-id match, so this must still succeed.
    sqlx::query(
        "UPDATE task SET claim_policy='broadcast', claim_lease_id='lease-a', \
         lease_expires_at = now() - interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("seed a broadcast task with an expired lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/heartbeat"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(
        res.status_code().is_success(),
        "a broadcast task's expired lease must not block heartbeat (Ruling C1): {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_complete_broadcast_task_bypasses_expired_lease() {
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
    .expect("seed a broadcast task with an expired lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(
        res.status_code().is_success(),
        "a broadcast task's expired lease must not block completion (Ruling C1): {}",
        res.text()
    );
    assert_eq!(res.json::<serde_json::Value>()["status"], "finished");
}

#[tokio::test]
async fn fencing_complete_claimed_by_agent_id_without_lease_succeeds() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Mirrors the real, live task_worker.rs daemon loop (out of scope for
    // this Tower-only plan): it calls /complete with {"agent_id": ...} only
    // — never a claim_lease_id, and never heartbeats. Enroll a real
    // non-full-trust agent and claim for real via REST, so
    // claimed_by_agent_id is a genuine identity match.
    let (agent_id, agent_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 2)
            .await;
    let claim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(claim_res.status_code().is_success(), "{}", claim_res.text());
    assert_eq!(
        claim_res.json::<serde_json::Value>()["claimed_by_agent_id"],
        agent_id
    );

    // Force the lease into the past — Ruling C2's identity path is
    // deliberately NOT gated on freshness (the state machine already makes
    // this race-safe without one), so this must still succeed even though
    // no lease is presented and the lease on file has expired.
    sqlx::query("UPDATE task SET lease_expires_at = now() - interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("force an expired lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(
        res.status_code().is_success(),
        "the real caller's own claimed_by_agent_id must complete without a lease (Ruling C2): {}",
        res.text()
    );
    assert_eq!(res.json::<serde_json::Value>()["status"], "finished");
}

#[tokio::test]
async fn fencing_complete_real_lease_match_non_bypass_succeeds() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // The actual production success path this predicate branch protects: a
    // restricted (non-full-trust) caller presenting the row's genuinely
    // live, correct claim_lease_id — not the identity path (caller's
    // subject is member_sa_token, not the task's claimed_by_agent_id) and
    // not the is_bypass path. Without this test the `(claim_lease_id = $5
    // OR $6)` branch would pass every existing test even hardcoded to $6.
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
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "a restricted caller presenting the real, live lease must complete: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_complete_expired_lease_not_yet_swept_is_rejected() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Distinct from fencing_complete_stale_lease_after_reclaim_is_409: no
    // reclaim sweep runs here, so claim_lease_id still matches on file.
    // This isolates the `lease_expires_at >= $4` freshness check itself —
    // the exact predicate the timezone-GUC bug lived in — proving it does
    // real rejection work independent of expire_stale_leases clearing the
    // lease id.
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
    .expect("force an expired lease, no sweep");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "a genuinely expired lease must be rejected even with a matching claim_lease_id \
         and no sweep having run: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_complete_timezone_guc_regression() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };

    // Regression guard for the bug this plan's Task 1 fix corrected:
    // lease_expires_at >= now() silently depended on the session TimeZone
    // GUC defaulting to UTC (an already-expired lease passed the fence
    // under America/Denver). The app now binds a Rust-computed naive `now`
    // instead of calling SQL now(), so behavior must be identical
    // regardless of session TimeZone. A second pool to the same database
    // whose connections default to a non-UTC session TimeZone drives the
    // actual HTTP request here.
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let tz_pool = sqlx::postgres::PgPoolOptions::new()
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET TIME ZONE 'America/Denver'")
                    .execute(conn)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&url)
        .await
        .expect("connect non-UTC pool");
    let s = server(tz_pool);

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
    .expect("force an expired lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "an expired lease must be rejected under a non-UTC session TimeZone too \
         (regression guard for the timezone-GUC bug): {}",
        res.text()
    );
}

// ── Task 3: fail_task — fenced CAS + 3-field lease clear ────────────────────

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
    assert!(
        row.get::<Option<String>, _>("claimed_by_agent_id").is_none(),
        "fail_task must clear claimed_by_agent_id, not just the lease fields"
    );
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

// Proactive coverage for fail_task's C1/C2 paths (Task 3's plan section
// already includes both from the start — see Task 1/2's fix rounds for why
// leaving these uncovered invites the exact same gap-and-reopen cycle).

#[tokio::test]
async fn fencing_fail_broadcast_task_bypasses_expired_lease() {
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
    .expect("seed a broadcast task with an expired lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"error": "boom"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "a broadcast task's expired lease must not block failure (Ruling C1): {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_fail_claimed_by_agent_id_without_lease_succeeds() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Mirrors task_worker.rs: calls /fail with {"agent_id": ...} only.
    let (agent_id, agent_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 2)
            .await;
    let claim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(claim_res.status_code().is_success(), "{}", claim_res.text());
    assert_eq!(
        claim_res.json::<serde_json::Value>()["claimed_by_agent_id"],
        agent_id
    );

    sqlx::query("UPDATE task SET lease_expires_at = now() - interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("force an expired lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"error": "boom"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "the real caller's own claimed_by_agent_id must fail without a lease (Ruling C2): {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_fail_real_lease_match_non_bypass_succeeds() {
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
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"claim_lease_id": "lease-a", "error": "boom"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "a restricted caller presenting the real, live lease must fail the task: {}",
        res.text()
    );
}
