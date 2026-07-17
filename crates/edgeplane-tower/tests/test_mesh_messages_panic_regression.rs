//! Regression test: reading back `meshmessage` rows must not panic. `id` and
//! `in_reply_to` are `integer` (i32) in Postgres; `row_to_message` and the
//! send-message handlers previously decoded them as i64 (and `body_json` as
//! non-nullable `&str`), which panics via `Row::get`'s internal
//! `try_get().unwrap()` — crashing the whole request with an empty reply on
//! any real message. This is the core inter-agent messaging path
//! (`send_mesh_message`/`list_mesh_messages`), so the panic was live in
//! production, not just a theoretical mismatch.

mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;

fn server(pool: PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

#[tokio::test]
async fn send_and_list_domain_messages_does_not_panic() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let auth = ("Authorization", format!("Bearer {}", ctx.owner_session_token));

    let first = s
        .post(&format!("/api/work/domains/{}/messages", ctx.domain_id))
        .add_header(axum::http::header::AUTHORIZATION, auth.1.clone())
        .json(&serde_json::json!({
            "channel": "coordination",
            "body": {"text": "hello"},
        }))
        .await;
    first.assert_status(axum::http::StatusCode::CREATED);
    let first_body: serde_json::Value = first.json();
    let first_id = first_body["id"].as_i64().expect("id must be a number");

    // A reply, to exercise the in_reply_to (nullable integer) decode path.
    let reply = s
        .post(&format!("/api/work/domains/{}/messages", ctx.domain_id))
        .add_header(axum::http::header::AUTHORIZATION, auth.1.clone())
        .json(&serde_json::json!({
            "channel": "coordination",
            "body": {"text": "reply"},
            "in_reply_to": first_id,
        }))
        .await;
    reply.assert_status(axum::http::StatusCode::CREATED);

    // This is the read path that previously panicked (empty reply, no JSON
    // body at all) on any row with a populated in_reply_to or body_json.
    let list = s
        .get(&format!("/api/work/domains/{}/messages", ctx.domain_id))
        .add_header(axum::http::header::AUTHORIZATION, auth.1)
        .await;
    list.assert_status_ok();
    let messages: Vec<serde_json::Value> = list.json();
    assert_eq!(messages.len(), 2, "response body: {messages:?}");

    let reply_msg = messages
        .iter()
        .find(|m| m["in_reply_to"] == serde_json::json!(first_id))
        .expect("reply message with matching in_reply_to must be present");
    assert_eq!(reply_msg["body_json"]["text"], "reply");
}
