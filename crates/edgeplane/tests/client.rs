use httpmock::Method::GET;
use httpmock::MockServer;
use edgeplane::client::EdgeplaneClient;
use edgeplane::config::EdgeplaneConfig;
use serde_json::json;

fn build_config(base_url: &str) -> EdgeplaneConfig {
    EdgeplaneConfig::from_parts(
        base_url, None, None, None, None, 2, true, false, false, None,
    )
    .unwrap()
}

fn build_config_with_context(base_url: &str) -> EdgeplaneConfig {
    EdgeplaneConfig::from_parts(
        base_url,
        None,
        Some("agent-alpha".into()),
        Some("rs_test123".into()),
        Some("research".into()),
        2,
        true,
        false,
        false,
        None,
    )
    .unwrap()
}

fn build_config_with_context_values(
    base_url: &str,
    agent_id: &str,
    runtime_session_id: &str,
    profile_name: &str,
) -> EdgeplaneConfig {
    EdgeplaneConfig::from_parts(
        base_url,
        None,
        Some(agent_id.to_string()),
        Some(runtime_session_id.to_string()),
        Some(profile_name.to_string()),
        2,
        true,
        false,
        false,
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn get_json_returns_expected_payload() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/api/mcp/tools");
        then.status(200)
            .json_body(json!({ "ok": true, "payload": "hello" }));
    });

    let config = build_config(&server.url(""));
    let client = EdgeplaneClient::new(&config).unwrap();
    let payload = client.get_json("/mcp/tools").await.unwrap();
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["payload"], "hello");
    mock.assert();
}

#[tokio::test]
async fn post_json_sends_body() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/api/mcp/call")
            .json_body(json!({ "tool": "foo" }));
        then.status(200)
            .json_body(json!({ "tool": "foo", "status": "ok" }));
    });

    let config = build_config(&server.url(""));
    let client = EdgeplaneClient::new(&config).unwrap();
    let payload = client
        .post_json("/mcp/call", &json!({ "tool": "foo" }))
        .await
        .unwrap();
    assert_eq!(payload["tool"], "foo");
    assert_eq!(payload["status"], "ok");
    mock.assert();
}

#[tokio::test]
async fn request_includes_agent_session_and_profile_headers() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path("/api/mcp/tools")
            .header("x-edgeplane-agent-id", "agent-alpha")
            .header("x-edgeplane-runtime-session-id", "rs_test123")
            .header("x-edgeplane-instance-id", "rs_test123")
            .header("x-edgeplane-agent-profile", "research");
        then.status(200).json_body(json!({ "ok": true }));
    });

    let config = build_config_with_context(&server.url(""));
    let client = EdgeplaneClient::new(&config).unwrap();
    let payload = client.get_json("/mcp/tools").await.unwrap();
    assert_eq!(payload["ok"], true);
    mock.assert();
}

#[tokio::test]
async fn concurrent_clients_keep_distinct_identity_headers() {
    let server = MockServer::start();
    let mock_a = server.mock(|when, then| {
        when.method(GET)
            .path("/api/mcp/tools")
            .header("x-edgeplane-agent-id", "agent-a")
            .header("x-edgeplane-runtime-session-id", "rs_a")
            .header("x-edgeplane-agent-profile", "research");
        then.status(200)
            .json_body(json!({ "ok": true, "agent": "a" }));
    });
    let mock_b = server.mock(|when, then| {
        when.method(GET)
            .path("/api/mcp/tools")
            .header("x-edgeplane-agent-id", "agent-b")
            .header("x-edgeplane-runtime-session-id", "rs_b")
            .header("x-edgeplane-agent-profile", "security");
        then.status(200)
            .json_body(json!({ "ok": true, "agent": "b" }));
    });

    let client_a = EdgeplaneClient::new(&build_config_with_context_values(
        &server.url(""),
        "agent-a",
        "rs_a",
        "research",
    ))
    .unwrap();
    let client_b = EdgeplaneClient::new(&build_config_with_context_values(
        &server.url(""),
        "agent-b",
        "rs_b",
        "security",
    ))
    .unwrap();

    let (resp_a, resp_b) = tokio::join!(
        client_a.get_json("/mcp/tools"),
        client_b.get_json("/mcp/tools")
    );
    assert_eq!(resp_a.unwrap()["ok"], true);
    assert_eq!(resp_b.unwrap()["ok"], true);
    mock_a.assert_hits(1);
    mock_b.assert_hits(1);
}
