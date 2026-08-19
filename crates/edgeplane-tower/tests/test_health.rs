use axum_test::TestServer;
use edgeplane_tower::{AppConfig, build_app};
use sqlx::PgPool;

fn test_pool() -> PgPool {
    PgPool::connect_lazy("postgres://localhost/test").expect("lazy pool")
}

#[tokio::test]
async fn test_health_returns_ok() {
    let app = build_app(test_pool(), AppConfig::default());
    let server = TestServer::new(app);
    let res = server.get("/api/health").await;
    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn test_health_includes_version() {
    let app = build_app(test_pool(), AppConfig::default());
    let server = TestServer::new(app);
    let res = server.get("/api/health").await;
    let body: serde_json::Value = res.json();
    assert!(body["version"].is_string());
    assert!(!body["version"].as_str().unwrap().is_empty());
}

// The k8s liveness probe hits /healthz at the *root* path (not under /api and
// not behind auth). It must not touch the database.
#[tokio::test]
async fn test_healthz_liveness_at_root() {
    let app = build_app(test_pool(), AppConfig::default());
    let server = TestServer::new(app);
    let res = server.get("/healthz").await;
    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "ok");
}

// The k8s readiness probe hits /readyz at the root path. It is DB-gated, so the
// status depends on Postgres availability — but it must be *registered*: before
// this route existed, /readyz fell through to the proxy fallback and 404'd,
// which is exactly the regression that made a chart deploy never reach Ready.
#[tokio::test]
async fn test_readyz_route_registered_at_root() {
    let app = build_app(test_pool(), AppConfig::default());
    let server = TestServer::new(app);
    let res = server.get("/readyz").await;
    let status = res.status_code().as_u16();
    assert_ne!(
        status, 404,
        "/readyz must be registered at the root path, got {status}"
    );
    assert!(
        status == 200 || status == 503,
        "/readyz should return 200 (db up) or 503 (db down), got {status}"
    );
}
