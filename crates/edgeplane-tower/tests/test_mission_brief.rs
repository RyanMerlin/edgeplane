mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{build_app, AppConfig};

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

fn bearer(token: &str) -> (axum::http::HeaderName, String) {
    (axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
}

// Regression: the flat mission-brief endpoints (`get_mission_brief_flat` /
// `put_mission_brief_flat`, `GET|PUT /missions/{id}/brief`) filtered on
// `WHERE archived_at IS NULL` — a column that does NOT exist on the `mission`
// table (missions carry a `status`, not a soft-delete timestamp; the non-flat
// `/domains/{d}/m/{m}/brief` variants never filtered on it). The bad predicate
// made every call 500 with a Postgres "column archived_at does not exist" error,
// so the flat brief endpoints were dead for ALL callers. Fixed by dropping the
// predicate. These tests prove the endpoints return 200 (not 500). Authorization
// for the flat GET is a separate concern (Group A of the authz-hardening plan)
// and intentionally not asserted here.

#[tokio::test]
async fn get_mission_brief_flat_does_not_500() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool)
        .get(&format!("/api/missions/{}/brief", ctx.mission_id))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "flat brief GET must succeed after dropping the nonexistent archived_at predicate, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn put_then_get_mission_brief_flat_round_trips() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let body_text = "hello from the flat brief";

    let (h, v) = bearer(&ctx.owner_session_token);
    let put = server(pool.clone())
        .put(&format!("/api/missions/{}/brief", ctx.mission_id))
        .add_header(h, v)
        .json(&serde_json::json!({ "content": body_text }))
        .await;
    assert_eq!(
        put.status_code(),
        200,
        "flat brief PUT must succeed, got {}",
        put.status_code()
    );
    let put_body: serde_json::Value = put.json();
    assert_eq!(put_body["content"], body_text);

    let (h2, v2) = bearer(&ctx.owner_session_token);
    let got = server(pool)
        .get(&format!("/api/missions/{}/brief", ctx.mission_id))
        .add_header(h2, v2)
        .await;
    assert_eq!(got.status_code(), 200);
    let got_body: serde_json::Value = got.json();
    assert_eq!(
        got_body["content"], body_text,
        "GET must return the brief written by PUT"
    );
}
