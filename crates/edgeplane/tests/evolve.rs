use httpmock::Method::{GET, POST};
use httpmock::MockServer;
use edgeplane::client::EdgeplaneClient;
use edgeplane::config::McConfig;
use edgeplane::evolve::{EvolveArgs, EvolveCommand, RunArgs, SeedArgs, StatusArgs, run};
use serde_json::json;
use std::io::Write;
use tempfile::NamedTempFile;

fn build_client(base_url: &str) -> EdgeplaneClient {
    let config = McConfig::from_parts(
        base_url, None, None, None, None, 2, true, false, false, None,
    )
    .unwrap();
    EdgeplaneClient::new(&config).unwrap()
}

#[tokio::test]
async fn evolve_seed_posts_spec_json() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/evolve/domains")
            .json_body(json!({"spec":{"name":"seed-test","tasks":[]}}));
        then.status(200)
            .json_body(json!({"domain_id":"evolve-123","status":"seeded"}));
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
async fn evolve_run_posts_runtime_kind_to_domain_path() {
    let server = MockServer::start();
    let domain_id = "evolve-abc12345";
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path(format!("/evolve/domains/{domain_id}/run"))
            .json_body(json!({"runtime_kind":"gemini"}));
        then.status(200)
            .json_body(json!({"domain_id":domain_id,"status":"launched"}));
    });

    let client = build_client(&server.url(""));
    run(
        EvolveArgs {
            command: EvolveCommand::Run(RunArgs {
                domain: domain_id.to_string(),
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
async fn evolve_status_gets_domain_status_path() {
    let server = MockServer::start();
    let domain_id = "evolve-xyz00001";
    let mock = server.mock(|when, then| {
        when.method(GET)
            .path(format!("/evolve/domains/{domain_id}/status"));
        then.status(200)
            .json_body(json!({"domain_id":domain_id,"status":"running","run_count":1}));
    });

    let client = build_client(&server.url(""));
    run(
        EvolveArgs {
            command: EvolveCommand::Status(StatusArgs {
                domain: domain_id.to_string(),
            }),
        },
        &client,
    )
    .await
    .unwrap();

    mock.assert();
}
