use axum_test::TestServer;
use edgeplane_tower::{build_app, AppConfig};
use sqlx::PgPool;

fn test_pool() -> PgPool {
    PgPool::connect_lazy("postgres://localhost/test").expect("lazy pool")
}

fn server() -> TestServer {
    TestServer::new(build_app(test_pool(), AppConfig::default()))
}

// ── MCP ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_mcp_health() {
    let res = server().get("/api/mcp/health").await;
    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn test_mcp_tools_returns_list() {
    let res = server().get("/api/mcp/tools").await;
    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert!(body.is_array());
    let tools = body.as_array().unwrap();
    assert!(!tools.is_empty(), "tool list should be non-empty");
    // Spot-check a few required fields
    let first = &tools[0];
    assert!(first.get("name").is_some());
    assert!(first.get("description").is_some());
}

#[tokio::test]
async fn test_mcp_call_requires_auth() {
    // Phase 1.5 of the auth spec: every route that extracts `Principal`
    // returns 401 for unauthenticated callers. /mcp/call is one of them —
    // MCP tool invocations do real work scoped to the caller, so accepting
    // unauthenticated calls would let any HTTP client mutate state with no
    // attribution. The previous version of this test asserted the old
    // (anonymous-permissive) behavior; the new shape is the correct one.
    let res = server()
        .post("/api/mcp/call")
        .json(&serde_json::json!({"tool": "nonexistent_tool", "args": {}}))
        .await;
    let status = res.status_code().as_u16();
    assert_eq!(status, 401, "/mcp/call must require auth, got {status}");
}

// ── Schema-pack ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_schema_pack_returns_json() {
    let res = server().get("/api/schema-pack").await;
    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert!(body.get("loaded").is_some());
}

// ── Ops admin routes return 401 without token ─────────────────────────────────

#[tokio::test]
async fn test_ops_backups_requires_auth() {
    let res = server().get("/api/ops/backups").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 200);
}

// ── Slack integration routes ──────────────────────────────────────────────────

#[tokio::test]
async fn test_slack_events_missing_sig_returns_401() {
    let res = server()
        .post("/api/integrations/slack/events")
        .text("{\"type\":\"url_verification\",\"challenge\":\"abc\"}")
        .await;
    // No SLACK_SIGNING_SECRET set → 401
    let status = res.status_code().as_u16();
    assert_eq!(status, 401);
}

// ── OIDC endpoints exist ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_oidc_start_exists() {
    let res = server().get("/api/auth/oidc/start").await;
    // Will fail due to missing OIDC env vars but should not 404
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "route should exist");
    assert_ne!(status, 405, "route should exist");
}

// ── Family governance ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_family_members_requires_auth() {
    let res = server().get("/api/family/members").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 200);
}

#[tokio::test]
async fn test_family_member_access_requires_auth() {
    let res = server().get("/api/family/members/somesubject/access").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 200);
}

