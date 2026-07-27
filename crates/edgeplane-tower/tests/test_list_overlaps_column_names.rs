//! Regression test: `GET /domains/{d}/m/{m}/t/{t}/overlaps` must actually
//! return overlap suggestions instead of 500ing. The handler queried
//! `overlapsuggestion` for columns `score`/`reason` and ordered by `score` —
//! none of which exist (`crates/edgeplane-tower/migrations/0001_initial_schema.sql`
//! has `similarity_score`/`evidence`/`suggested_action`, all NOT NULL). Every
//! call to this route has 500'd with "column does not exist" since it was
//! written; the MCP `get_overlap_suggestions` handler (routes/mcp.rs) already
//! used the correct column names for the same table.

mod common;

use axum_test::TestServer;
use common::{seed_overlap_suggestion, seed_task, setup};
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

#[tokio::test]
async fn list_overlaps_returns_similarity_score_evidence_suggested_action() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };

    let task_id = seed_task(&pool, &ctx.mission_id).await;
    seed_overlap_suggestion(&pool, &task_id).await;

    let s = server(pool);
    let res = s
        .get(&format!(
            "/api/domains/{}/m/{}/t/{}/overlaps",
            ctx.domain_id, ctx.mission_id, task_id
        ))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;

    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    let overlaps = body.as_array().expect("result must be array");
    assert_eq!(overlaps.len(), 1, "response body: {body}");
    assert_eq!(overlaps[0]["similarity_score"], serde_json::json!(0.9));
    assert_eq!(overlaps[0]["evidence"], serde_json::json!("test evidence"));
    assert_eq!(overlaps[0]["suggested_action"], serde_json::json!("merge"));
    assert!(overlaps[0].get("score").is_none(), "response body: {body}");
    assert!(overlaps[0].get("reason").is_none(), "response body: {body}");
}
