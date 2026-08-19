//! Group-based admin (#77 follow-up): exercises the full request-time path —
//! the `groups` claim persisted on the session (migration 0011) is read back
//! and resolved against `EP_ADMIN_GROUPS` into `Principal.is_admin`, surfaced
//! via `/api/auth/me`. Env-gated on TEST_DATABASE_URL (skips without a DB).
mod common;

use axum_test::TestServer;
use common::{mint_grant, mint_session_with_groups, setup};
use edgeplane_tower::{AppConfig, build_app};

fn server_with_admin_groups(pool: sqlx::PgPool, groups: &[&str]) -> TestServer {
    let config = AppConfig {
        admin_groups: groups.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    TestServer::new(build_app(pool, config))
}

async fn whoami_is_admin(server: &TestServer, token: &str) -> bool {
    let res = server
        .get("/api/auth/me")
        .add_header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
        .await;
    assert_eq!(res.status_code(), 200, "auth/me should succeed");
    res.json::<serde_json::Value>()["is_admin"]
        .as_bool()
        .expect("is_admin present")
}

#[tokio::test]
async fn group_member_resolves_admin() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    let token = mint_session_with_groups(
        &pool,
        "sub-admin",
        "admin@example.com",
        &["EdgePlane Admins"],
    )
    .await;
    let s = server_with_admin_groups(pool, &["EdgePlane Admins"]);
    assert!(
        whoami_is_admin(&s, &token).await,
        "member of an EP_ADMIN_GROUPS group must be admin"
    );
}

#[tokio::test]
async fn non_member_is_not_admin() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    let token = mint_session_with_groups(&pool, "sub-user", "user@example.com", &["Users"]).await;
    let s = server_with_admin_groups(pool, &["EdgePlane Admins"]);
    assert!(
        !whoami_is_admin(&s, &token).await,
        "a user not in any admin group must not be admin"
    );
}

#[tokio::test]
async fn unconfigured_admin_groups_grants_no_one() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    // Carries the group name, but EP_ADMIN_GROUPS is empty → fail-closed.
    let token =
        mint_session_with_groups(&pool, "sub-x", "x@example.com", &["EdgePlane Admins"]).await;
    let s = server_with_admin_groups(pool, &[]);
    assert!(
        !whoami_is_admin(&s, &token).await,
        "empty EP_ADMIN_GROUPS must grant no admin"
    );
}

/// Full CLI path: a grant carrying display_name + groups, exchanged via
/// /auth/oidc/exchange, must (a) return the display name and (b) issue a session
/// that whoami reports as the named, group-admin identity — even with no email.
#[tokio::test]
async fn exchange_threads_display_name_and_group_admin() {
    let Some((pool, _ctx)) = setup().await else {
        return;
    };
    // email "" mirrors the verified-email gate dropping it; admin must come from groups.
    let grant_id = mint_grant(&pool, "sub-ex", "", "merlin", &["EdgePlane Admins"]).await;
    let s = server_with_admin_groups(pool, &["EdgePlane Admins"]);

    let res = s
        .post("/api/auth/oidc/exchange")
        .json(&serde_json::json!({ "grant_id": grant_id }))
        .await;
    assert_eq!(res.status_code(), 200, "exchange should succeed");
    let body: serde_json::Value = res.json();
    assert_eq!(
        body["name"], "merlin",
        "exchange response carries display_name"
    );
    let token = body["access_token"]
        .as_str()
        .expect("access_token")
        .to_string();

    let me = s
        .get("/api/auth/me")
        .add_header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
        .await;
    assert_eq!(me.status_code(), 200);
    let me_body: serde_json::Value = me.json();
    assert_eq!(
        me_body["name"], "merlin",
        "whoami reflects the persisted display name"
    );
    assert_eq!(
        me_body["is_admin"], true,
        "group membership grants admin via exchange"
    );
}
