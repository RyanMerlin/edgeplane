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
use sqlx::PgPool;

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

/// Task 6 review follow-up: the controller's amendment to `TaskTransition::Block`
/// (`OR claim_lease_id = $5` in the fence, restoring `edgeplane mesh task block
/// --claim-lease-id`'s real capability) had zero test coverage — deleting that
/// clause from the fence would leave every existing test green. This proves the
/// lease branch is load-bearing: `ctx.member_sa_token` is neither the task's
/// claimer (`agent-A`) nor a bypass principal (`is_full_trust` is false for
/// `auth_type="service_account"`, unlike `ctx.owner_session_token`'s `"session"`
/// type, which would pass via the bypass branch regardless of the lease branch's
/// correctness). The only way this call can succeed is via `claim_lease_id = $5`.
#[tokio::test]
async fn mcp_block_via_matching_lease_succeeds_for_non_owner_non_bypass_caller() {
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
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "block_mesh_task",
            "args": {"task_id": task_id, "claim_lease_id": "lease-a"}
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["ok"], true,
        "a non-owner, non-bypass caller presenting the task's real, live claim_lease_id must be able to block it via MCP: {body}"
    );
    assert_eq!(
        body["result"]["status"], "blocked",
        "block must report the task as blocked: {body}"
    );

    // Block deliberately preserves claimed_by_agent_id (Task 5's identity-
    // preserving behavior, restated in the fence's inline comment) — confirm
    // the lease-authorized caller didn't accidentally overwrite ownership.
    let claimed_by: Option<String> =
        sqlx::query_scalar("SELECT claimed_by_agent_id FROM task WHERE id=$1")
            .bind(&task_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        claimed_by.as_deref(),
        Some("agent-A"),
        "block must preserve claimed_by_agent_id, not overwrite it with the lease-authorized caller's identity"
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
