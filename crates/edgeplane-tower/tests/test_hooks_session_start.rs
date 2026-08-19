//! Regression test: `find_or_create_agent` (the Claude/Codex `session-start`
//! hook's agent upsert) must actually insert an `agent` row. It previously
//! omitted `public_id` (NOT NULL, no DB default), so every session-start
//! hook call for a not-yet-seen subject 500'd with a `database_error`.

mod common;

use axum_test::TestServer;
use common::mint_session;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;
use uuid::Uuid;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

#[tokio::test]
async fn session_start_creates_agent_with_public_id() {
    let Some(url) = std::env::var("TEST_DATABASE_URL").ok() else {
        return;
    };
    let pool = PgPool::connect(&url)
        .await
        .expect("connect TEST_DATABASE_URL");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate");

    let subject = format!("hook-subject-{}", Uuid::new_v4().simple());
    let token = mint_session(&pool, &subject, &format!("{subject}@example.com")).await;

    let s = server(pool.clone());
    let res = s
        .post("/api/hooks/claude/session-start")
        .add_header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
        .json(&serde_json::json!({
            "session_id": format!("sess-{}", Uuid::new_v4().simple()),
            "source": "startup",
        }))
        .await;

    res.assert_status_ok();

    let (public_id, capabilities): (String, String) =
        sqlx::query_as("SELECT public_id, capabilities FROM agent WHERE name = $1")
            .bind(&subject)
            .fetch_one(&pool)
            .await
            .expect("session-start must persist an agent row");
    assert!(
        public_id.starts_with(&format!("{subject}-")),
        "public_id should follow the {{name}}-{{8hex}} convention: {public_id}"
    );
    assert_eq!(capabilities, "claude-code");
}
