use httpmock::Method::POST;
use httpmock::MockServer;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn mc_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mc")
}

/// Seed a session.json with a test token in `mc_home`, mimicking the file
/// written by `mc auth login`. Returns the MC_HOME path.
fn seed_session(mc_home: &Path, base_url: &str) {
    fs::create_dir_all(mc_home).expect("mc_home");
    let session = json!({
        "token": "mcs_test-token",
        "subject": "test-user",
        "expires_at": "2099-01-01T00:00:00Z",
        "base_url": base_url,
        "session_id": null
    });
    fs::write(
        mc_home.join("session.json"),
        serde_json::to_string(&session).unwrap(),
    )
    .expect("session.json");
}

#[test]
fn profile_list_uses_mcp_call() {
    let server = MockServer::start();
    let tmp = tempdir().expect("tmp");
    let mc_home = tmp.path().join("mc-home");
    seed_session(&mc_home, &server.url(""));

    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/mcp/call")
            .json_body(json!({"tool":"list_profiles","args":{"limit":2}}));
        then.status(200).json_body(json!({
            "ok": true,
            "result": {
                "profiles": [{"name":"research","sha256":"abc"}]
            }
        }));
    });

    let output = Command::new(mc_bin())
        .args(["profile", "list", "--limit", "2"])
        .env("MC_BASE_URL", server.url(""))
        .env("MC_HOME", &mc_home)
        .output()
        .expect("run mc profile list");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("research"), "stdout={stdout}");
    mock.assert();
}

#[test]
fn profile_publish_uses_mcp_call() {
    let server = MockServer::start();
    let tmp = tempdir().expect("tmp");
    let mc_home = tmp.path().join("mc-home");
    seed_session(&mc_home, &server.url(""));
    let bundle = tmp.path().join("bundle.tar");
    fs::write(&bundle, b"demo-profile-bundle").expect("write bundle");

    let mock = server.mock(|when, then| {
        when.method(POST).path("/mcp/call");
        then.status(200).json_body(json!({
            "ok": true,
            "result": {
                "profile": {"name":"dev","sha256":"new-sha","is_default":false}
            }
        }));
    });

    let output = Command::new(mc_bin())
        .args([
            "--json",
            "profile",
            "publish",
            "--name",
            "dev",
            "--bundle",
            bundle.to_str().expect("bundle path"),
        ])
        .env("MC_BASE_URL", server.url(""))
        .env("MC_HOME", &mc_home)
        .output()
        .expect("run mc profile publish");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"name\": \"dev\""), "stdout={stdout}");
    mock.assert_hits(1);
}

#[test]
fn profile_pull_respects_pin_mismatch_from_mcp() {
    let server = MockServer::start();
    let tmp = tempdir().expect("tmp");
    let mc_home = tmp.path().join("mc-home");
    seed_session(&mc_home, &server.url(""));
    let profile_dir = mc_home.join("profiles").join("research");
    fs::create_dir_all(&profile_dir).expect("profile dir");
    fs::write(
        profile_dir.join("pin.json"),
        r#"{"profile":"research","pinned_sha256":"pinned-sha"}"#,
    )
    .expect("pin");

    let mock = server.mock(|when, then| {
        when.method(POST).path("/mcp/call").json_body(json!({
            "tool":"download_profile",
            "args":{"name":"research","if_sha256":"pinned-sha"}
        }));
        then.status(200).json_body(json!({
            "ok": false,
            "error": "profile_sha_mismatch",
            "result": {"expected_sha256":"pinned-sha","current_sha256":"remote-sha"}
        }));
    });

    let output = Command::new(mc_bin())
        .args(["profile", "pull", "--name", "research"])
        .env("MC_BASE_URL", server.url(""))
        .env("MC_HOME", &mc_home)
        .output()
        .expect("run mc profile pull");

    assert!(
        !output.status.success(),
        "stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("profile_sha_mismatch"), "stderr={stderr}");
    mock.assert();
}

#[test]
fn profile_status_calls_get_and_pin_tools() {
    let server = MockServer::start();
    let tmp = tempdir().expect("tmp");
    let mc_home = tmp.path().join("mc-home");
    seed_session(&mc_home, &server.url(""));
    let profile_dir = mc_home.join("profiles").join("research");
    fs::create_dir_all(&profile_dir).expect("profile dir");
    fs::write(
        profile_dir.join("pin.json"),
        r#"{"profile":"research","pinned_sha256":"remote-sha"}"#,
    )
    .expect("pin");

    let get_mock = server.mock(|when, then| {
        when.method(POST).path("/mcp/call").json_body(json!({
            "tool":"get_profile",
            "args":{"name":"research"}
        }));
        then.status(200).json_body(json!({
            "ok": true,
            "result": {"profile":{"name":"research","sha256":"remote-sha"}}
        }));
    });

    let pin_mock = server.mock(|when, then| {
        when.method(POST).path("/mcp/call").json_body(json!({
            "tool":"pin_profile_version",
            "args":{"name":"research","sha256":"remote-sha"}
        }));
        then.status(200).json_body(json!({
            "ok": true,
            "result": {
                "name":"research",
                "pinned_sha256":"remote-sha",
                "remote_sha256":"remote-sha",
                "matches": true
            }
        }));
    });

    let output = Command::new(mc_bin())
        .args(["--json", "profile", "status", "--name", "research"])
        .env("MC_BASE_URL", server.url(""))
        .env("MC_HOME", &mc_home)
        .output()
        .expect("run mc profile status");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"pin_check\""), "stdout={stdout}");
    assert!(stdout.contains("\"matches\": true"), "stdout={stdout}");
    get_mock.assert();
    pin_mock.assert();
}

#[test]
fn init_bootstraps_default_profile_when_empty() {
    let server = MockServer::start();
    let tmp = tempdir().expect("tmp");
    let mc_home = tmp.path().join("mc-home");
    seed_session(&mc_home, &server.url(""));

    let list_mock = server.mock(|when, then| {
        when.method(POST)
            .path("/mcp/call")
            .json_body(json!({"tool":"list_profiles","args":{"limit":1}}));
        then.status(200).json_body(json!({
            "ok": true,
            "result": {"profiles": []}
        }));
    });
    let publish_mock = server.mock(|when, then| {
        when.method(POST).path("/mcp/call");
        then.status(200).json_body(json!({
            "ok": true,
            "result": {
                "profile": {"name":"default","sha256":"seed-sha","is_default":true}
            }
        }));
    });

    let output = Command::new(mc_bin())
        .args(["--json", "init"])
        .env("MC_BASE_URL", server.url(""))
        .env("MC_HOME", &mc_home)
        .output()
        .expect("run mc init");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"created\": true"), "stdout={stdout}");
    assert!(stdout.contains("\"name\": \"default\""), "stdout={stdout}");
    list_mock.assert_hits(1);
    let _ = publish_mock;
}
