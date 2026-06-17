use axum_test::TestServer;
use edgeplane_tower::{build_app, AppConfig};
use sqlx::PgPool;
use wiremock::{matchers::{method, path}, Mock, MockServer, ResponseTemplate};

fn test_pool() -> PgPool {
    PgPool::connect_lazy("postgres://localhost/test").expect("lazy pool")
}

#[tokio::test]
async fn test_proxy_forwards_unknown_route() {
    let mock_backend = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/some-proxied-path"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&mock_backend)
        .await;

    let config = AppConfig {
        api_proxy: Some(mock_backend.uri()),
        node_id: 1,
        advertise_url: None,
        admin_emails: Default::default(),
    };
    let app = build_app(test_pool(), config);
    let server = TestServer::new(app);

    let res = server.get("/some-proxied-path").await;
    res.assert_status_ok();
}

#[tokio::test]
async fn test_proxy_does_not_override_health() {
    let config = AppConfig {
        api_proxy: Some("http://127.0.0.1:1".to_string()),
        node_id: 1,
        advertise_url: None,
        admin_emails: Default::default(),
    };
    let app = build_app(test_pool(), config);
    let server = TestServer::new(app);

    let res = server.get("/api/health").await;
    res.assert_status_ok();
    let body: serde_json::Value = res.json();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn test_proxy_returns_502_when_upstream_unreachable() {
    let config = AppConfig {
        api_proxy: Some("http://127.0.0.1:1".to_string()),
        node_id: 1,
        advertise_url: None,
        admin_emails: Default::default(),
    };
    let app = build_app(test_pool(), config);
    let server = TestServer::new(app);

    let res = server.get("/some/unknown/path").await;
    res.assert_status(axum::http::StatusCode::BAD_GATEWAY);
}
