//! Subprocess wrappers around `zellij action` for ZellijHosted agents.
//!
//! Ported from external fleet management tooling as part of Phase 2 of
//! the daemon-absorption plan. External fleet commands may coexist with
//! this code during a deprecation window (Phase 6); the two implementations
//! are deliberately behavior-identical so the dual period doesn't surface
//! differences.
//!
//! ## Testability
//!
//! Every method that constructs a [`Command`] does so via a `*_argv` helper
//! that returns `Vec<String>` first. Unit tests assert the argv shape
//! without invoking Zellij. Actual subprocess execution is gated behind
//! integration tests (run manually against a live Zellij in pre-merge
//! validation; not in default `cargo test`).

use anyhow::{Context, Result};
use std::process::Command;
use std::sync::OnceLock;

/// Pane id used for the primary terminal in agent sessions. Matches the
/// default first pane Zellij creates when a session starts. Hard-coded
/// because the default layout is always used; if an agent ever uses a
/// custom layout with a differently-named pane, lift this to
/// `AgentLaunchContext`.
pub const DEFAULT_PANE_ID: &str = "terminal_0";

pub struct ZellijSession {
    pub name: String,
}

impl ZellijSession {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Check whether the session is currently running on this node.
    ///
    /// Runs `zellij list-sessions --short` and looks for an exact line match.
    /// Strips `ZELLIJ*` env vars so this works correctly even when edgeplaned was
    /// started from inside another Zellij session.
    pub fn is_alive(&self) -> bool {
        match Command::new(zellij_binary())
            .args(["list-sessions", "--short"])
            .env_remove("ZELLIJ")
            .env_remove("ZELLIJ_SESSION_NAME")
            .output()
        {
            Ok(out) => String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line.trim() == self.name.as_str()),
            Err(_) => false,
        }
    }

    /// Send a prompt: paste the text, wait 300ms for Zellij to flush, send
    /// Enter. The 300ms is the established timing that prevents the Enter
    /// from arriving mid-paste.
    pub fn send_prompt(&self, text: &str) -> Result<()> {
        self.paste(DEFAULT_PANE_ID, text)?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        self.send_keys(DEFAULT_PANE_ID, &["Enter"])?;
        Ok(())
    }

    pub fn paste(&self, pane_id: &str, chars: &str) -> Result<()> {
        let argv = self.paste_argv(pane_id, chars);
        let status = build_cmd(&argv)
            .status()
            .with_context(|| format!("zellij paste failed for session {}", self.name))?;
        anyhow::ensure!(status.success(), "zellij paste exited with {status}");
        Ok(())
    }

    pub fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        let argv = self.send_keys_argv(pane_id, keys);
        let status = build_cmd(&argv)
            .status()
            .with_context(|| format!("zellij send-keys failed for session {}", self.name))?;
        anyhow::ensure!(status.success(), "zellij send-keys exited with {status}");
        Ok(())
    }

    pub fn dump_screen(&self, pane_id: &str, full: bool) -> Result<String> {
        let argv = self.dump_screen_argv(pane_id, full);
        let output = build_cmd(&argv)
            .output()
            .with_context(|| format!("zellij dump-screen failed for session {}", self.name))?;
        anyhow::ensure!(
            output.status.success(),
            "zellij dump-screen exited with {}",
            output.status
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    // -- argv builders (testable without subprocess) --

    pub fn paste_argv(&self, pane_id: &str, chars: &str) -> Vec<String> {
        vec![
            "--session".into(),
            self.name.clone(),
            "action".into(),
            "paste".into(),
            "--pane-id".into(),
            pane_id.into(),
            chars.into(),
        ]
    }

    pub fn send_keys_argv(&self, pane_id: &str, keys: &[&str]) -> Vec<String> {
        let mut v = vec![
            "--session".into(),
            self.name.clone(),
            "action".into(),
            "send-keys".into(),
            "--pane-id".into(),
            pane_id.into(),
        ];
        for key in keys {
            v.push((*key).into());
        }
        v
    }

    pub fn dump_screen_argv(&self, pane_id: &str, full: bool) -> Vec<String> {
        let mut v = vec![
            "--session".into(),
            self.name.clone(),
            "action".into(),
            "dump-screen".into(),
            "--pane-id".into(),
            pane_id.into(),
        ];
        if full {
            v.push("--full".into());
        }
        v
    }
}

fn build_cmd(argv: &[String]) -> Command {
    let mut cmd = Command::new(zellij_binary());
    for a in argv {
        cmd.arg(a);
    }
    cmd
}

/// Resolve the `zellij` binary. Probes candidate paths once, caches
/// the result for the process lifetime. Falls back to bare `"zellij"`
/// when nothing else works so the resulting `Command` still attempts
/// PATH lookup (and surfaces a useful error from the child process).
///
/// systemd `--user` services start with a stripped PATH that omits
/// `~/.cargo/bin` and `~/.local/bin` — the two places users typically
/// install zellij. `which::which` is unreliable inside these services
/// for the same reason. Probe explicitly.
pub fn zellij_binary() -> &'static str {
    static RESOLVED: OnceLock<String> = OnceLock::new();
    RESOLVED
        .get_or_init(|| {
            for candidate in zellij_candidates() {
                if std::process::Command::new(&candidate)
                    .arg("--version")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    return candidate;
                }
            }
            "zellij".to_string()
        })
        .as_str()
}

/// Candidate locations to probe, in priority order: bare `zellij` (for
/// the case where PATH is set), then `~/.cargo/bin/zellij` (cargo install
/// default), then `~/.local/bin/zellij`, then system paths.
pub fn zellij_candidates() -> Vec<String> {
    let mut out = vec!["zellij".to_string()];
    if let Some(home) = dirs::home_dir() {
        out.push(
            home.join(".cargo/bin/zellij")
                .to_string_lossy()
                .into_owned(),
        );
        out.push(
            home.join(".local/bin/zellij")
                .to_string_lossy()
                .into_owned(),
        );
    }
    out.push("/usr/local/bin/zellij".into());
    out.push("/usr/bin/zellij".into());
    out
}

// ── Screen classification ───────────────────────────────────────────────
//
// Heuristic scrape of the visible viewport (or a tail of it) to classify
// what state the agent appears to be in. Used for diagnostics; intentionally NOT
// gating the `signal()` send path (see plan D2.6).

/// True when the screen looks idle — Claude's prompt marker `❯` is visible
/// at the start of a line.
pub fn is_idle_screen(lines: &[&str]) -> bool {
    lines.iter().any(|l| {
        let t = l.trim();
        t == "❯" || t.starts_with("❯ ")
    })
}

/// Classify the state of the agent based on a tail of the visible viewport.
/// Returns one of: "idle", "working", "error", "unknown". Heuristic — depends
/// on Claude's exact output format and will need updates if that changes.
pub fn classify_state(tail: &[&str]) -> &'static str {
    if is_idle_screen(tail) {
        return "idle";
    }
    const SPINNERS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    for line in tail {
        let t = line.trim();
        if t.contains("Running tool") || SPINNERS.iter().any(|&c| t.contains(c)) {
            return "working";
        }
        if t.contains("Error:") || t.contains('✗') {
            return "error";
        }
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- argv shape --

    #[test]
    fn paste_argv_shape() {
        let s = ZellijSession::new("work");
        let argv = s.paste_argv("terminal_0", "hello world");
        assert_eq!(
            argv,
            vec![
                "--session",
                "work",
                "action",
                "paste",
                "--pane-id",
                "terminal_0",
                "hello world"
            ]
        );
    }

    #[test]
    fn send_keys_argv_single_key() {
        let s = ZellijSession::new("operator");
        let argv = s.send_keys_argv("terminal_0", &["Enter"]);
        assert_eq!(
            argv,
            vec![
                "--session",
                "operator",
                "action",
                "send-keys",
                "--pane-id",
                "terminal_0",
                "Enter"
            ]
        );
    }

    #[test]
    fn send_keys_argv_multi_key() {
        let s = ZellijSession::new("research");
        let argv = s.send_keys_argv("terminal_0", &["Ctrl c"]);
        assert_eq!(argv.last().unwrap(), "Ctrl c");
    }

    #[test]
    fn dump_screen_argv_default() {
        let s = ZellijSession::new("work");
        let argv = s.dump_screen_argv("terminal_0", false);
        assert!(!argv.iter().any(|a| a == "--full"));
        assert_eq!(argv.last().unwrap(), "terminal_0");
    }

    #[test]
    fn dump_screen_argv_full() {
        let s = ZellijSession::new("work");
        let argv = s.dump_screen_argv("terminal_0", true);
        assert_eq!(argv.last().unwrap(), "--full");
    }

    // -- screen classification --

    #[test]
    fn idle_detects_bare_prompt() {
        assert!(is_idle_screen(&["❯"]));
    }

    #[test]
    fn idle_detects_prompt_with_space() {
        assert!(is_idle_screen(&["❯ some context"]));
    }

    #[test]
    fn idle_handles_leading_whitespace() {
        assert!(is_idle_screen(&["  ❯ "]));
    }

    #[test]
    fn idle_returns_false_for_empty() {
        let v: Vec<&str> = vec![];
        assert!(!is_idle_screen(&v));
    }

    #[test]
    fn idle_returns_false_for_normal_output() {
        assert!(!is_idle_screen(&[
            "thinking...",
            "Running tool: web_search"
        ]));
    }

    #[test]
    fn classify_idle() {
        assert_eq!(classify_state(&["❯"]), "idle");
    }

    #[test]
    fn classify_working_via_running_tool() {
        assert_eq!(
            classify_state(&["Running tool: read_file", "path: foo.rs"]),
            "working"
        );
    }

    #[test]
    fn classify_working_via_spinner() {
        assert_eq!(classify_state(&["⠋ thinking..."]), "working");
    }

    #[test]
    fn classify_error() {
        assert_eq!(classify_state(&["Error: rate limit exceeded"]), "error");
    }

    #[test]
    fn classify_error_via_xmark() {
        assert_eq!(classify_state(&["✗ build failed"]), "error");
    }

    #[test]
    fn classify_unknown() {
        assert_eq!(classify_state(&["some random line"]), "unknown");
    }

    #[test]
    fn classify_prefers_idle_over_other_signals() {
        // If the prompt is visible, the agent is ready — even if there's
        // stale output from a previous task in the tail.
        assert_eq!(classify_state(&["Running tool: foo", "❯"]), "idle");
    }
}
