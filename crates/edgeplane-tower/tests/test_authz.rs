mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{AppConfig, build_app};

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

/// A server running on a real HTTP port; required for WS upgrade requests.
fn http_server(pool: sqlx::PgPool) -> TestServer {
    TestServer::builder()
        .http_transport()
        .build(build_app(pool, AppConfig::default()))
}

#[tokio::test]
async fn harness_skips_without_db() {
    // Compiles the harness; no-op unless TEST_DATABASE_URL is set.
    let _ = common::setup().await;
}

#[tokio::test]
async fn create_task_denied_for_outsider_sa() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({ "title": "pwn" }))
        .await;
    assert_eq!(res.status_code(), 403);
}

#[tokio::test]
async fn create_task_allowed_for_owner_session() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({ "title": "legit" }))
        .await;
    assert_eq!(res.status_code(), 201);
}

#[tokio::test]
async fn create_task_allowed_for_member_sa_contributor() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({ "title": "ok" }))
        .await;
    assert_eq!(res.status_code(), 201);
}

#[tokio::test]
async fn mcp_submit_mesh_task_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "submit_mesh_task",
            "args": { "mission_id": ctx.mission_id, "title": "pwn" }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(body["error"], "forbidden");
}

#[tokio::test]
async fn domain_stream_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Use a real HTTP port so WebSocketUpgrade extraction can find hyper's
    // OnUpgrade extension. Our authz guard fires before on_upgrade and returns
    // 403; the connection is never actually upgraded.
    let s = http_server(pool);
    let res = s
        .get(&format!("/api/work/domains/{}/stream", ctx.other_domain_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .add_header(axum::http::header::CONNECTION, "upgrade")
        .add_header(axum::http::header::UPGRADE, "websocket")
        .add_header(axum::http::header::SEC_WEBSOCKET_VERSION, "13")
        .add_header(axum::http::header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
        .await;
    assert_eq!(res.status_code(), 403);
}

#[tokio::test]
async fn agent_cannot_complete_unassigned_task() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Seed a task claimed by "agent-A" — member_sa is a domain contributor so
    // the domain guard passes, but it is NOT agent-A, so the owner guard fires.
    let task_id = common::seed_claimed_task(
        &pool,
        &ctx.mission_id,
        &ctx.domain_id,
        "agent-A",
    )
    .await;
    let s = server(pool.clone());

    // A domain-member SA that is not the claimer must get 403.
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.member_sa_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(res.status_code(), 403);

    // A full-trust session owner can complete it.
    let res = s
        .post(&format!("/api/work/tasks/{task_id}/complete"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    // 200 means finished; waiting_review (200) is also acceptable if gates exist.
    assert!(
        res.status_code().is_success(),
        "owner session should complete task, got {}",
        res.status_code()
    );
}

#[tokio::test]
async fn mcp_get_artifact_download_url_denied_for_outsider() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let artifact_id = common::seed_artifact(&pool, &ctx.mission_id).await;
    let s = server(pool);
    let res = s
        .post("/api/mcp/call")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.outsider_sa_token),
        )
        .json(&serde_json::json!({
            "tool": "get_artifact_download_url",
            "args": { "artifact_id": artifact_id }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(body["error"], "forbidden");
}

#[tokio::test]
async fn global_sse_denied_for_non_admin() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let res = s
        .get("/api/sse")
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .await;
    assert_eq!(res.status_code(), 403);
}

// ── T4: per-agent JWT — mint endpoint + enrollment ────────────────────────────

/// Enroll an agent (as owner session) and return the enrolled agent_id +
/// agent_token from the response.
async fn enroll_and_get_token(
    s: &axum_test::TestServer,
    domain_id: &str,
    session_token: &str,
) -> (String, String) {
    let res = s
        .post(&format!("/api/work/domains/{domain_id}/agents/enroll"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {session_token}"),
        )
        .json(&serde_json::json!({"runtime_kind": "test"}))
        .await;
    assert_eq!(
        res.status_code(),
        201,
        "enroll failed: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    let agent_id = body["id"].as_str().unwrap().to_string();
    let agent_token = body["agent_token"].as_str().unwrap().to_string();
    (agent_id, agent_token)
}

#[tokio::test]
async fn agent_cannot_mint_peer_token() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);

    // Enroll two agents via the owner session.
    let (agent_a_id, agent_a_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let (agent_b_id, _agent_b_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // Agent-A must NOT be able to mint a token for agent-B (peer impersonation).
    let res = s
        .post(&format!("/api/work/agents/{agent_b_id}/token"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_a_token}"),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "agent {agent_a_id} should not mint token for {agent_b_id}: {}",
        res.text()
    );
}

#[tokio::test]
async fn enrolled_agent_token_denied_in_foreign_domain() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    // Use a real HTTP port so WebSocketUpgrade extraction can proceed.
    let s = http_server(pool);

    // Enroll an agent into domain A.
    let (_agent_id, agent_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // The agent's token is scoped to domain A — accessing domain B's stream must 403.
    let res = s
        .get(&format!("/api/work/domains/{}/stream", ctx.other_domain_id))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {agent_token}"),
        )
        .add_header(axum::http::header::CONNECTION, "upgrade")
        .add_header(axum::http::header::UPGRADE, "websocket")
        .add_header(axum::http::header::SEC_WEBSOCKET_VERSION, "13")
        .add_header(axum::http::header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "agent token should not access foreign domain: {}",
        res.text()
    );
}

#[tokio::test]
async fn full_trust_session_can_mint_agent_token() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);

    // Enroll an agent via the owner session (to get the agent_id).
    let (agent_id, _) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;

    // A full-trust session should be able to re-mint a token for the agent.
    let res = s
        .post(&format!("/api/work/agents/{agent_id}/token"))
        .add_header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", ctx.owner_session_token),
        )
        .json(&serde_json::json!({}))
        .await;
    assert_eq!(
        res.status_code(),
        200,
        "full-trust session should mint agent token: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert!(
        body["agent_token"].as_str().is_some(),
        "response must contain agent_token"
    );
    assert_eq!(body["expires_in"], 12 * 3600);
}
