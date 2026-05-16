use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn mc_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mc")
}

#[test]
fn initialized_request_returns_result_and_list_changed_notification() {
    let tmp = tempdir().expect("tmp");
    let mc_home = tmp.path().join("mc-home");
    fs::create_dir_all(&mc_home).expect("mc_home");
    let session = json!({
        "token": "mcs_test-token",
        "subject": "test-user",
        "expires_at": "2099-01-01T00:00:00Z",
        "base_url": "http://127.0.0.1:9",
        "session_id": null
    });
    fs::write(
        mc_home.join("session.json"),
        serde_json::to_string(&session).unwrap(),
    )
    .expect("session.json");

    let mut child = Command::new(mc_bin())
        .args(["serve"])
        .env("MC_BASE_URL", "http://127.0.0.1:9")
        .env("MC_HOME", &mc_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mc serve");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"test","version":"1"}}}}}}"#
        )
        .expect("write initialize");
        writeln!(
            stdin,
            r#"{{"jsonrpc":"2.0","id":2,"method":"initialized","params":{{}}}}"#
        )
        .expect("write initialized");
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("wait output");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut saw_init_response = false;
    let mut saw_initialized_result = false;
    let mut saw_list_changed = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.contains('{') {
            continue;
        }
        let start = match trimmed.find('{') {
            Some(idx) => idx,
            None => continue,
        };
        let end = match trimmed.rfind('}') {
            Some(idx) => idx,
            None => continue,
        };
        let candidate = &trimmed[start..=end];
        let Ok(msg) = serde_json::from_str::<Value>(candidate) else {
            continue;
        };
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if msg.get("id") == Some(&Value::from(1)) && msg.get("result").is_some() {
            saw_init_response = true;
        }
        if msg.get("id") == Some(&Value::from(2)) && msg.get("result").is_some() {
            saw_initialized_result = true;
        }
        if method == "notifications/tools/list_changed" {
            saw_list_changed = true;
        }
    }

    assert!(saw_init_response, "missing initialize response: {stdout}");
    assert!(
        saw_initialized_result,
        "missing request-style initialized result response: {stdout}"
    );
    assert!(
        saw_list_changed,
        "missing tools/list_changed notification: {stdout}"
    );
}
