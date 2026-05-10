/// Tests for the work module's task-available broadcast registry and
/// the adaptive backoff constants used by the mc-mesh daemon.
use axum_test::TestServer;
use mc_controlplane::{build_app, AppConfig};
use sqlx::PgPool;

fn test_pool() -> PgPool {
    PgPool::connect_lazy("postgres://localhost/test").expect("lazy pool")
}

fn server() -> TestServer {
    TestServer::new(build_app(test_pool(), AppConfig::default()))
}

// ── broadcast_task_available ──────────────────────────────────────────────────

#[tokio::test]
async fn test_broadcast_delivers_to_subscriber() {
    use mc_controlplane::routes::work::{broadcast_task_available, notify_registry};
    use tokio::sync::broadcast;

    // Subscribe to a test mission channel before broadcasting.
    let mut rx = {
        let mut reg = notify_registry().lock().await;
        let tx = reg
            .entry("test-mission-broadcast".into())
            .or_insert_with(|| broadcast::channel::<String>(8).0);
        tx.subscribe()
    };

    broadcast_task_available("test-mission-broadcast", "k-123", "t-456").await;

    let msg = rx.try_recv().expect("should have received a message");
    let v: serde_json::Value = serde_json::from_str(&msg).expect("valid JSON");
    assert_eq!(v["type"], "task_available");
    assert_eq!(v["kluster_id"], "k-123");
    assert_eq!(v["task_id"], "t-456");
}

#[tokio::test]
async fn test_broadcast_no_subscriber_is_silent() {
    use mc_controlplane::routes::work::broadcast_task_available;
    // A mission with no subscriber — should not panic or error.
    broadcast_task_available("no-subscribers-mission", "k-x", "t-y").await;
}

#[tokio::test]
async fn test_broadcast_multiple_missions_isolated() {
    use mc_controlplane::routes::work::{broadcast_task_available, notify_registry};
    use tokio::sync::broadcast;

    let mut rx_a = {
        let mut reg = notify_registry().lock().await;
        let tx = reg
            .entry("iso-mission-a".into())
            .or_insert_with(|| broadcast::channel::<String>(8).0);
        tx.subscribe()
    };

    // Broadcast to mission-b only — rx_a should not receive anything.
    broadcast_task_available("iso-mission-b", "k-1", "t-1").await;

    assert!(rx_a.try_recv().is_err(), "mission-a should not receive mission-b's notification");

    // Broadcast to mission-a — rx_a should now receive.
    broadcast_task_available("iso-mission-a", "k-2", "t-2").await;
    let msg = rx_a.try_recv().expect("mission-a should receive its own notification");
    let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
    assert_eq!(v["type"], "task_available");
}

// ── /work/agents/{id}/notify route registration ───────────────────────────────

#[tokio::test]
async fn test_agent_notify_route_requires_auth() {
    let res = server()
        .get("/work/agents/test-agent-id/notify")
        .await;
    // Without a valid token the auth middleware rejects before WS upgrade.
    // Acceptable: 401 (explicit auth failure) or 400 (WS handshake rejected).
    // Not acceptable: 404 (route not registered) or 200.
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "/work/agents/{{id}}/notify route should be registered");
    assert_ne!(status, 200, "unauthenticated request must not succeed");
}

// ── /work/klusters/{id}/graph route ──────────────────────────────────────────

#[tokio::test]
async fn test_kluster_graph_route_requires_auth() {
    let res = server().get("/work/klusters/k-123/graph").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "/work/klusters/{{id}}/graph should be registered");
    assert_ne!(status, 200);
}

// ── Phase 4a: node-keyed assignment-change registry ──────────────────────────
//
// Mirrors the mission-keyed `notify_registry` tests above. mc-mesh daemons
// subscribe per `runtime_node_id`; the controlplane publishes here from
// `enroll_agent` (and Phase 4d's reassign / unassign handlers).

#[tokio::test]
async fn test_assignment_changed_delivers_to_node_subscriber() {
    use mc_controlplane::routes::work::{broadcast_assignment_changed, node_notify_registry};
    use tokio::sync::broadcast;

    let mut rx = {
        let mut reg = node_notify_registry().lock().await;
        let tx = reg
            .entry("test-node-assigned".into())
            .or_insert_with(|| broadcast::channel::<String>(8).0);
        tx.subscribe()
    };

    broadcast_assignment_changed(
        "test-node-assigned",
        serde_json::json!({
            "type": "agent.assigned",
            "agent_id": "a-1",
            "agent": { "id": "a-1", "mission_id": "m-1", "runtime_kind": "claude_agent_acp" },
        }),
    )
    .await;

    let msg = rx.try_recv().expect("subscriber should receive notification");
    let v: serde_json::Value = serde_json::from_str(&msg).expect("valid JSON");
    assert_eq!(v["type"], "agent.assigned");
    assert_eq!(v["agent_id"], "a-1");
    assert_eq!(v["agent"]["runtime_kind"], "claude_agent_acp");
}

#[tokio::test]
async fn test_assignment_changed_no_subscriber_is_silent() {
    use mc_controlplane::routes::work::broadcast_assignment_changed;
    broadcast_assignment_changed(
        "no-subscriber-node",
        serde_json::json!({"type": "agent.unassigned", "agent_id": "a-x"}),
    )
    .await;
}

#[tokio::test]
async fn test_assignment_changed_isolates_per_node() {
    use mc_controlplane::routes::work::{broadcast_assignment_changed, node_notify_registry};
    use tokio::sync::broadcast;

    let mut rx_a = {
        let mut reg = node_notify_registry().lock().await;
        let tx = reg
            .entry("iso-node-a".into())
            .or_insert_with(|| broadcast::channel::<String>(8).0);
        tx.subscribe()
    };

    // Publish to node-b only — node-a must not see it.
    broadcast_assignment_changed(
        "iso-node-b",
        serde_json::json!({"type": "agent.assigned", "agent_id": "a-b1"}),
    )
    .await;
    assert!(rx_a.try_recv().is_err(), "node-a must not receive node-b's event");

    // Publish to node-a — must arrive.
    broadcast_assignment_changed(
        "iso-node-a",
        serde_json::json!({"type": "agent.reassigned", "agent_id": "a-a1"}),
    )
    .await;
    let msg = rx_a.try_recv().expect("node-a should receive its own event");
    let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
    assert_eq!(v["type"], "agent.reassigned");
    assert_eq!(v["agent_id"], "a-a1");
}

// ── Route registration smoke tests ───────────────────────────────────────────
//
// We can't easily create a registered runtimenode + valid principal here
// without a real DB, so we just verify the routes are wired. Behavior is
// covered by integration tests against a live stack.

#[tokio::test]
async fn test_list_node_agents_route_registered() {
    let res = server().get("/runtime/nodes/test-node-id/agents").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "/runtime/nodes/{{id}}/agents should be registered");
    assert_ne!(status, 200, "unauthenticated request must not succeed");
}

#[tokio::test]
async fn test_node_notify_route_registered() {
    let res = server().get("/runtime/nodes/test-node-id/notify").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "/runtime/nodes/{{id}}/notify should be registered");
    assert_ne!(status, 200, "unauthenticated request must not succeed");
}
