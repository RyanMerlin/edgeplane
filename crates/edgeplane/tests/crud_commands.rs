//! Integration tests for `edgeplane domain/mission/task` CRUD CLI commands.
//! Uses two complementary approaches:
//!   1. Binary invocation (--help / parse smoke tests) — no network needed.
//!   2. httpmock round-trip tests — verify the right HTTP verb+path is sent.
//!
//! All mutation operations (show/update/delete for missions; create/show/update/delete
//! for tasks) require --domain-id because tower only serves domain-scoped paths.

use httpmock::Method::{DELETE, GET, PATCH, POST};
use httpmock::MockServer;
use edgeplane::client::EdgeplaneClient;
use edgeplane::config::EdgeplaneConfig;
use edgeplane::booster::AgentBooster;
use edgeplane::commands::{run, EdgeplaneCommand, MissionCommand, TaskCommand};
use edgeplane::output::OutputMode;
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
fn mission_help_is_available() {
    let out = edgeplane_bin()
        .args(["mission", "--help"])
        .output()
        .expect("spawn edgeplane");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    for sub in ["create", "list", "show", "update", "delete"] {
        assert!(
            combined.contains(sub),
            "`edgeplane mission --help` should list '{sub}', got:\n{combined}"
        );
    }
}

#[test]
fn task_help_is_available() {
    let out = edgeplane_bin()
        .args(["task", "--help"])
        .output()
        .expect("spawn edgeplane");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    for sub in ["create", "list", "show", "update", "delete"] {
        assert!(
            combined.contains(sub),
            "`edgeplane task --help` should list '{sub}', got:\n{combined}"
        );
    }
}

#[test]
fn domain_help_lists_crud_subcommands() {
    let out = edgeplane_bin()
        .args(["domain", "--help"])
        .output()
        .expect("spawn edgeplane");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    for sub in ["create", "list", "show", "update", "delete"] {
        assert!(
            combined.contains(sub),
            "`edgeplane domain --help` should list '{sub}', got:\n{combined}"
        );
    }
}

// ── mission HTTP round-trip tests ─────────────────────────────────────────────

#[tokio::test]
async fn mission_create_posts_to_domain_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/api/domains/dom-abc/m")
            .json_body(json!({ "name": "test-mission", "domain_id": "dom-abc" }));
        then.status(200)
            .json_body(json!({ "id": "mis-456", "name": "test-mission" }));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Mission(MissionCommand::Create {
            name: "test-mission".into(),
            domain_id: "dom-abc".into(),
            description: None,
            owners: None,
            contributors: None,
            tags: None,
            status: None,
            workstream: None,
        }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn mission_list_without_domain_uses_search() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/api/search/missions");
        then.status(200)
            .json_body(json!([{ "id": "mis-1", "name": "alpha" }]));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Mission(MissionCommand::List { domain_id: None }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn mission_list_with_domain_uses_domain_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/api/domains/dom-xyz/m");
        then.status(200)
            .json_body(json!([{ "id": "mis-2", "name": "beta" }]));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Mission(MissionCommand::List { domain_id: Some("dom-xyz".into()) }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn mission_show_uses_domain_scoped_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/api/domains/dom-1/m/mis-99");
        then.status(200)
            .json_body(json!({ "id": "mis-99", "name": "my-mission" }));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Mission(MissionCommand::Show {
            id: "mis-99".into(),
            domain_id: "dom-1".into(),
        }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn mission_update_patches_domain_scoped_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(PATCH)
            .path("/api/domains/dom-1/m/mis-99")
            .json_body(json!({ "status": "active" }));
        then.status(200)
            .json_body(json!({ "id": "mis-99", "status": "active" }));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Mission(MissionCommand::Update {
            id: "mis-99".into(),
            domain_id: "dom-1".into(),
            name: None,
            description: None,
            owners: None,
            contributors: None,
            tags: None,
            status: Some("active".into()),
        }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn mission_delete_sends_delete_to_domain_scoped_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(DELETE).path("/api/domains/dom-1/m/mis-del-1");
        then.status(204);
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Mission(MissionCommand::Delete {
            id: "mis-del-1".into(),
            domain_id: "dom-1".into(),
        }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

// ── task HTTP round-trip tests ────────────────────────────────────────────────

#[tokio::test]
async fn task_create_posts_to_domain_scoped_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/api/domains/dom-abc/m/mis-abc/t")
            .json_body(json!({ "title": "Do the thing", "mission_id": "mis-abc" }));
        then.status(200)
            .json_body(json!({ "id": 42, "title": "Do the thing" }));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Task(TaskCommand::Create {
            title: "Do the thing".into(),
            mission_id: "mis-abc".into(),
            domain_id: "dom-abc".into(),
            description: None,
            status: None,
            owner: None,
            contributors: None,
            dod: None,
            dependencies: None,
        }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn task_list_uses_mission_shortcut_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/api/missions/mis-xyz/t");
        then.status(200)
            .json_body(json!([{ "id": 1, "title": "a task" }]));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Task(TaskCommand::List { mission_id: "mis-xyz".into() }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn task_show_uses_domain_scoped_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(GET).path("/api/domains/dom-1/m/mis-1/t/task-5");
        then.status(200)
            .json_body(json!({ "id": 5, "title": "a task" }));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Task(TaskCommand::Show {
            id: "task-5".into(),
            mission_id: "mis-1".into(),
            domain_id: "dom-1".into(),
        }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn task_update_patches_task_via_domain_scoped_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(PATCH)
            .path("/api/domains/dom-1/m/mis-1/t/task-99")
            .json_body(json!({ "status": "done" }));
        then.status(200)
            .json_body(json!({ "id": 99, "status": "done" }));
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Task(TaskCommand::Update {
            id: "task-99".into(),
            mission_id: "mis-1".into(),
            domain_id: "dom-1".into(),
            title: None,
            description: None,
            status: Some("done".into()),
            owner: None,
            contributors: None,
            dod: None,
            dependencies: None,
        }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn task_delete_sends_delete_via_domain_scoped_path() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(DELETE).path("/api/domains/dom-1/m/mis-2/t/task-7");
        then.status(204);
    });

    let (client, booster) = build_client_and_booster(&server.url(""));
    let config = build_config(&server.url(""));
    run(
        EdgeplaneCommand::Task(TaskCommand::Delete {
            id: "task-7".into(),
            mission_id: "mis-2".into(),
            domain_id: "dom-1".into(),
        }),
        client,
        booster,
        config,
        OutputMode::Json,
    )
    .await
    .unwrap();

    mock.assert();
}
