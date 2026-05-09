use mc_tui::data::{DataClient, RemoteDataClient};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path, query_param}};

async fn make_client(server: &MockServer) -> RemoteDataClient {
    RemoteDataClient::new(server.uri(), None).unwrap()
}

#[tokio::test]
async fn ping_calls_health() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server).await;
    make_client(&server).await.ping().await.unwrap();
}

#[tokio::test]
async fn list_missions_calls_missions() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/missions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server).await;
    make_client(&server).await.list_missions().await.unwrap();
}

#[tokio::test]
async fn list_klusters_uses_mission_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/missions/m1/k"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server).await;
    make_client(&server).await.list_klusters("m1").await.unwrap();
}

#[tokio::test]
async fn list_tasks_uses_canonical_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/missions/m1/k/k1/t"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server).await;
    make_client(&server).await.list_tasks("m1", "k1").await.unwrap();
}

#[tokio::test]
async fn list_approvals_includes_status_pending() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/approvals"))
        .and(query_param("status", "pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server).await;
    make_client(&server).await.list_approvals(None).await.unwrap();
}

#[tokio::test]
async fn respond_approval_posts_to_correct_path() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/approvals/42/respond"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server).await;
    make_client(&server).await.respond_approval("42", "approve", None).await.unwrap();
}

#[tokio::test]
async fn list_agents_accepts_integer_id() {
    let server = MockServer::start().await;
    // Wire shape returned by mc-server: id is an integer, not a string.
    Mock::given(method("GET")).and(path("/agents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "id": 7,
                "name": "agent-alpha",
                "status": "idle",
                "capabilities": "bash,python",
                "updated_at": "2026-05-09T12:00:00Z"
            }
        ])))
        .mount(&server).await;
    let agents = make_client(&server).await.list_agents().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].id, "7");
    assert_eq!(agents[0].name, "agent-alpha");
}

#[tokio::test]
async fn list_agents_accepts_string_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/agents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": "abc-123", "name": "agent-beta", "status": "online" }
        ])))
        .mount(&server).await;
    let agents = make_client(&server).await.list_agents().await.unwrap();
    assert_eq!(agents[0].id, "abc-123");
}
