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

/// A test server whose `AppConfig::admin_emails` contains `admin_email`,
/// plus a real session token minted for that same email — the only way to
/// get a `Principal` with `is_admin=true` in this test harness. Every other
/// `is_bypass`/full-trust test in this suite uses `ctx.owner_session_token`
/// or a node JWT instead, both of which satisfy `is_full_trust` (session or
/// node `auth_type`) but NOT `is_admin` (`server()`'s plain
/// `AppConfig::default()` has an empty `admin_emails`) — `is_admin=true` had
/// zero coverage anywhere in this plan before this helper (independent
/// review, Task 7).
async fn server_with_admin(pool: sqlx::PgPool, db: &sqlx::PgPool) -> (TestServer, String) {
    let admin_email = format!("admin-{}@example.com", uuid::Uuid::new_v4().simple());
    let mut admin_emails = std::collections::HashSet::new();
    admin_emails.insert(admin_email.clone());
    let config = AppConfig {
        admin_emails,
        ..Default::default()
    };
    let token = common::mint_session(db, &admin_email, &admin_email).await;
    (TestServer::new(build_app(pool, config)), token)
}

/// Builds a test server whose node/agent JWT verification key is a freshly
/// generated RSA keypair we also hold the private half of, so the caller can
/// sign a real, verifiable node JWT — the actual `task_worker.rs` auth
/// shape (it authenticates with its node's full-trust credential, never a
/// per-agent token). `build_app` reads `EP_JWT_SIGNING_KEY` at call time
/// (`server.rs::load_jwt_keys`), so this must be set before `server()` runs;
/// nextest's per-test-process isolation makes mutating this process-wide env
/// var safe here (each test is its own process).
fn server_with_node_signing_key(pool: sqlx::PgPool) -> (TestServer, jsonwebtoken::EncodingKey) {
    let (priv_pem, _pub_pem) = edgeplane_tower::jwt::generate_rsa_keypair().unwrap();
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, priv_pem.as_bytes());
    // SAFETY (soundness, not memory): mutating a process-wide env var is
    // inherently racy if another thread reads it concurrently; nextest
    // isolates each #[tokio::test] in its own process, so nothing else in
    // this process reads EP_JWT_SIGNING_KEY at the same time.
    unsafe {
        std::env::set_var("EP_JWT_SIGNING_KEY", b64);
    }
    let encoding_key = edgeplane_tower::jwt::encoding_key_from_pem(&priv_pem).unwrap();
    (server(pool), encoding_key)
}

/// Registers a `runtimenode`, gives it `domain_id` scope via a `meshagent`
/// row (`resolve_node_domain_scope` reads `meshagent.runtime_node_id`/
/// `node_id`), signs a real node JWT against `encoding_key`, and inserts the
/// matching `nodetoken` row `auth.rs`'s node-JWT path requires (signature
/// verification alone isn't enough — it also checks a live, non-revoked
/// `nodetoken` by `jti`). Returns the Bearer token string.
async fn seed_node_caller(
    pool: &sqlx::PgPool,
    domain_id: &str,
    encoding_key: &jsonwebtoken::EncodingKey,
) -> String {
    let node_name = format!("fencing-test-node-{}", uuid::Uuid::new_v4().simple());
    let node_id = common::seed_runtime_node(pool, "harness", &node_name).await;
    common::seed_node_agent(pool, domain_id, &node_id).await;
    let (node_jwt, jti) = edgeplane_tower::jwt::sign_node_jwt(&node_id, encoding_key, 1)
        .expect("sign node jwt");
    sqlx::query(
        "INSERT INTO nodetoken (jti, node_id, revoked, issued_at, expires_at) \
         VALUES ($1, $2, false, now(), now() + interval '1 day')",
    )
    .bind(&jti)
    .bind(&node_id)
    .execute(pool)
    .await
    .expect("insert nodetoken");
    node_jwt
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
    // seed_assigned_task's default status='open' would already fail the
    // fenced predicate's status IN ('claimed','running') term on its own —
    // this test would pass even with kind='claimable' deleted from the
    // predicate entirely, and so wouldn't actually isolate the kind gate it
    // claims to test (independent review, Task 8 second pass). Force status
    // into the claimable-only vocabulary so ONLY the kind mismatch can be
    // what rejects this row.
    sqlx::query("UPDATE task SET status='running' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .unwrap();
    let s = server(pool);
    // append_progress now requires claim_lease_id in the body (checked before
    // the task is even touched — a 422 for a missing lease would mask what
    // this test is actually about, the kind='assigned' rejection). The value
    // is irrelevant here: the caller is a full-trust session, which
    // classify_fenced_rejection unconditionally routes to 409 regardless of
    // any lease presented.
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "nope", "claim_lease_id": "irrelevant"}))
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
        .json(&serde_json::json!({"event_type": "status", "summary": "still working", "claim_lease_id": lease_id}))
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
async fn fencing_heartbeat_broadcast_task_without_matching_lease_is_403() {
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
    // Broadcast + an expired lease, but the caller presents NO lease at all
    // and has no relationship to the task whatsoever. Dual-review (2026-08-19,
    // rust-reviewer + security-reviewer, independently) found the original
    // "claim_policy = 'broadcast'" bare disjunct let ANY domain member
    // terminalize/hijack ANY broadcast task — a full ownership bypass, not
    // just a freshness bypass. This must be rejected: broadcast waives
    // freshness (Ruling C1's real fix target), never ownership.
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
    assert_eq!(
        res.status_code(),
        403,
        "an unrelated caller with no lease must not heartbeat someone else's \
         broadcast task just because claim_policy='broadcast': {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_heartbeat_broadcast_task_with_matching_lease_and_expired_freshness_succeeds() {
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
    // re-claimed (claim_task requires status='ready') — the real C1 wedge.
    // Caller presents the row's actual current lease id, proving broadcast
    // correctly waives freshness while still requiring lease-or-bypass proof.
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
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "a broadcast task's expired lease must not block heartbeat when the \
         caller presents the real, current lease id (Ruling C1): {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_complete_broadcast_task_without_matching_lease_is_403() {
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
    assert_eq!(
        res.status_code(),
        403,
        "an unrelated caller with no lease must not be able to complete \
         (hijack/terminalize) someone else's broadcast task: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_complete_broadcast_task_with_matching_lease_and_expired_freshness_succeeds() {
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
        .json(&serde_json::json!({"claim_lease_id": "lease-a"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "a broadcast task's expired lease must not block completion when the \
         caller presents the real, current lease id (Ruling C1): {}",
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

    // The restricted-agent self-identity path: a real, non-full-trust
    // agent-token principal completing a task it genuinely claimed itself,
    // with no lease presented. NOT what task_worker.rs actually does —
    // task_worker.rs authenticates as its node (full-trust), not as a
    // per-agent token (confirmed: no mint_agent_token call anywhere in
    // task_worker.rs); see fencing_complete_node_caller_on_behalf_of_
    // stale_lease_succeeds below for that real shape.
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
async fn fencing_fail_broadcast_task_without_matching_lease_is_403() {
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
    assert_eq!(
        res.status_code(),
        403,
        "an unrelated caller with no lease must not be able to fail \
         (hijack/DoS) someone else's broadcast task: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_fail_broadcast_task_with_matching_lease_and_expired_freshness_succeeds() {
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
        .json(&serde_json::json!({"claim_lease_id": "lease-a", "error": "boom"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "a broadcast task's expired lease must not block failure when the \
         caller presents the real, current lease id (Ruling C1): {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_fail_claimed_by_agent_id_without_lease_succeeds() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // The restricted-agent self-identity path (see complete_task's twin
    // test for why this is NOT actually task_worker.rs's shape — that's
    // fencing_fail_node_caller_on_behalf_of_stale_lease_succeeds below).
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

// ── Adversarial review fix round: Ruling C2's identity path was inert for
// its actual target caller. task_worker.rs authenticates as its NODE
// (full-trust, subject "node:<id>"), never a per-agent token, so comparing
// claimed_by_agent_id against the caller's own principal.subject can never
// match — the caller's identity and the on-behalf-of agent it claimed for
// are two different strings. Fixed by reading ownership back the same
// on-behalf-of way claim_task already writes it (work.rs:1055-1068):
// bypass callers may supply body.agent_id; restricted callers cannot.

#[tokio::test]
async fn fencing_complete_node_caller_on_behalf_of_stale_lease_succeeds() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (s, encoding_key) = server_with_node_signing_key(pool.clone());
    let node_token = seed_node_caller(&pool, &ctx.domain_id, &encoding_key).await;

    // The real task_worker.rs call shape: claimed_by_agent_id is an
    // ephemeral agent id the node claimed on behalf of, never heartbeated,
    // now well past LEASE_TTL_SECS (120s) with no sweep having run.
    let ephemeral_agent_id = format!("agent-{}", uuid::Uuid::new_v4().simple());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some(&ephemeral_agent_id),
        1,
    )
    .await;
    sqlx::query("UPDATE task SET lease_expires_at = now() - interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("force an expired, never-heartbeated lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {node_token}"),
        )
        .json(&serde_json::json!({"agent_id": ephemeral_agent_id}))
        .await;
    assert!(
        res.status_code().is_success(),
        "the real task_worker.rs shape (node caller, on-behalf-of agent_id, \
         no lease, stale lease on file) must complete: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_fail_node_caller_on_behalf_of_stale_lease_succeeds() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (s, encoding_key) = server_with_node_signing_key(pool.clone());
    let node_token = seed_node_caller(&pool, &ctx.domain_id, &encoding_key).await;

    let ephemeral_agent_id = format!("agent-{}", uuid::Uuid::new_v4().simple());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some(&ephemeral_agent_id),
        1,
    )
    .await;
    sqlx::query("UPDATE task SET lease_expires_at = now() - interval '1 hour' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("force an expired, never-heartbeated lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {node_token}"),
        )
        .json(&serde_json::json!({"agent_id": ephemeral_agent_id, "error": "boom"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "the real task_worker.rs shape (node caller, on-behalf-of agent_id, \
         no lease, stale lease on file) must fail the task: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_complete_restricted_caller_cannot_spoof_agent_id_in_body() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // A restricted (non-bypass) agent-token caller claims its OWN task, then
    // tries to complete a DIFFERENT task by naming its real claimer in
    // body.agent_id. The on-behalf-of path must be bypass-gated only —
    // otherwise any compromised agent could spoof any other agent's
    // ownership via this field (the same risk claim_task's own on-behalf-of
    // write already guards against, work.rs:1055-1058).
    let (_caller_agent_id, caller_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let victim_agent_id = format!("agent-{}", uuid::Uuid::new_v4().simple());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some(&victim_agent_id),
        1,
    )
    .await;
    sqlx::query(
        "UPDATE task SET claim_lease_id='victim-lease', lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("seed the victim's live lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {caller_token}"),
        )
        .json(&serde_json::json!({"agent_id": victim_agent_id}))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "a restricted caller must not be able to spoof ownership via body.agent_id: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_fail_kind_assigned_still_works_after_predicate_split() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_assigned_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        ctx.owner_session_subject(),
    )
    .await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"error": "boom"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "kind='assigned' failure must still work after the predicate split: {}",
        res.text()
    );
}

// ── Attribution + idempotent-retry design pass (2026-08-20): finalized_by_subject
// preserves who actually completed/failed a task after claimed_by_agent_id is
// nulled, and classify_fenced_rejection treats "already at this transition's
// target status" as an unconditional 409 rather than losing that signal along
// with the ownership evidence. ──────────────────────────────────────────────

#[tokio::test]
async fn fencing_complete_finalized_by_subject_preserves_claimer_identity() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

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

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert!(
        body["claimed_by_agent_id"].is_null(),
        "claimed_by_agent_id must still be cleared: {body}"
    );
    assert_eq!(
        body["finalized_by_subject"], agent_id,
        "finalized_by_subject must preserve the real claimer's identity: {body}"
    );
}

#[tokio::test]
async fn fencing_fail_finalized_by_subject_preserves_claimer_identity() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

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

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"error": "boom"}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["finalized_by_subject"], agent_id,
        "finalized_by_subject must preserve the real claimer's identity: {body}"
    );
}

#[tokio::test]
async fn fencing_complete_finalized_by_subject_falls_back_to_caller_for_assigned_kind() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // kind='assigned' rows never have claimed_by_agent_id — the fallback
    // (COALESCE's second branch) records the completing caller's own
    // identity instead.
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
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["finalized_by_subject"],
        ctx.owner_session_subject(),
        "assigned-kind completion must fall back to the caller's own identity: {body}"
    );
}

#[tokio::test]
async fn fencing_complete_idempotent_retry_after_success_is_409_not_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // The exact shape that broke: a restricted agent-token caller completes
    // its own claim with no lease presented (the identity-only path). Once
    // claimed_by_agent_id is nulled by that success, a duplicate delivery of
    // the identical request has zero ownership evidence left and must not
    // fall through to 403 — it must still classify as 409 (already done).
    let (_agent_id, agent_token) =
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

    let first = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(first.status_code().is_success(), "{}", first.text());

    let retry = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        retry.status_code(),
        409,
        "an idempotent retry after a successful completion must be 409, not 403: {}",
        retry.text()
    );
}

#[tokio::test]
async fn fencing_fail_idempotent_retry_after_success_is_409_not_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    let (_agent_id, agent_token) =
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

    let first = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"error": "boom"}))
        .await;
    assert!(first.status_code().is_success(), "{}", first.text());

    let retry = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"error": "boom again"}))
        .await;
    assert_eq!(
        retry.status_code(),
        409,
        "an idempotent retry after a successful failure must be 409, not 403: {}",
        retry.text()
    );
}

#[tokio::test]
async fn fencing_complete_already_finished_rejects_unrelated_caller_as_409_not_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // A caller with ZERO relationship to the task, ever, hits an
    // already-finished row. Deliberately universal: "already done" is a
    // state fact, not an authorization fact, so this classifies as 409 the
    // same way an idempotent retry from the real claimer does — the door is
    // closed to everyone equally, not selectively re-opened for a stranger.
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "finished",
        None,
        1,
    )
    .await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "an already-finished task must classify as 409 for any caller, not 403: {}",
        res.text()
    );
}

// ── Task 4: cancel_task — fenced CAS ─────────────────────────────────────────

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
    // owner_session_token is full-trust, so this exercises classify_fenced_
    // rejection's is_full_trust short-circuit, NOT already_done_statuses —
    // "cancelled" isn't even in cancel_task's set (only its own target
    // status is, matching complete_task/fail_task's narrow scoping; see
    // fencing_cancel_idempotent_retry_after_success_is_409_not_403 for the
    // test that actually exercises already_done_statuses via a restricted
    // caller).
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

#[tokio::test]
async fn fencing_cancel_kind_assigned_still_works_after_predicate_split() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_assigned_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        ctx.owner_session_subject(),
    )
    .await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/cancel"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "kind='assigned' cancellation must still work after the fenced rewrite: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_cancel_self_cancel_finalized_by_subject_is_the_agent() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // A self-cancel: canceller == claimer, so this can't distinguish
    // "record the canceller" from "record the claimer" — see
    // fencing_cancel_full_trust_caller_cancelling_different_agents_task_
    // attributes_to_canceller_not_claimer below for the test that does.
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

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/cancel"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert!(body["claimed_by_agent_id"].is_null());
    assert_eq!(body["finalized_by_subject"], agent_id, "{body}");
    assert!(
        body["finalized_at"].is_string(),
        "cancel must stamp finalized_at like every other terminal transition: {body}"
    );
}

#[tokio::test]
async fn fencing_cancel_full_trust_caller_cancelling_different_agents_task_attributes_to_canceller_not_claimer()
 {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // The actual production shape: an operator (full-trust CLI session,
    // edgeplane daemon task cancel) interrupts a DIFFERENT agent's live
    // task. Adversarial review (2026-08-20) caught that recording the
    // claimer here (as complete/fail correctly do) would attribute the
    // cancellation to its victim, not its actor — this proves the fix.
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

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/cancel"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());
    let body: serde_json::Value = res.json();
    assert_ne!(
        body["finalized_by_subject"], agent_id,
        "cancelling someone else's task must not attribute it to the victim claimer: {body}"
    );
    assert_eq!(
        body["finalized_by_subject"],
        ctx.owner_session_subject(),
        "must attribute to the canceller (the actor), not the claimer: {body}"
    );
}

#[tokio::test]
async fn fencing_cancel_after_fail_preserves_original_failure_attribution() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Cancelling an already-failed task is legal (only finished/cancelled
    // are excluded from the claimable branch's status precondition). The
    // agent that failed it already has a real, recorded attribution —
    // a later administrative cancel must not clobber it with the canceller.
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
    let fail_res = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .json(&serde_json::json!({"error": "boom"}))
        .await;
    assert!(fail_res.status_code().is_success(), "{}", fail_res.text());
    assert_eq!(fail_res.json::<serde_json::Value>()["finalized_by_subject"], agent_id);

    let cancel_res = s
        .post(&format!("/api/work/tasks/{task_id}/cancel"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert!(cancel_res.status_code().is_success(), "{}", cancel_res.text());
    let body: serde_json::Value = cancel_res.json();
    assert_eq!(
        body["finalized_by_subject"], agent_id,
        "cancelling an already-failed task must preserve the original failure's \
         attribution, not overwrite it with the canceller: {body}"
    );
}

#[tokio::test]
async fn fencing_cancel_broadcast_task_claimed_by_other_agent_restricted_caller_is_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Cheap insurance against the exact CRITICAL bug class dual-review
    // found on heartbeat/complete/fail (commit 37dca61a): pin that
    // cancel_task's predicate has no claim_policy carve-out at all, so a
    // restricted, unrelated caller cannot hijack-cancel a broadcast task
    // any more than a non-broadcast one.
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "running",
        Some("agent-A"),
        1,
    )
    .await;
    sqlx::query("UPDATE task SET claim_policy='broadcast' WHERE id=$1")
        .bind(&task_id)
        .execute(&pool)
        .await
        .expect("seed a broadcast task");

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
        "claim_policy='broadcast' must not bypass ownership on cancel: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_cancel_idempotent_retry_after_success_is_409_not_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    let (_agent_id, agent_token) =
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

    let first = s
        .post(&format!("/api/work/tasks/{task_id}/cancel"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(first.status_code().is_success(), "{}", first.text());

    let retry = s
        .post(&format!("/api/work/tasks/{task_id}/cancel"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert_eq!(
        retry.status_code(),
        409,
        "an idempotent retry after a successful cancel must be 409, not 403: {}",
        retry.text()
    );
}

// ── Task 5: block_task — net-new fenced precondition + lease release ────────

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
async fn fencing_block_clears_lease_but_preserves_claimer_identity() {
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
    // Deliberately preserved, NOT cleared — an adversarial review
    // (2026-08-20) caught that nulling this locks the blocker out of
    // unblock_task (its only non-bypass ownership proof), and that the
    // original "re-enters the claimable pool" justification for clearing
    // it was factually false (claim_task requires status='ready';
    // expire_stale_leases doesn't sweep 'blocked' rows either).
    assert_eq!(
        row.get::<Option<String>, _>("claimed_by_agent_id"),
        Some("agent-A".to_string()),
        "claimed_by_agent_id must survive block so the blocker can still unblock it"
    );
    assert!(row.get::<Option<String>, _>("claim_lease_id").is_none());
    assert!(row
        .get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at")
        .is_none());
}

#[tokio::test]
async fn fencing_block_real_claimer_non_bypass_succeeds() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // The claimed_by_agent_id = $3 branch specifically, via a restricted
    // (non-bypass) agent-token caller — the existing releases-the-lease
    // test only exercised this through owner_session_token (full-trust),
    // so a broken bind or a broken agent: prefix strip would have kept
    // every prior test green.
    let (_agent_id, agent_token) =
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

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "the real claimer, non-bypass, must be able to block its own task: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_block_idempotent_retry_after_success_is_409_not_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    let (_agent_id, agent_token) =
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

    let first = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(first.status_code().is_success(), "{}", first.text());

    let retry = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert_eq!(
        retry.status_code(),
        409,
        "an idempotent retry after a successful block must be 409, not 403: {}",
        retry.text()
    );
}

#[tokio::test]
async fn fencing_block_kind_assigned_is_rejected() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    // kind='assigned' rows have no claim/lease semantics — block_task is
    // scoped to kind='claimable' only (mirrors retry_task's precedent),
    // so this must fail the predicate entirely, not silently succeed with
    // a no-op lease-release.
    let task_id = common::seed_assigned_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        ctx.owner_session_subject(),
    )
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
        "kind='assigned' rows must be rejected by block_task, not silently blocked: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_block_non_owner_restricted_caller_is_403() {
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
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "a restricted caller with no claim on the task must get 403 on block: {}",
        res.text()
    );
}

// ── Task 6: unblock_task — net-new fenced precondition ──────────────────────

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

#[tokio::test]
async fn fencing_unblock_non_owner_restricted_caller_is_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "blocked",
        Some("agent-someone-else"),
        1,
    )
    .await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "a restricted caller with no claim on the task must get 403 on unblock: {}",
        res.text()
    );
}

#[tokio::test]
async fn fencing_unblock_real_blocker_non_bypass_succeeds_and_preserves_claimer() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // The direct regression test for what Task 5's review found broken:
    // block preserves claimed_by_agent_id specifically so the blocker can
    // unblock its own task via this identity path, non-bypass.
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
    let block_res = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(block_res.status_code().is_success(), "{}", block_res.text());

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(
        res.status_code().is_success(),
        "the real blocker, non-bypass, must be able to unblock its own task: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "ready");
    assert_eq!(
        body["claimed_by_agent_id"], agent_id,
        "unblock deliberately preserves claimed_by_agent_id (resume-your-own-work \
         semantic, unlike retry_task's fresh-start semantic): {body}"
    );
}

#[tokio::test]
async fn fencing_unblock_idempotent_retry_after_success_is_409_not_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    let (_agent_id, agent_token) =
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
    let block_res = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(block_res.status_code().is_success(), "{}", block_res.text());

    let first = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(first.status_code().is_success(), "{}", first.text());

    let retry = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert_eq!(
        retry.status_code(),
        409,
        "an idempotent retry after a successful unblock must be 409, not 403 \
         (resolves via owns_directly — claimed_by_agent_id survives unblock): {}",
        retry.text()
    );
}

#[tokio::test]
async fn fencing_unblock_loses_race_to_concurrent_cancel_is_409() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Concurrent conflicting operation: a blocked task is legally
    // cancellable too (cancel_task's status precondition doesn't exclude
    // 'blocked'). Simulate the race sequentially: cancel wins first, then
    // the blocker's own unblock attempt — which would have succeeded had
    // it run first — must correctly lose, but as 409 (lost a race it had
    // real standing in), not 403 (no standing at all).
    //
    // An earlier version of this test asserted 403 here, reasoning that
    // cancel_task nulling claimed_by_agent_id erased agent-A's ownership
    // evidence. An independent review (2026-08-20) caught that this was
    // wrong on both counts it rested on: (1) cancel_task's own
    // finalized_by_subject=COALESCE(finalized_by_subject, $subject) (see
    // 79d0c493) means agent-A's identity survives its own cancel in a
    // column classify_fenced_rejection simply wasn't reading — the
    // evidence was never actually erased; (2) the claimed parallel to
    // complete_task/fail_task doesn't hold — their real callers are either
    // a full-trust node (task_worker.rs, hits the unconditional bypass) or
    // always carry a claim_lease_id (task_loop.rs/edgeplaned-work, hits
    // the lease_id.is_some() escape hatch), so neither actually exercises
    // the "no evidence at all" dead branch block_task/unblock_task's real
    // callers can hit, being the only two lease-less endpoints. Fixed by
    // adding finalized_by_subject to classify_fenced_rejection's read.
    let (_agent_id, agent_token) =
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
    let block_res = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(block_res.status_code().is_success(), "{}", block_res.text());

    let cancel_res = s
        .post(&format!("/api/work/tasks/{task_id}/cancel"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert!(
        cancel_res.status_code().is_success(),
        "a blocked task must be legally cancellable: {}",
        cancel_res.text()
    );
    assert_eq!(cancel_res.json::<serde_json::Value>()["status"], "cancelled");

    let unblock_res = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .await;
    assert_eq!(
        unblock_res.status_code(),
        409,
        "unblock must correctly lose to a cancel that already moved the row \
         out of 'blocked', classified as 409 since agent-A's identity survives \
         via finalized_by_subject, not a bare 403: {}",
        unblock_res.text()
    );
}

#[tokio::test]
async fn fencing_unblock_stale_actor_after_reclaim_is_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());

    // Category (1), stale-actor retry — the one required test category an
    // independent review (2026-08-20) found missing from this task's
    // original six: A claims, blocks, and successfully unblocks (status
    // 'ready', claimed_by_agent_id deliberately still A per this task's own
    // design). B then claims the now-ready row, overwriting claimed_by_
    // agent_id to B. A's client, unaware its unblock already succeeded
    // (e.g. a lost response), retries the identical unblock call.
    //
    // Correctly 403, NOT the 409 the sibling concurrent-cancel test now
    // asserts: this is the genuinely harder case that fix doesn't reach.
    // unblock_task never writes finalized_by_subject (it isn't a
    // terminal/attribution transition), and B's claim overwrites
    // claimed_by_agent_id outright — so nothing on the row points to A
    // anymore, unlike the cancel case where A's identity survived in a
    // column the classifier just wasn't reading. A's retry presents no
    // evidence in the request itself either (unblock has never accepted a
    // lease or on-behalf-of param) — there is no server-side signal left
    // to distinguish "A, legitimately retrying" from "a stranger polling
    // this task_id at random." Per spec's own 403-vs-409 rule (only a
    // caller who presented zero proof and isn't full-trust gets 403), zero
    // surviving proof is exactly the 403 case, and lease-based leniency
    // (why a stale-but-real lease still gets 409 elsewhere) does not apply
    // here since this endpoint never had a lease to be stale in the first
    // place.
    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let (_agent_b_id, agent_b_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let task_id =
        common::seed_claimable_task(&pool, &ctx.mission_id, &ctx.domain_id, "ready", None, 2)
            .await;

    let claim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(claim_res.status_code().is_success(), "{}", claim_res.text());
    let block_res = s
        .post(&format!("/api/work/tasks/{task_id}/block"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .await;
    assert!(block_res.status_code().is_success(), "{}", block_res.text());
    let first_unblock = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .await;
    assert!(first_unblock.status_code().is_success(), "{}", first_unblock.text());
    assert_eq!(
        first_unblock.json::<serde_json::Value>()["claimed_by_agent_id"],
        agent_a_id
    );

    let reclaim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_b_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert!(reclaim_res.status_code().is_success(), "{}", reclaim_res.text());

    let retry_unblock = s
        .post(&format!("/api/work/tasks/{task_id}/unblock"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .await;
    assert_eq!(
        retry_unblock.status_code(),
        403,
        "a stale retry with zero surviving evidence, after a legitimate reclaim \
         by a different agent, must be 403: {}",
        retry_unblock.text()
    );
}

// ── Task 7: resolve_gate — bespoke fenced transaction ───────────────────────
//
// Zero pre-existing coverage of this endpoint anywhere in this file before
// this task (verified by grep before writing these). Unlike every other
// endpoint in this plan, resolve_gate does not call classify_fenced_rejection
// (ownership is gate ownership, not task lease) — it gets its own fenced
// gate-row CAS plus a fenced task-transition CAS, per the plan's "bespoke
// transaction" framing.

async fn seed_pending_gate(pool: &sqlx::PgPool, owner_subject: &str, task_id: &str) -> String {
    let gate_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO reviewgate (id, owner_subject, mesh_task_id, run_id, gate_type, \
         required_approvals, status, created_at) \
         VALUES ($1, $2, $3, NULL, 'manual', 'any', 'pending', now())",
    )
    .bind(&gate_id)
    .bind(owner_subject)
    .bind(task_id)
    .execute(pool)
    .await
    .expect("seed pending gate");
    gate_id
}

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
    let gate_id = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

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
    let gate_id = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

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
        "resolve_gate's rejection path currently clears NOTHING pre-fix — spec §1 finding"
    );
    assert!(row.get::<Option<String>, _>("claim_lease_id").is_none());
    assert!(row
        .get::<Option<chrono::NaiveDateTime>, _>("lease_expires_at")
        .is_none());
}

#[tokio::test]
async fn fencing_resolve_gate_approved_stamps_finalized_by_subject_and_finalized_at() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "waiting_review",
        Some(&agent_a_id),
        1,
    )
    .await;
    let gate_id = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

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
        "SELECT finalized_by_subject, finalized_at FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row.get::<Option<String>, _>("finalized_by_subject").as_deref(),
        Some(agent_a_id.as_str()),
        "resolve_gate must attribute finalization to the task's actual claimer \
         (complete_task/fail_task's 'record the claimer' rationale — this finalizes \
         the claimer's own submitted work, it doesn't interrupt someone else's), not \
         the gate resolver — roadmap item 'resolve_gate inherits the attribution + \
         idempotent-retry pattern'"
    );
    assert!(
        row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("finalized_at").is_some(),
        "resolve_gate is the one terminal-transition endpoint in this codebase that \
         never stamped finalized_at (tasks.rs, mcp.rs, and every other work.rs \
         terminal transition all do) — closing that gap while touching this code"
    );
    let _ = agent_a_token; // only the id is needed for this test
}

#[tokio::test]
async fn fencing_resolve_gate_rejected_stamps_finalized_by_subject_and_finalized_at() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let (agent_a_id, _agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "waiting_review",
        Some(&agent_a_id),
        1,
    )
    .await;
    let gate_id = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "rejected"}))
        .await;
    assert!(res.status_code().is_success(), "{}", res.text());

    let row = sqlx::query("SELECT finalized_by_subject, finalized_at FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        row.get::<Option<String>, _>("finalized_by_subject").as_deref(),
        Some(agent_a_id.as_str())
    );
    assert!(row
        .get::<Option<chrono::DateTime<chrono::Utc>>, _>("finalized_at")
        .is_some());
}

#[tokio::test]
async fn fencing_resolve_gate_non_owner_restricted_caller_is_403() {
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
    // Owned by the harness/owner session, not the restricted member token.
    let gate_id = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "a caller who isn't the gate's owner and isn't admin must be 403, even \
         though the fenced UPDATE's WHERE clause (not an app-level precheck) is what \
         now rejects it: {}",
        res.text()
    );

    let gate_status: String = sqlx::query_scalar("SELECT status FROM reviewgate WHERE id=$1")
        .bind(&gate_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(gate_status, "pending", "the fenced UPDATE must not have touched the row");
}

/// Idempotent retry (three-test-minimum category 3): the SAME caller repeats
/// an already-successful resolve on the SAME gate. Must be 409, not a second
/// success and not a silent overwrite.
///
/// Honest scope note (corrected after independent review): this is a
/// SEQUENTIAL retry, and a sequential retry already got 409 pre-fix too, via
/// the old app-level `if gate_status != "pending"` check — so this test does
/// NOT by itself distinguish the new fenced `WHERE status='pending'` CAS
/// from the app-level check it replaced (both give 409 here; both also
/// happened to compare `finalized_by_subject` as `None == None` pre-fix,
/// since that column didn't exist yet). It's a permanent regression guard
/// for the idempotent-retry category the plan's three-test-minimum
/// requires, not proof of the concurrent TOCTOU fix — proving that requires
/// genuine concurrent request timing, which this file's integration tests
/// (via `axum_test::TestServer`, no `tokio::join!` precedent anywhere in
/// this suite) don't exercise for ANY of this plan's "race" tests, not just
/// this one. The TOCTOU fix itself is argued from Postgres's documented
/// EvalPlanQual re-check of an UPDATE's WHERE clause after a row-lock wait
/// (see the doc comment on resolve_gate's reviewgate UPDATE).
#[tokio::test]
async fn fencing_resolve_gate_idempotent_retry_same_decision_is_409() {
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
    let gate_id = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

    let first = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert!(first.status_code().is_success(), "{}", first.text());
    let finalized_by_after_first: Option<String> =
        sqlx::query_scalar("SELECT finalized_by_subject FROM task WHERE id=$1")
            .bind(&task_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let retry = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert_eq!(
        retry.status_code(),
        409,
        "retrying an identical, already-successful resolve must be 409, not 200 \
         again and not silently accepted: {}",
        retry.text()
    );

    let finalized_by_after_retry: Option<String> =
        sqlx::query_scalar("SELECT finalized_by_subject FROM task WHERE id=$1")
            .bind(&task_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        finalized_by_after_first, finalized_by_after_retry,
        "attribution from the first, real resolution must survive the retry unchanged"
    );
}

/// Concurrent conflicting operation (three-test-minimum category 2): two
/// DIFFERENT, both-legitimately-pending gates on the SAME task, resolved in
/// sequence — the first resolution's task-side transition (any_rejected →
/// failed) must be the only one that ever lands. The second gate's own
/// resolution still succeeds (its row genuinely was still 'pending'), but the
/// `AND task.status='waiting_review'` CAS guard baked into the task UPDATE's
/// WHERE clause must make its stale recompute a no-op rather than
/// double-firing finalized_at/finalized_by_subject or flipping the task's
/// terminal outcome.
///
/// This genuinely exercises the SQL-level CAS, not just an app-level
/// short-circuit (independent review, second pass — the first version of
/// this function had an app-level `if task_status == "waiting_review"`
/// BEFORE ever attempting the task UPDATE, so this test's second call never
/// even reached the guarded statement; that app-level branch is gone now —
/// the CTE-UPDATE's own WHERE clause is the only thing deciding whether
/// gate2's stale recompute touches the row, and gate2's request really does
/// call it).
#[tokio::test]
async fn fencing_resolve_gate_second_gates_recompute_does_not_reprocess_finalized_task() {
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
    let gate1 = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;
    let gate2 = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

    let reject_res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate1}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "rejected"}))
        .await;
    assert!(reject_res.status_code().is_success(), "{}", reject_res.text());

    let row_after_first = sqlx::query(
        "SELECT status, finalized_at, finalized_by_subject FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row_after_first.get::<String, _>("status"), "failed");
    let finalized_at_first: chrono::DateTime<chrono::Utc> =
        row_after_first.get("finalized_at");

    // gate2 is still legitimately pending — its own resolution must succeed —
    // but the task is already terminal, so this call's recompute must not
    // touch it.
    let approve_res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate2}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert!(
        approve_res.status_code().is_success(),
        "gate2 itself was genuinely still pending, its own resolution must succeed: {}",
        approve_res.text()
    );

    let row_after_second = sqlx::query(
        "SELECT status, finalized_at, finalized_by_subject FROM task WHERE id=$1",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row_after_second.get::<String, _>("status"),
        "failed",
        "a later, unrelated gate's own successful resolution must not flip an \
         already-finalized task's outcome"
    );
    assert_eq!(
        row_after_second.get::<chrono::DateTime<chrono::Utc>, _>("finalized_at"),
        finalized_at_first,
        "finalized_at must not be re-stamped by the second gate's stale recompute"
    );
}

/// Stale-actor retry (three-test-minimum category 1): after resolve_gate
/// finalizes a task, the ORIGINAL CLAIMER (now stripped of
/// claimed_by_agent_id) retries a different terminal endpoint on the
/// now-finished task and must get 409, not 403 — proving resolve_gate's
/// finalized_by_subject stamp is actually read by classify_fenced_rejection
/// (the exact cross-endpoint blind spot Task 6's review caught: a column
/// written by one endpoint but not read by the shared classifier).
///
/// Deliberately uses `fail_task`, not `complete_task` (corrected after
/// independent review): `complete_task`'s `already_done_statuses=["finished"]`
/// matches this task's post-resolve status directly, so
/// `classify_fenced_rejection`'s `if already_done { return conflict(...) }`
/// short-circuits BEFORE ever reaching the `finalized_by_subject` read below
/// it — a complete_task-based version of this test would get 409 for the
/// wrong reason and prove nothing about the attribution stamp.
/// `fail_task`'s `already_done_statuses=["failed"]` does NOT match
/// 'finished', so it falls through to the identity check and genuinely
/// exercises `finalized_by_subject`.
#[tokio::test]
async fn fencing_resolve_gate_stale_claimer_retry_via_fail_is_409_not_403() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "waiting_review",
        Some(&agent_a_id),
        1,
    )
    .await;
    let gate_id = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

    let resolve_res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert!(resolve_res.status_code().is_success(), "{}", resolve_res.text());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM task WHERE id=$1")
            .bind(&task_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "finished"
    );

    // Agent A never saw the gate resolve (e.g. its poll of the task raced
    // the resolution) and retries an endpoint whose already_done_statuses
    // doesn't match 'finished', so this must fall through to the identity
    // check rather than short-circuit on the already-done fast path.
    let stale_fail = s
        .post(&format!("/api/work/tasks/{task_id}/fail"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .json(&serde_json::json!({"error": "stale retry"}))
        .await;
    assert_eq!(
        stale_fail.status_code(),
        409,
        "agent A's identity survives in finalized_by_subject after resolve_gate \
         finalizes the task, so a stale retry must be 409 (lost a race), not 403 \
         (zero proof) — and fail_task's WHERE clause doesn't match a 'finished' \
         row, and its already_done_statuses=[\"failed\"] doesn't short-circuit on \
         it either, so a 409 here can only come from the finalized_by_subject \
         identity check: {}",
        stale_fail.text()
    );
}

/// Coverage gap closed after independent review: a caller who isn't the
/// gate's owner but IS admin must still bypass via the `(owner_subject=$5 OR
/// $6)` disjunct in the fenced UPDATE's WHERE clause — the `$6` arm had zero
/// test coverage anywhere in this suite (see `server_with_admin`'s doc
/// comment: `is_admin=true` is otherwise never exercised in this plan,
/// since every other bypass test relies on `is_full_trust` via session/node
/// auth instead).
#[tokio::test]
async fn fencing_resolve_gate_admin_bypass_succeeds_for_non_owner() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (s, admin_token) = server_with_admin(pool.clone(), &pool).await;
    let task_id = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "waiting_review",
        Some("agent-A"),
        1,
    )
    .await;
    // Owned by a subject that is neither the admin session nor the member
    // session used below — the admin path must not depend on owner match.
    let gate_id = seed_pending_gate(&pool, "someone-else@example.com", &task_id).await;

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {admin_token}"),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "the owner session is full-trust/admin per this harness's setup(), \
         so it must bypass the owner-mismatch check: {}",
        res.text()
    );
}

/// Coverage gap closed after independent review: a `gate_id` that exists but
/// doesn't belong to the `task_id` in the path must 404, matching the
/// pre-fix behavior's explicit `mesh_task_id=$2` join condition.
#[tokio::test]
async fn fencing_resolve_gate_task_id_mismatch_is_404() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let task_a = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "waiting_review",
        Some("agent-A"),
        1,
    )
    .await;
    let task_b = common::seed_claimable_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "waiting_review",
        Some("agent-B"),
        1,
    )
    .await;
    let gate_on_a = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_a).await;

    let res = s
        .post(&format!("/api/work/tasks/{task_b}/gates/{gate_on_a}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert_eq!(
        res.status_code(),
        404,
        "a gate that exists but doesn't belong to the task in the path must \
         404, not silently resolve against the wrong task: {}",
        res.text()
    );
}

/// Ordering note (flagged by independent review, L4): resolving an
/// already-resolved gate as a non-owner now classifies as 409 ("Gate is
/// already X") rather than 403 ("not authorized"), a deliberate change from
/// the pre-fix code's owner-check-before-status-check order. This matches
/// classify_fenced_rejection's own stated rationale elsewhere in this file —
/// a row already at its target status is a state fact, independent of who's
/// asking — and leaks nothing list_gates doesn't already expose to any
/// domain member. Locking in the new order explicitly rather than leaving it
/// as an untested side effect of the fence.
#[tokio::test]
async fn fencing_resolve_gate_non_owner_on_already_resolved_gate_is_409_not_403() {
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
    let gate_id = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

    let resolve_res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert!(resolve_res.status_code().is_success(), "{}", resolve_res.text());

    let retry_as_non_owner = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate_id}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert_eq!(
        retry_as_non_owner.status_code(),
        409,
        "a non-owner resolving an already-resolved gate must be 409 (state \
         fact) not 403 (authz fact): {}",
        retry_as_non_owner.text()
    );
}

/// Baseline multi-gate happy path for the rewritten CTE aggregate
/// (`bool_or(status='rejected')`/`bool_and(status IN ('approved','expired'))`
/// replacing the original Rust-side `.any()`/`.all()` over a separately
/// fetched Vec) — not a race test, just correctness coverage for logic that
/// was completely rewritten and, before this task, had zero tests of any
/// kind on this endpoint. Two pending gates: resolving only one must leave
/// the task in `waiting_review` (not finish early); resolving the second
/// must then finish it and fire `unblock_dependents`.
#[tokio::test]
async fn fencing_resolve_gate_all_must_resolve_before_task_finishes() {
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
    let gate1 = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;
    let gate2 = seed_pending_gate(&pool, ctx.owner_session_subject(), &task_id).await;

    let first_res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate1}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert!(first_res.status_code().is_success(), "{}", first_res.text());

    let status_after_first: String = sqlx::query_scalar("SELECT status FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status_after_first, "waiting_review",
        "one of two gates approved must not finish the task early — bool_and \
         over ('approved','pending') must be false"
    );

    let second_res = s
        .post(&format!("/api/work/tasks/{task_id}/gates/{gate2}/resolve"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({"decision": "approved"}))
        .await;
    assert!(second_res.status_code().is_success(), "{}", second_res.text());

    let status_after_second: String = sqlx::query_scalar("SELECT status FROM task WHERE id=$1")
        .bind(&task_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        status_after_second, "finished",
        "both gates approved must finish the task — bool_and over \
         ('approved','approved') must be true"
    );
}

// ── Task 8: append_progress — required lease, fenced insert ─────────────────

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

    // Correct lease — must succeed. Deliberately uses member_sa_token, NOT
    // owner_session_token (corrected after independent review: owner_
    // session_token is a session-auth principal, so is_full_trust=true makes
    // the predicate's `$3` bypass arm satisfy it regardless of what lease
    // value is presented — that leg would pass identically with a wrong or
    // missing lease and prove nothing about lease matching). member_sa_token
    // is restricted and isn't "agent-A" either, so success here can only
    // come from the lease actually matching.
    let ok_res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "with lease", "claim_lease_id": "lease-a"}))
        .await;
    assert!(ok_res.status_code().is_success(), "{}", ok_res.text());

    // Stale/wrong lease — must be rejected. 409, not just "any non-success":
    // classify_fenced_rejection treats a presented (even wrong) lease as
    // ownership proof, so this is a lost race (409), not zero-proof (403).
    let bad_res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "wrong lease", "claim_lease_id": "not-it"}))
        .await;
    assert_eq!(
        bad_res.status_code(),
        409,
        "progress with a wrong-but-presented lease must be 409 (lost a \
         race), not silently accepted or misclassified as 403: {}",
        bad_res.text()
    );
}

/// Second pass (independent rust-reviewer, Task 8): the first version of
/// append_progress's fenced predicate had `claim_policy = 'broadcast'` as a
/// bare top-level OR disjunct spanning the ENTIRE ownership+lease clause —
/// the exact CRITICAL bug commit 37dca61a already fixed in heartbeat_task/
/// complete_task/fail_task, reintroduced here by copying this plan's own
/// stale Task 8 text (never updated after 37dca61a) verbatim. No test in
/// this suite caught it: every test in this file seeds via
/// seed_claimable_task, which hardcodes claim_policy='any'
/// (tests/common/mod.rs), so the broadcast branch was never evaluated by
/// any prior progress test. Mirrors the sibling regression tests 37dca61a
/// added for heartbeat/complete/fail/cancel.
#[tokio::test]
async fn fencing_progress_broadcast_task_without_matching_lease_is_403() {
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
    // Broadcast + an expired lease, but the caller presents NO lease at all
    // and has no relationship to the task whatsoever.
    sqlx::query(
        "UPDATE task SET claim_policy='broadcast', claim_lease_id='lease-a', \
         lease_expires_at = now() - interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("seed a broadcast task with an expired lease");

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "hijack", "claim_lease_id": ""}))
        .await;
    // An empty claim_lease_id hits the 400 "required" guard before the fence
    // even runs — use a non-empty, definitely-wrong value to actually
    // exercise the predicate.
    assert_eq!(
        res.status_code(),
        400,
        "sanity: an empty lease must 400 before reaching the fence: {}",
        res.text()
    );

    let res = s
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "hijack", "claim_lease_id": "not-the-real-lease"}))
        .await;
    assert_eq!(
        res.status_code(),
        409,
        "an unrelated caller presenting a lease that doesn't match must not \
         post progress to someone else's broadcast task just because \
         claim_policy='broadcast' — 409 (lease presented, but wrong), not a \
         silent 200: {}",
        res.text()
    );

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM meshprogressevent WHERE task_id=$1")
            .bind(&task_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "no progress event must have been inserted"
    );
}

/// Positive counterpart: broadcast correctly waives FRESHNESS, never
/// ownership — a caller presenting the row's actual current lease must
/// still succeed even though the lease has expired (the real point of the
/// broadcast carve-out: a broadcast task's lease is never auto-reclaimed by
/// expire_stale_leases and can never be re-claimed either, so it must stay
/// operable past one lease window for whoever's actually still working it).
#[tokio::test]
async fn fencing_progress_broadcast_task_with_matching_lease_and_expired_freshness_succeeds() {
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
        .post(&format!("/api/work/tasks/{task_id}/progress"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({"event_type": "status", "summary": "still working", "claim_lease_id": "lease-a"}))
        .await;
    assert!(
        res.status_code().is_success(),
        "a broadcast task's expired lease must not block progress-posting \
         when the caller presents the real, current lease id: {}",
        res.text()
    );
}
