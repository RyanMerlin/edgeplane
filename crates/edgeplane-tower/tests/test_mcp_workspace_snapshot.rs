//! Regression tests: `load_mission_workspace`'s task snapshot must reflect
//! the live, agent-claimable `meshtask` table, not the disconnected legacy
//! `task` table (the two have zero synchronization — see
//! docs/superpowers/plans/2026-07-16-fix-mcp-workspace-snapshot-meshtask.md).

mod common;

use axum_test::TestServer;
use common::{seed_ready_task, setup};
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;
use uuid::Uuid;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

/// Insert a meshtask row with an explicit status (any value, including
/// terminal states not covered by the shared `common::seed_*` helpers).
async fn seed_task_with_status(db: &PgPool, mission_id: &str, domain_id: &str, status: &str) -> String {
    let task_id = format!("task-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO meshtask \
         (id, mission_id, domain_id, title, description, input_json, claim_policy, \
          depends_on, produces, consumes, required_capabilities, \
          status, priority, version_counter, created_by_subject, \
          created_at, updated_at) \
         VALUES ($1, $2, $3, 'test-task', '', '{}', 'any', '[]', '{}', '{}', '[]', \
                 $4, 0, 1, 'harness', now(), now())",
    )
    .bind(&task_id)
    .bind(mission_id)
    .bind(domain_id)
    .bind(status)
    .execute(db)
    .await
    .expect("insert meshtask with status");
    task_id
}

#[tokio::test]
async fn load_mission_workspace_reflects_meshtask_not_legacy_task() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let task_id = seed_ready_task(&pool, &ctx.mission_id, &ctx.domain_id).await;

    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "load_mission_workspace",
            "args": { "mission_id": ctx.mission_id }
        }))
        .await;

    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["ok"], true, "response body: {body}");

    let tasks = body["result"]["workspace_snapshot"]["tasks"]
        .as_array()
        .expect("tasks must be an array");
    assert_eq!(
        tasks.len(),
        1,
        "expected exactly the one seeded meshtask; snapshot must read from \
         meshtask, not the disconnected task table: {body}"
    );
    assert_eq!(tasks[0]["id"], task_id);
    assert_eq!(tasks[0]["status"], "ready");
}

#[tokio::test]
async fn load_mission_workspace_excludes_terminal_meshtasks() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let ready_id = seed_ready_task(&pool, &ctx.mission_id, &ctx.domain_id).await;
    let _finished_id = seed_task_with_status(&pool, &ctx.mission_id, &ctx.domain_id, "finished").await;
    let _cancelled_id = seed_task_with_status(&pool, &ctx.mission_id, &ctx.domain_id, "cancelled").await;

    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "load_mission_workspace",
            "args": { "mission_id": ctx.mission_id }
        }))
        .await;

    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    let tasks = body["result"]["workspace_snapshot"]["tasks"]
        .as_array()
        .expect("tasks must be an array");
    assert_eq!(
        tasks.len(),
        1,
        "only the non-terminal (ready) task should appear: {body}"
    );
    assert_eq!(tasks[0]["id"], ready_id);
}
