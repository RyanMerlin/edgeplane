mod common;

use axum_test::TestServer;
use common::{mint_session_with_groups, seed_control_plane_agent, setup};
use edgeplane_tower::{build_app, AppConfig};

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

fn bearer(token: &str) -> (axum::http::HeaderName, String) {
    (axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
}

fn server_with_admin_groups(pool: sqlx::PgPool, groups: &[&str]) -> TestServer {
    let config = AppConfig {
        admin_groups: groups.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    TestServer::new(build_app(pool, config))
}

// Group B (agents.rs) authz: the agent write-mutation handlers previously ignored
// the principal (`_principal`) — any authenticated caller could mutate any agent
// in any domain. They now gate on the agent's own domain via the new
// `authz_by_control_plane_agent` (the `agent` table's current/home domain, NOT
// the wrong `meshagent` table). These tests prove a cross-domain outsider is
// denied 403 on each write, and the domain owner is allowed.
//
// Scope note: read handlers (get_agent/list_agents) and self-register (create_agent)
// remain intentionally ungated for operational lookup and fleet enroll/signal flow.
// Messaging and attach_domain are covered by the Tier-3 tests appended below.

#[tokio::test]
async fn update_agent_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .patch(&format!("/api/agents/{agent}"))
        .add_header(h, v)
        .json(&serde_json::json!({ "status": "offline" }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must not mutate another domain's agent, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn update_agent_allowed_for_owner() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool)
        .patch(&format!("/api/agents/{agent}"))
        .add_header(h, v)
        .json(&serde_json::json!({ "status": "offline" }))
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "domain owner must be allowed to update its own domain's agent, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn delete_agent_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .delete(&format!("/api/agents/{agent}"))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must not delete another domain's agent, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn restart_agent_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .post(&format!("/api/agents/{agent}/restart"))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must not restart another domain's agent, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn clear_context_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .post(&format!("/api/agents/{agent}/clear-context"))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must not clear another domain's agent context, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn list_messages_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .get(&format!("/api/agents/{agent}/messages"))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must not list another domain agent's messages, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn list_messages_allowed_for_owner() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&ctx.owner_session_token);
    let res = server(pool)
        .get(&format!("/api/agents/{agent}/messages"))
        .add_header(h, v)
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "domain owner must be allowed to list own-domain agent messages, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn send_message_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .post(&format!("/api/agents/{agent}/message"))
        .add_header(h, v)
        .json(&serde_json::json!({
            "to_agent_id": agent,
            "content": "x",
            "message_type": "note"
        }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "cross-domain outsider must not send as another domain's agent, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn attach_domain_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&ctx.outsider_sa_token);
    let res = server(pool)
        .patch(&format!("/api/agents/{agent}/domain"))
        .add_header(h, v)
        .json(&serde_json::json!({ "domain_id": ctx.other_domain_id }))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "non-admin outsider must not change another agent's domain, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn attach_domain_allowed_for_admin() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let token =
        mint_session_with_groups(&pool, "sub-admin", "admin@example.com", &["EdgePlane Admins"])
            .await;
    let agent = seed_control_plane_agent(&pool, &ctx.domain_id).await;
    let (h, v) = bearer(&token);
    let res = server_with_admin_groups(pool, &["EdgePlane Admins"])
        .patch(&format!("/api/agents/{agent}/domain"))
        .add_header(h, v)
        .json(&serde_json::json!({ "domain_id": ctx.other_domain_id }))
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "admin must be allowed to change an agent's domain, got {}",
        res.status_code()
    );
}
