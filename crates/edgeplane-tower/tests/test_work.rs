/// Tests for the work module's task-available broadcast registry and
/// the adaptive backoff constants used by the edgeplaned daemon.
use axum_test::TestServer;
use edgeplane_tower::{build_app, AppConfig};
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
    use edgeplane_tower::routes::work::{broadcast_task_available, notify_registry};
    use tokio::sync::broadcast;

    // Subscribe to a test domain channel before broadcasting.
    let mut rx = {
        let mut reg = notify_registry().lock().await;
        let tx = reg
            .entry("test-domain-broadcast".into())
            .or_insert_with(|| broadcast::channel::<String>(8).0);
        tx.subscribe()
    };

    broadcast_task_available("test-domain-broadcast", "k-123", "t-456").await;

    let msg = rx.try_recv().expect("should have received a message");
    let v: serde_json::Value = serde_json::from_str(&msg).expect("valid JSON");
    assert_eq!(v["type"], "task_available");
    assert_eq!(v["mission_id"], "k-123");
    assert_eq!(v["task_id"], "t-456");
}

#[tokio::test]
async fn test_broadcast_no_subscriber_is_silent() {
    use edgeplane_tower::routes::work::broadcast_task_available;
    // A domain with no subscriber — should not panic or error.
    broadcast_task_available("no-subscribers-domain", "k-x", "t-y").await;
}

#[tokio::test]
async fn test_broadcast_multiple_domains_isolated() {
    use edgeplane_tower::routes::work::{broadcast_task_available, notify_registry};
    use tokio::sync::broadcast;

    let mut rx_a = {
        let mut reg = notify_registry().lock().await;
        let tx = reg
            .entry("iso-domain-a".into())
            .or_insert_with(|| broadcast::channel::<String>(8).0);
        tx.subscribe()
    };

    // Broadcast to domain-b only — rx_a should not receive anything.
    broadcast_task_available("iso-domain-b", "k-1", "t-1").await;

    assert!(rx_a.try_recv().is_err(), "domain-a should not receive domain-b's notification");

    // Broadcast to domain-a — rx_a should now receive.
    broadcast_task_available("iso-domain-a", "k-2", "t-2").await;
    let msg = rx_a.try_recv().expect("domain-a should receive its own notification");
    let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
    assert_eq!(v["type"], "task_available");
}

// ── /work/agents/{id}/notify route registration ───────────────────────────────

#[tokio::test]
async fn test_agent_notify_route_requires_auth() {
    let res = server()
        .get("/api/work/agents/test-agent-id/notify")
        .await;
    // Without a valid token the auth middleware rejects before WS upgrade.
    // Acceptable: 401 (explicit auth failure) or 400 (WS handshake rejected).
    // Not acceptable: 404 (route not registered) or 200.
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "/work/agents/{{id}}/notify route should be registered");
    assert_ne!(status, 200, "unauthenticated request must not succeed");
}

// ── /work/missions/{id}/graph route ──────────────────────────────────────────

#[tokio::test]
async fn test_mission_graph_route_requires_auth() {
    let res = server().get("/api/work/missions/k-123/graph").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "/work/missions/{{id}}/graph should be registered");
    assert_ne!(status, 200);
}

// ── Phase 4a: node-keyed assignment-change registry ──────────────────────────
//
// Mirrors the domain-keyed `notify_registry` tests above. edgeplaned daemons
// subscribe per `runtime_node_id`; the controlplane publishes here from
// `enroll_agent` (and Phase 4d's reassign / unassign handlers).

#[tokio::test]
async fn test_assignment_changed_delivers_to_node_subscriber() {
    use edgeplane_tower::routes::work::{broadcast_assignment_changed, node_notify_registry};
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
            "agent": { "id": "a-1", "domain_id": "m-1", "runtime_kind": "claude_agent_acp" },
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
    use edgeplane_tower::routes::work::broadcast_assignment_changed;
    broadcast_assignment_changed(
        "no-subscriber-node",
        serde_json::json!({"type": "agent.unassigned", "agent_id": "a-x"}),
    )
    .await;
}

#[tokio::test]
async fn test_assignment_changed_isolates_per_node() {
    use edgeplane_tower::routes::work::{broadcast_assignment_changed, node_notify_registry};
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
    let res = server().get("/api/runtime/nodes/test-node-id/agents").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "/runtime/nodes/{{id}}/agents should be registered");
    assert_ne!(status, 200, "unauthenticated request must not succeed");
}

#[tokio::test]
async fn test_node_notify_route_registered() {
    let res = server().get("/api/runtime/nodes/test-node-id/notify").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "/runtime/nodes/{{id}}/notify should be registered");
    assert_ne!(status, 200, "unauthenticated request must not succeed");
}

// Admin POST /work/tasks/{id}/dispatched — terminal-from-ready transition
// for the triage routing path (replaces the 4-call temp-agent dance).
#[tokio::test]
async fn test_dispatch_task_route_registered() {
    let res = server().post("/api/work/tasks/test-task-id/dispatched").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "POST /work/tasks/{{id}}/dispatched should be registered");
    assert_ne!(status, 200, "unauthenticated request must not succeed");
    assert_ne!(status, 204, "unauthenticated request must not succeed");
}

// Admin DELETE /work/agents/{id} for ephemeral subagent cleanup.
// See docs/design/ephemeral-task-subagents.md.
#[tokio::test]
async fn test_delete_agent_route_registered() {
    let res = server().delete("/api/work/agents/test-agent-id").await;
    let status = res.status_code().as_u16();
    assert_ne!(status, 404, "DELETE /work/agents/{{id}} should be registered");
    assert_ne!(status, 200, "unauthenticated request must not succeed");
    assert_ne!(status, 204, "unauthenticated request must not succeed");
}

// StartRunRequest accepts both `agent_id`/`task_id` and the column-aligned
// aliases `mesh_agent_id`/`mesh_task_id`. Without the aliases, callers using
// column names silently get NULL FKs on agentrun.
#[test]
fn test_start_run_request_accepts_mesh_aliases() {
    use edgeplane_tower::models::run::StartRunRequest;
    let json = r#"{"runtime_kind":"claude_headless","mesh_agent_id":"a1","mesh_task_id":"t1"}"#;
    let req: StartRunRequest = serde_json::from_str(json).expect("must deserialize");
    assert_eq!(req.agent_id.as_deref(), Some("a1"));
    assert_eq!(req.task_id.as_deref(), Some("t1"));
}

#[test]
fn test_start_run_request_still_accepts_short_names() {
    use edgeplane_tower::models::run::StartRunRequest;
    let json = r#"{"runtime_kind":"claude_headless","agent_id":"a1","task_id":"t1"}"#;
    let req: StartRunRequest = serde_json::from_str(json).expect("must deserialize");
    assert_eq!(req.agent_id.as_deref(), Some("a1"));
    assert_eq!(req.task_id.as_deref(), Some("t1"));
}
