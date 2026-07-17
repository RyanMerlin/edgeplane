//! Regression test: the `list_mesh_messages` MCP tool must not panic when a
//! `meshmessage.body_json` row is NULL. `body_json` is nullable `text`
//! (`crates/edgeplane-tower/migrations/0001_initial_schema.sql`), but the
//! handler decoded it as non-Option `String` via `Row::get`, which panics on
//! any NULL row — the same failure mechanism (and same table) as the
//! `row_to_message` bug already fixed in #113, just a separate MCP-tool code
//! path #113 didn't reach. Every current INSERT path happens to always
//! supply a body_json string, so this never fires from live traffic today,
//! but the column is nullable per schema and legacy/manually-inserted rows
//! are exactly the case `Row::get` panics on instead of erroring gracefully.

mod common;

use axum_test::TestServer;
use common::{seed_agent, setup};
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

#[tokio::test]
async fn list_mesh_messages_does_not_panic_on_null_body_json() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };

    let agent_id = format!("agent-{}", uuid::Uuid::new_v4().simple());
    seed_agent(&pool, &ctx.domain_id, &agent_id).await;

    // Insert directly via SQL (bypassing the app's own INSERT paths, which
    // always populate body_json) to reproduce the NULL row this schema
    // permits but the application code never currently produces itself.
    sqlx::query(
        "INSERT INTO meshmessage \
         (domain_id, from_agent_id, to_agent_id, channel, body_json, created_at) \
         VALUES ($1, 'harness', $2, 'coordination', NULL, now())",
    )
    .bind(&ctx.domain_id)
    .bind(&agent_id)
    .execute(&pool)
    .await
    .expect("insert meshmessage with NULL body_json");

    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({
            "tool": "list_mesh_messages",
            "args": { "agent_id": agent_id, "limit": 100 }
        }))
        .await;

    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["ok"], true, "response body: {body}");
    let messages = body["result"].as_array().expect("result must be array");
    assert_eq!(messages.len(), 1, "response body: {body}");
    // body_json must decode to something (empty string default), not crash the request.
    assert_eq!(messages[0]["body_json"], serde_json::json!(""));
}
