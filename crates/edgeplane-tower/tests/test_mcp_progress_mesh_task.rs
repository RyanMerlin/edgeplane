//! Regression test: the `progress_mesh_task` MCP tool must actually insert a
//! `meshprogressevent` row. It previously omitted `seq` (NOT NULL, no DB
//! default — computed application-side in both this handler and the REST
//! `post_progress` handler in `routes/work.rs`), so every call 500'd with a
//! `database_error`.

mod common;

use axum_test::TestServer;
use common::{seed_ready_task, setup};
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

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

    let seqs: Vec<i32> =
        sqlx::query_scalar("SELECT seq FROM meshprogressevent WHERE task_id = $1 ORDER BY seq")
            .bind(&task_id)
            .fetch_all(&pool)
            .await
            .expect("fetch progress events");
    assert_eq!(
        seqs,
        vec![0, 1, 2],
        "seq must be sequential per task, not null/duplicated"
    );
}
