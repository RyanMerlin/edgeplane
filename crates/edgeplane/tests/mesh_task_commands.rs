//! Integration tests for `edgeplane task mesh` CLI commands — the CLI mirror of the
//! `mcp__edgeplane__*_mesh_task` MCP tools. These operate on the real, agent-claimable
//! `meshtask` table via `/mcp/call`, unrelated to the legacy `task create/list/show/
//! update/delete` CRUD covered by `crud_commands.rs` (which hits domain-scoped REST
//! paths against the disconnected UI-only `task` table).
//!
//! Follows the same two-pronged approach as `crud_commands.rs`:
//!   1. Binary invocation (--help) — no network needed.
//!   2. httpmock round-trip tests via in-process `run(...)` — verify the CLI posts
//!      the right `{"tool": ..., "args": ...}` envelope to `/mcp/call`.

use edgeplane::booster::AgentBooster;
use edgeplane::client::EdgeplaneClient;
use edgeplane::commands::{EdgeplaneCommand, MeshTaskCommand, TaskCommand, run};
use edgeplane::config::EdgeplaneConfig;
use edgeplane::output::OutputMode;
use httpmock::Method::POST;
use httpmock::MockServer;
use serde_json::json;
use std::process::Command;

// ── helpers ──────────────────────────────────────────────────────────────────

fn edgeplane_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_edgeplane"))
}

fn build_config(base_url: &str) -> EdgeplaneConfig {
    EdgeplaneConfig::from_parts(
        base_url,
        Some("test-token".into()),
        None,
        None,
        None,
        2,
        true,
        false, // booster disabled
        false,
        None,
    )
    .unwrap()
}

fn build_client_and_booster(base_url: &str) -> (EdgeplaneClient, AgentBooster) {
    let config = build_config(base_url);
    let client = EdgeplaneClient::new(&config).unwrap();
    let booster = AgentBooster::load(&config).unwrap();
    (client, booster)
}

// ── --help smoke tests (binary, no network) ──────────────────────────────────

#[test]
fn task_mesh_help_is_available() {
    let out = edgeplane_bin()
        .args(["task", "mesh", "--help"])
        .output()
        .expect("spawn edgeplane");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    for sub in [
        "submit",
        "claim",
        "get",
        "list",
        "heartbeat",
        "progress",
        "complete",
        "fail",
        "block",
    ] {
        assert!(
            combined.contains(sub),
            "`edgeplane task mesh --help` should list '{sub}', got:\n{combined}"
        );
    }
}

#[test]
fn task_mesh_submit_help_is_available() {
    let out = edgeplane_bin()
        .args(["task", "mesh", "submit", "--help"])
        .output()
        .expect("spawn edgeplane");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    for flag in [
        "--mission-id",
        "--title",
        "--description",
        "--kind",
        "--priority",
        "--input-json",
    ] {
        assert!(
            combined.contains(flag),
            "`edgeplane task mesh submit --help` should list '{flag}', got:\n{combined}"
        );
    }
}

// ── httpmock round-trip tests ─────────────────────────────────────────────────

#[tokio::test]
async fn mesh_task_submit_posts_to_mcp_call() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/api/mcp/call").json_body(json!({
            "tool": "submit_mesh_task",
            "args": { "mission_id": "X", "title": "Y" }
        }));
        then.status(200)
            .json_body(json!({ "task_id": "mt-1", "status": "pending" }));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Task(TaskCommand::Mesh(MeshTaskCommand::Submit {
            mission_id: "X".into(),
            title: "Y".into(),
            description: None,
            kind: None,
            priority: None,
            input_json: None,
        })),
        client,
        booster,
        config,
        OutputMode::Json,
        None,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn mesh_task_claim_posts_to_mcp_call() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/api/mcp/call").json_body(json!({
            "tool": "claim_mesh_task",
            "args": { "task_id": "mt-1", "agent_id": "agent-a", "lease_seconds": 120 }
        }));
        then.status(200)
            .json_body(json!({ "task_id": "mt-1", "claim_lease_id": "lease-1" }));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Task(TaskCommand::Mesh(MeshTaskCommand::Claim {
            task_id: "mt-1".into(),
            agent_id: Some("agent-a".into()),
            lease_seconds: Some(120),
        })),
        client,
        booster,
        config,
        OutputMode::Json,
        None,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn mesh_task_complete_posts_to_mcp_call() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/api/mcp/call").json_body(json!({
            "tool": "complete_mesh_task",
            "args": {
                "task_id": "mt-1",
                "claim_lease_id": "lease-1",
                "output_json": "{\"result\":\"ok\"}"
            }
        }));
        then.status(200)
            .json_body(json!({ "task_id": "mt-1", "status": "completed" }));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Task(TaskCommand::Mesh(MeshTaskCommand::Complete {
            task_id: "mt-1".into(),
            claim_lease_id: Some("lease-1".into()),
            output_json: Some("{\"result\":\"ok\"}".into()),
        })),
        client,
        booster,
        config,
        OutputMode::Json,
        None,
    )
    .await
    .unwrap();

    mock.assert();
}
