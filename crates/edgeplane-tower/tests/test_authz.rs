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
