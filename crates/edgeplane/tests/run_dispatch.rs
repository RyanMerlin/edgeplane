//! Dispatch-surface tests for the unified `edgeplane run` command and the
//! removal of `edgeplane launch`. These exercise arg parsing and the
//! dispatch-level error paths only — no tower connection is required.

use std::process::Command;

fn edgeplane() -> Command {
    Command::new(env!("CARGO_BIN_EXE_edgeplane"))
}

#[test]
fn unknown_runtime_is_rejected_with_helpful_message() {
    // `run` is an online command, so it requires a configured server URL. The
    // unknown-runtime check fires before any network I/O, so a dummy (unreached)
    // EP_BASE_URL is enough to get past the "no server configured" startup gate
    // without making a tower connection.
    let out = edgeplane()
        .env("EP_BASE_URL", "http://localhost:8008")
        .args(["run", "definitely-not-a-runtime"])
        .output()
        .expect("spawn edgeplane");
    assert!(!out.status.success(), "expected non-zero exit for unknown runtime");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown runtime") && stderr.contains("openclaw"),
        "stderr should name the unknown runtime and list known ones, got:\n{stderr}"
    );
}

#[test]
fn launch_command_is_removed() {
    let out = edgeplane()
        .args(["launch", "gemini"])
        .output()
        .expect("spawn edgeplane");
    assert!(!out.status.success(), "`edgeplane launch` should no longer exist");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unrecognized subcommand"),
        "stderr should report an unrecognized subcommand, got:\n{stderr}"
    );
}

#[test]
fn run_help_lists_all_five_runtimes() {
    let out = edgeplane().args(["run", "--help"]).output().expect("spawn edgeplane");
    assert!(out.status.success(), "`run --help` should succeed");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for rt in ["claude", "codex", "gemini", "openclaw", "custom"] {
        assert!(
            combined.contains(rt),
            "`run --help` should mention runtime '{rt}', got:\n{combined}"
        );
    }
}

#[test]
fn top_level_help_exposes_run_and_not_launch() {
    let out = edgeplane().args(["--help"]).output().expect("spawn edgeplane");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(combined.contains("run"), "top-level help should list `run`");
    assert!(
        !combined.lines().any(|l| l.trim_start().starts_with("launch")),
        "`launch` should not be listed as a subcommand, got:\n{combined}"
    );
}
