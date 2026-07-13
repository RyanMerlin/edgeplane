mod common;

use axum_test::TestServer;
use common::{seed_control_plane_agent, setup};
use edgeplane_tower::{build_app, AppConfig};

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

fn bearer(token: &str) -> (axum::http::HeaderName, String) {
    (axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
}

// Group B (agents.rs) authz: the agent write-mutation handlers previously ignored
// the principal (`_principal`) — any authenticated caller could mutate any agent
// in any domain. They now gate on the agent's own domain via the new
// `authz_by_control_plane_agent` (the `agent` table's current/home domain, NOT
// the wrong `meshagent` table). These tests prove a cross-domain outsider is
// denied 403 on each write, and the domain owner is allowed.
//
// Scope note: read handlers (get_agent/list_agents), self-register (create_agent),
// attach_domain, and messaging are intentionally NOT gated here — they need
// design decisions (cross-domain operational reads, self-vs-other ownership) and
// are tracked separately in docs/plans/2026-07-10-authz-hardening.md.

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
