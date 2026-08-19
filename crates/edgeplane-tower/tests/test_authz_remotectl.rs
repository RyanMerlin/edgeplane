mod common;

use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::Row;

const MAX_LAUNCH_TTL_HOURS: i64 = 87_600;
const MAX_SCOPE_ENTRIES: usize = 64;
const MAX_SCOPE_TOTAL_LEN: usize = 4096;

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default()))
}

fn bearer(token: &str) -> (axum::http::HeaderName, String) {
    (axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
}

fn launch_body() -> serde_json::Value {
    serde_json::json!({
        "transport": "ssh",
        "agent_kind": "claude-code"
    })
}

async fn enroll_and_get_token(
    s: &axum_test::TestServer,
    domain_id: &str,
    session_token: &str,
) -> (String, String) {
    let (h, v) = bearer(session_token);
    let res = s
        .post(&format!("/api/work/domains/{domain_id}/agents/enroll"))
        .add_header(h, v)
        .json(&serde_json::json!({"runtime_kind": "test"}))
        .await;
    assert_eq!(res.status_code(), 201, "enroll failed: {}", res.text());
    let body: serde_json::Value = res.json();
    let agent_id = body["id"].as_str().unwrap().to_string();
    let agent_token = body["agent_token"].as_str().unwrap().to_string();
    (agent_id, agent_token)
}

async fn register_node_and_get_jwt(s: &axum_test::TestServer, session_token: &str) -> String {
    let (h, v) = bearer(session_token);
    let jt_res = s
        .post("/api/runtime/join-tokens")
        .add_header(h, v)
        .json(&serde_json::json!({"expires_in_seconds": 300}))
        .await;
    assert_eq!(
        jt_res.status_code(),
        201,
        "join token create failed: {}",
        jt_res.text()
    );
    let jt_body: serde_json::Value = jt_res.json();
    let bootstrap_token = jt_body["token"]
        .as_str()
        .expect("join token must contain token");

    let node_name = format!("node-remotectl-{}", uuid::Uuid::new_v4().simple());
    let reg_res = s
        .post("/api/runtime/nodes/register")
        .json(&serde_json::json!({
            "node_name": node_name,
            "hostname": "test-host",
            "bootstrap_token": bootstrap_token,
        }))
        .await;
    assert_eq!(
        reg_res.status_code(),
        201,
        "node register failed: {}",
        reg_res.text()
    );
    let reg_body: serde_json::Value = reg_res.json();
    reg_body["node_jwt"]
        .as_str()
        .expect("register must return node_jwt")
        .to_string()
}

#[tokio::test]
async fn create_launch_denied_for_agent_jwt() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let (_agent_id, agent_token) =
        enroll_and_get_token(&s, &ctx.domain_id, &ctx.owner_session_token).await;
    let (h, v) = bearer(&agent_token);

    let res = s
        .post("/api/remotectl/launches")
        .add_header(h, v)
        .json(&launch_body())
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "agent JWT must not create launch: {}",
        res.text()
    );
}

#[tokio::test]
async fn create_launch_denied_for_node_jwt() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let node_jwt = register_node_and_get_jwt(&s, &ctx.owner_session_token).await;
    let (h, v) = bearer(&node_jwt);

    let res = s
        .post("/api/remotectl/launches")
        .add_header(h, v)
        .json(&launch_body())
        .await;
    assert_eq!(
        res.status_code(),
        403,
        "node JWT must not create launch: {}",
        res.text()
    );
}

#[tokio::test]
async fn create_launch_allowed_for_session() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let (h, v) = bearer(&ctx.owner_session_token);

    let res = s
        .post("/api/remotectl/launches")
        .add_header(h, v)
        .json(&launch_body())
        .await;
    assert_eq!(
        res.status_code(),
        201,
        "session should create launch: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    assert!(
        !body["session_token"].as_str().unwrap_or("").is_empty(),
        "response must contain non-empty session_token"
    );
}

#[tokio::test]
async fn create_launch_clamps_ttl() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool.clone());
    let (h, v) = bearer(&ctx.owner_session_token);
    let mut body = launch_body();
    body["ttl_hours"] = serde_json::json!(9_999_999);

    let res = s
        .post("/api/remotectl/launches")
        .add_header(h, v)
        .json(&body)
        .await;
    assert_eq!(
        res.status_code(),
        201,
        "session should create launch with oversized ttl: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    let session_token = body["session_token"]
        .as_str()
        .expect("response must contain session_token");
    let token_hash = edgeplane_tower::auth::hash_token(session_token);
    let row = sqlx::query("SELECT created_at, expires_at FROM usersession WHERE token_hash = $1")
        .bind(token_hash)
        .fetch_one(&pool)
        .await
        .expect("fetch minted usersession");
    let created_at: chrono::NaiveDateTime = row.get("created_at");
    let expires_at: chrono::NaiveDateTime = row.get("expires_at");
    let ttl_seconds = (expires_at - created_at).num_seconds();
    assert!(
        ttl_seconds <= (MAX_LAUNCH_TTL_HOURS * 3600) + 5,
        "ttl should be clamped, got {ttl_seconds} seconds"
    );
}

#[tokio::test]
async fn create_launch_clamps_scope() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let (h, v) = bearer(&ctx.owner_session_token);
    let mut body = launch_body();
    body["capability_scope"] = serde_json::Value::Array(
        (0..200)
            .map(|i| serde_json::Value::String(format!("capability-{i}")))
            .collect(),
    );

    let res = s
        .post("/api/remotectl/launches")
        .add_header(h, v)
        .json(&body)
        .await;
    assert_eq!(
        res.status_code(),
        201,
        "session should create launch with oversized scope: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    let capability_scope = body["capability_scope"]
        .as_str()
        .expect("response must contain capability_scope");
    let entry_count = if capability_scope.is_empty() {
        0
    } else {
        capability_scope.split(',').count()
    };
    assert!(
        entry_count <= MAX_SCOPE_ENTRIES,
        "scope should contain at most {MAX_SCOPE_ENTRIES} entries, got {entry_count}"
    );
    assert!(
        capability_scope.len() <= MAX_SCOPE_TOTAL_LEN,
        "scope should be at most {MAX_SCOPE_TOTAL_LEN} bytes, got {}",
        capability_scope.len()
    );
}

/// Exercises the byte-length trim path specifically: <= MAX_SCOPE_ENTRIES entries
/// (so `take` alone can't clamp) but each ~200 bytes, so their join exceeds
/// MAX_SCOPE_TOTAL_LEN and the `while` loop must pop whole entries to fit.
#[tokio::test]
async fn create_launch_clamps_scope_byte_length() {
    let Some((pool, ctx)) = setup().await else {
        return;
    };
    let s = server(pool);
    let (h, v) = bearer(&ctx.owner_session_token);
    let mut body = launch_body();
    // 60 entries (< 64, so the entry cap does NOT trigger) of ~200 bytes each →
    // joined length ~12 KB ≫ MAX_SCOPE_TOTAL_LEN, forcing the byte-length trim.
    body["capability_scope"] = serde_json::Value::Array(
        (0..60)
            .map(|i| serde_json::Value::String(format!("cap-{i}-{}", "x".repeat(200))))
            .collect(),
    );

    let res = s
        .post("/api/remotectl/launches")
        .add_header(h, v)
        .json(&body)
        .await;
    assert_eq!(
        res.status_code(),
        201,
        "session should create launch with byte-oversized scope: {}",
        res.text()
    );
    let body: serde_json::Value = res.json();
    let capability_scope = body["capability_scope"]
        .as_str()
        .expect("response must contain capability_scope");
    assert!(
        capability_scope.len() <= MAX_SCOPE_TOTAL_LEN,
        "byte-length trim must bound scope to {MAX_SCOPE_TOTAL_LEN} bytes, got {}",
        capability_scope.len()
    );
    // The trim popped entries, so fewer than the 60 submitted remain — proving the
    // `while` loop ran (not just the `take` cap or a no-op).
    let entry_count = if capability_scope.is_empty() {
        0
    } else {
        capability_scope.split(',').count()
    };
    assert!(
        entry_count < 60,
        "trim loop should have dropped entries, got {entry_count}"
    );
}
