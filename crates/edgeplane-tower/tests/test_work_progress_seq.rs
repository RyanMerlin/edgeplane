//! Regression test: `POST /work/tasks/{id}/progress` (routes/work.rs's
//! `post_progress` handler — the same one `edgeplaned-work`'s task.rs client
//! calls) must assign sequential `seq` values per task. It previously decoded
//! the `MAX(seq)+1` query as `i64` against a Postgres `integer` (i32) column;
//! sqlx's runtime type check silently failed every time and fell through to
//! `unwrap_or(0)` — every progress event ever posted got seq=0, regardless of
//! how many prior events existed for the task.

mod common;

use axum_test::TestServer;
use common::{seed_ready_task, setup};
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

#[tokio::test]
async fn post_progress_assigns_sequential_seq() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // append_progress now requires the task be claimed/running and a live
    // lease presented (EP-1 §1) — a bare 'ready' task with no lease no
    // longer qualifies (see fencing_progress_requires_lease_now in
    // test_task_kind_unification.rs for the dedicated fencing coverage;
    // this test is purely about the seq-increment regression it was
    // written for, so a fixed lease value that satisfies the fence is
    // enough, not a real claim/heartbeat cycle).
    let task_id = seed_ready_task(&pool, &ctx.mission_id, &ctx.domain_id).await;
    sqlx::query(
        "UPDATE task SET status='running', claim_lease_id='lease-seq-test', lease_expires_at = now() + interval '1 hour' WHERE id=$1",
    )
    .bind(&task_id)
    .execute(&pool)
    .await
    .expect("seed a live lease");
    let s = server(pool.clone());

    for i in 0..3 {
        let res = s
            .post(&format!("/api/work/tasks/{task_id}/progress"))
            .add_header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {}", ctx.owner_session_token),
            )
            .json(&serde_json::json!({
                "event_type": "phase_finished",
                "summary": format!("iteration {i}"),
                "claim_lease_id": "lease-seq-test",
            }))
            .await;
        res.assert_status_ok();
    }

    let seqs: Vec<i32> = sqlx::query_scalar(
        "SELECT seq FROM meshprogressevent WHERE task_id = $1 ORDER BY id",
    )
    .bind(&task_id)
    .fetch_all(&pool)
    .await
    .expect("fetch progress events");
    assert_eq!(seqs, vec![0, 1, 2], "seq must be sequential per task, not stuck at 0");
}
