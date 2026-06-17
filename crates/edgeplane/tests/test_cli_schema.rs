use std::process::Command;

#[test]
fn discover_stdout_is_valid_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_edgeplane"))
        .arg("discover")
        .arg("--deep")
        .output()
        .expect("failed to run edgeplane discover --deep");
    assert!(output.status.success(), "edgeplane discover --deep should exit 0");
    let stdout = String::from_utf8(output.stdout).expect("non-utf8 output");
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("discover --deep output should be valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["binary"], "edgeplane");
    assert!(parsed["root"]["subcommands"].as_array().unwrap().len() > 5);
}

#[test]
fn discover_includes_agent_subcommand() {
    let output = Command::new(env!("CARGO_BIN_EXE_edgeplane"))
        .arg("discover")
        .arg("--deep")
        .output()
        .expect("failed to run edgeplane discover --deep");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let subcommands = parsed["root"]["subcommands"].as_array().unwrap();
    let agent = subcommands.iter().find(|s| s["name"] == "agent");
    assert!(agent.is_some(), "should have 'agent' in top-level subcommands");
    assert!(!agent.unwrap()["subcommands"].as_array().unwrap().is_empty());
}

#[test]
fn discover_default_depth_one_no_nested_subcommands() {
    // Without --deep, each top-level subcommand's children should be empty.
    let output = Command::new(env!("CARGO_BIN_EXE_edgeplane"))
        .arg("discover")
        .output()
        .expect("failed to run edgeplane discover");
    assert!(output.status.success(), "edgeplane discover should exit 0");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let subcommands = parsed["root"]["subcommands"].as_array().unwrap();
    for sub in subcommands {
        let nested = sub["subcommands"].as_array();
        assert!(
            nested.map(|v| v.is_empty()).unwrap_or(true),
            "top-level '{}' should have no nested subcommands without --deep",
            sub["name"]
        );
    }
}

#[test]
fn discover_path_drills_into_subtree() {
    // `edgeplane discover agent` should return the agent subtree at the root.
    let output = Command::new(env!("CARGO_BIN_EXE_edgeplane"))
        .arg("discover")
        .arg("agent")
        .output()
        .expect("failed to run edgeplane discover agent");
    assert!(output.status.success(), "edgeplane discover agent should exit 0");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // root should be the agent node itself
    assert_eq!(parsed["root"]["name"], "agent");
}
