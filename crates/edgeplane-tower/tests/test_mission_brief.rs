mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{AppConfig, build_app};

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
// predicate.
//
// Dropping the predicate also *unmasks* both endpoints: while they 500'd they
// were un-exploitable, but a live PUT with no domain gate is a write IDOR (any
// caller overwrites any mission's brief by id) and a live GET a read IDOR. So
// this same change gates both on the mission's domain via `authz_domain`. The
// tests below prove (1) an authorized owner gets 200 (not 500) and round-trips,
// and (2) a cross-domain outsider is denied 403 on both verbs.

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

// IDOR closure: an authenticated principal that is NOT a member of the mission's
// domain must be denied on both flat verbs. Without the gate, dropping the
// archived_at predicate would let any caller read/overwrite any mission's brief.

#[tokio::test]
async fn get_mission_brief_flat_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .get(&format!("/api/missions/{}/brief", ctx.mission_id))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must be denied the flat brief GET, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn put_mission_brief_flat_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .put(&format!("/api/missions/{}/brief", ctx.mission_id))
        .add_header(h, v)
        .json(&serde_json::json!({ "content": "outsider should not be able to write this" }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must be denied the flat brief PUT (write IDOR), got {}",
        res.status_code()
    );
}
