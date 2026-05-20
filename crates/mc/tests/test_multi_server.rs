use mc::client::MultiServerClient;
use std::time::Duration;
use wiremock::{matchers::{method, path}, Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_client_uses_first_live_server() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"ok"})))
        .mount(&mock)
        .await;

    let client = MultiServerClient::new(
        vec![mock.uri()],
        Duration::from_secs(5),
    )
    .unwrap();

    let res = client.get_json("/health").await.unwrap();
    assert_eq!(res["status"], "ok");
}

#[tokio::test]
async fn test_client_fails_over_to_second_server() {
    let live = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&live)
        .await;

    let client = MultiServerClient::new(
        vec![
            "http://127.0.0.1:1".to_string(), // dead
            live.uri(),
        ],
        Duration::from_secs(5),
    )
    .unwrap();

    let res = client.get_json("/missions").await.unwrap();
    assert_eq!(res, serde_json::json!([]));
}

#[tokio::test]
async fn test_client_returns_error_when_all_servers_dead() {
    let client = MultiServerClient::new(
        vec![
            "http://127.0.0.1:1".to_string(),
            "http://127.0.0.1:2".to_string(),
        ],
        Duration::from_secs(2),
    )
    .unwrap();

    let res = client.get_json("/health").await;
    assert!(res.is_err(), "should fail when all servers are unreachable");
}

#[tokio::test]
async fn test_client_returns_4xx_immediately_without_trying_next() {
    let server_a = MockServer::start().await;
    let server_b = MockServer::start().await;

    // server_a returns 404
    Mock::given(method("GET"))
        .and(path("/not-found"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server_a)
        .await;

    // server_b should never be reached
    Mock::given(method("GET"))
        .and(path("/not-found"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0) // must not be called
        .mount(&server_b)
        .await;

    let client = MultiServerClient::new(
        vec![server_a.uri(), server_b.uri()],
        Duration::from_secs(5),
    )
    .unwrap();

    let res = client.get_json("/not-found").await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("404"));
}

#[tokio::test]
async fn test_client_skips_5xx_and_tries_next() {
    let server_a = MockServer::start().await;
    let server_b = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/data"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server_a)
        .await;

    Mock::given(method("GET"))
        .and(path("/data"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok":true})))
        .mount(&server_b)
        .await;

    let client = MultiServerClient::new(
        vec![server_a.uri(), server_b.uri()],
        Duration::from_secs(5),
    )
    .unwrap();

    let res = client.get_json("/data").await.unwrap();
    assert_eq!(res["ok"], true);
}
