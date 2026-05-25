use std::process::Command;

#[test]
fn cli_schema_stdout_is_valid_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_edgeplane"))
        .arg("cli-schema")
        .output()
        .expect("failed to run edgeplane cli-schema");
    assert!(output.status.success(), "cli-schema should exit 0");
    let stdout = String::from_utf8(output.stdout).expect("non-utf8 output");
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("cli-schema output should be valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["binary"], "edgeplane");
    assert!(parsed["root"]["subcommands"].as_array().unwrap().len() > 5);
}

#[test]
fn cli_schema_includes_agent_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_edgeplane"))
        .arg("cli-schema")
        .output()
        .expect("failed to run edgeplane cli-schema");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let subcommands = parsed["root"]["subcommands"].as_array().unwrap();
    let agent = subcommands.iter().find(|s| s["name"] == "agent");
    assert!(agent.is_some(), "should have 'agent' in top-level subcommands");
    assert!(agent.unwrap()["subcommands"].as_array().unwrap().len() > 0);
}
