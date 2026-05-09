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
