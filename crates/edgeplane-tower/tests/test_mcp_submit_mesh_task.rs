//! Regression test: the `submit_mesh_task` MCP tool must actually insert a
//! `meshtask` row. It previously omitted `claim_policy` (NOT NULL, no DB
//! default — see `crates/edgeplane-tower/migrations/0001_initial_schema.sql`)
//! from its INSERT, so every call 500'd with a `database_error`.

mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

#[tokio::test]
async fn submit_mesh_task_inserts_a_claimable_meshtask() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };

    let s = server(pool.clone());
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "submit_mesh_task",
            "args": {
                "mission_id": ctx.mission_id,
                "title": "mcp-submitted-task",
                "description": "created via submit_mesh_task",
            }
        }))
        .await;

    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["ok"], true, "response body: {body}");
    let task_id = body["result"]["task_id"]
        .as_str()
        .expect("task_id must be a string")
        .to_string();

    // Prove the row actually landed with a valid, non-null claim_policy —
    // not just that the handler returned 200 while the INSERT silently no-op'd.
    // `meshtask` was renamed to `task` by migration 0014 (task/meshtask
    // unification); claim_policy is nullable post-unification (NULL for
    // kind='assigned' rows), but this row is kind='claimable' and must still
    // carry a real value.
    let (claim_policy, status): (Option<String>, String) = sqlx::query_as(
        "SELECT claim_policy, status FROM task WHERE id = $1 AND kind = 'claimable'",
    )
    .bind(&task_id)
    .fetch_one(&pool)
    .await
    .expect("submitted meshtask row must exist");
    let claim_policy = claim_policy.expect("claimable row must have a non-null claim_policy");
    assert_eq!(claim_policy, "first_claim");
    assert_eq!(status, "ready");

    // And that it's actually claimable through the normal /work path — the
    // point of creating it in the first place.
    let claim_res = s
        .post(&format!("/api/work/tasks/{task_id}/claim"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    claim_res.assert_status_ok();
}
