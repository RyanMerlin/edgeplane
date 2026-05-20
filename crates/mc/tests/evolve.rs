use httpmock::Method::{GET, POST};
use httpmock::MockServer;
use mc::client::MissionControlClient;
use mc::config::McConfig;
use mc::evolve::{EvolveArgs, EvolveCommand, RunArgs, SeedArgs, StatusArgs, run};
use serde_json::json;
use std::io::Write;
use tempfile::NamedTempFile;

fn build_client(base_url: &str) -> MissionControlClient {
    let config = McConfig::from_parts(
        base_url, None, None, None, None, 2, true, false, false, None,
    )
    .unwrap();
    MissionControlClient::new(&config).unwrap()
}

#[tokio::test]
async fn evolve_seed_posts_spec_json() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/evolve/missions")
            .json_body(json!({"spec":{"name":"seed-test","tasks":[]}}));
        then.status(200)
            .json_body(json!({"mission_id":"evolve-123","status":"seeded"}));
    });

    let mut spec_file = NamedTempFile::new().unwrap();
    writeln!(spec_file, "{{\"name\":\"seed-test\",\"tasks\":[]}}").unwrap();
    let client = build_client(&server.url(""));
    run(
        EvolveArgs {
            command: EvolveCommand::Seed(SeedArgs {
                spec: spec_file.path().display().to_string(),
            }),
        },
        &client,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn evolve_run_posts_runtime_kind_to_mission_path() {
    let server = MockServer::start();
    let mission_id = "evolve-abc12345";
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path(format!("/evolve/missions/{mission_id}/run"))
            .json_body(json!({"runtime_kind":"gemini"}));
        then.status(200)
            .json_body(json!({"mission_id":mission_id,"status":"launched"}));
    });

    let client = build_client(&server.url(""));
    run(
        EvolveArgs {
            command: EvolveCommand::Run(RunArgs {
                mission: mission_id.to_string(),
                agent: "gemini".to_string(),
            }),
        },
        &client,
    )
    .await
    .unwrap();

    mock.assert();
}

#[tokio::test]
async fn evolve_status_gets_mission_status_path() {
    let server = MockServer::start();
    let mission_id = "evolve-xyz00001";
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path(format!("/evolve/missions/{mission_id}/status"));
        then.status(200)
            .json_body(json!({"mission_id":mission_id,"status":"running","run_count":1}));
    });

    let client = build_client(&server.url(""));
    run(
        EvolveArgs {
            command: EvolveCommand::Status(StatusArgs {
                mission: mission_id.to_string(),
            }),
        },
        &client,
    )
    .await
    .unwrap();

    mock.assert();
}
