//! Subprocess wrappers around `herdr` for HerdrHosted agents.
//!
//! Mirrors zellij_session.rs's structure and testability pattern (every
//! method that shells out has a separate `*_argv` helper, unit-testable
//! without a subprocess). Two load-bearing differences from Zellij:
//!
//! - Session targeting is via the `HERDR_SESSION` environment variable, not
//!   a `--session` CLI flag.
//! - `agent prompt --wait` is an atomic submit + lifecycle-state wait
//!   (idle/working/blocked/done, with an explicit `agent_prompt_stalled`
//!   failure signal) — not a blind paste+sleep+Enter. This is strictly
//!   better than Zellij's approach and needs no plugin-routing complexity
//!   to approximate it (see zellij_hosted.rs's PluginRouting — there is no
//!   Herdr equivalent because none is needed).

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::process::Command;
use std::sync::OnceLock;

/// Fallback pane id for a freshly-created Herdr session's first pane. Real
/// dispatch discovers the pane via `discover_pane_id` — this is only the
/// fallback when discovery fails (mirrors ariad's HerdrDispatcher in the
/// `aria-rs` repo, which established this same pattern).
pub const DEFAULT_PANE_ID: &str = "w1:p1";

pub struct HerdrSession {
    pub name: String,
}

impl HerdrSession {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Check whether the session is currently running on this node.
    /// `herdr session list --json` is serverless/global — works with no
    /// server running and needs no `HERDR_SESSION`.
    pub fn is_alive(&self) -> bool {
        let out = match Command::new(herdr_binary())
            .args(["session", "list", "--json"])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return false,
        };
        let parsed: SessionListOutput = match serde_json::from_slice(&out.stdout) {
            Ok(p) => p,
            Err(_) => return false,
        };
        parsed
            .sessions
            .iter()
            .any(|s| s.name == self.name && s.running)
    }

    /// Discover the pane_id of the agent named "claude" in this session.
    /// Falls back to DEFAULT_PANE_ID with a loud warning if detection fails
    /// — never silently guesses wrong without saying so.
    pub fn discover_pane_id(&self) -> String {
        match self.agent_list() {
            Ok(agents) => match agents.iter().find(|a| a.agent == "claude") {
                Some(a) => a.pane_id.clone(),
                None => {
                    tracing::warn!(
                        "HerdrSession({}): no 'claude' agent found in `agent list`; \
                         falling back to {DEFAULT_PANE_ID}",
                        self.name
                    );
                    DEFAULT_PANE_ID.to_string()
                }
            },
            Err(e) => {
                tracing::warn!(
                    "HerdrSession({}): agent list failed ({e:#}); falling back to {DEFAULT_PANE_ID}",
                    self.name
                );
                DEFAULT_PANE_ID.to_string()
            }
        }
    }

    fn agent_list(&self) -> Result<Vec<AgentListEntry>> {
        let out = self
            .build_cmd(&["agent".to_string(), "list".to_string()])
            .output()
            .with_context(|| format!("herdr agent list failed for session {}", self.name))?;
        anyhow::ensure!(
            out.status.success(),
            "herdr agent list exited with {}",
            out.status
        );
        let envelope: AgentListEnvelope = serde_json::from_slice(&out.stdout)
            .with_context(|| format!("parsing agent list JSON for session {}", self.name))?;
        Ok(envelope.result.agents)
    }

    /// Deliver a prompt: atomic submit + lifecycle-state wait. Distinguishes
    /// a stalled submission from other failures so callers can tell "the
    /// agent never acknowledged" from "herdr itself errored."
    pub fn send_prompt(&self, pane_id: &str, text: &str) -> Result<()> {
        let argv = self.prompt_argv(pane_id, text);
        let out = self
            .build_cmd(&argv)
            .output()
            .with_context(|| format!("herdr agent prompt failed for session {}", self.name))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("agent_prompt_stalled") {
                bail!(
                    "herdr agent prompt stalled for session {} pane {pane_id}: {stderr}",
                    self.name
                );
            }
            bail!(
                "herdr agent prompt exited {} for session {}: {stderr}",
                out.status,
                self.name
            );
        }
        Ok(())
    }

    pub fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        let argv = self.send_keys_argv(pane_id, keys);
        let status = self
            .build_cmd(&argv)
            .status()
            .with_context(|| format!("herdr pane send-keys failed for session {}", self.name))?;
        anyhow::ensure!(
            status.success(),
            "herdr pane send-keys exited with {status}"
        );
        Ok(())
    }

    /// Read recent pane output. `lines` mirrors herdr's own `--lines <N>`
    /// flag — herdr has no `--full` scrollback flag like Zellij's
    /// `dump-screen --full`, so callers that want "as much as possible"
    /// should pass a large explicit value.
    pub fn dump_screen(&self, pane_id: &str, lines: Option<u32>) -> Result<String> {
        let argv = self.read_argv(pane_id, lines);
        let out = self
            .build_cmd(&argv)
            .output()
            .with_context(|| format!("herdr pane read failed for session {}", self.name))?;
        anyhow::ensure!(
            out.status.success(),
            "herdr pane read exited with {}",
            out.status
        );
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    // -- argv builders (testable without subprocess; HERDR_SESSION is set by build_cmd, not argv) --

    pub fn prompt_argv(&self, pane_id: &str, text: &str) -> Vec<String> {
        vec![
            "agent".into(),
            "prompt".into(),
            pane_id.into(),
            text.into(),
            "--wait".into(),
            "--until".into(),
            "working".into(),
            "--until".into(),
            "blocked".into(),
            "--timeout".into(),
            "15000".into(),
        ]
    }

    pub fn send_keys_argv(&self, pane_id: &str, keys: &[&str]) -> Vec<String> {
        let mut v = vec![
            "pane".to_string(),
            "send-keys".to_string(),
            pane_id.to_string(),
        ];
        v.extend(keys.iter().map(|k| k.to_string()));
        v
    }

    pub fn read_argv(&self, pane_id: &str, lines: Option<u32>) -> Vec<String> {
        let mut v = vec![
            "pane".to_string(),
            "read".to_string(),
            pane_id.to_string(),
            "--source".to_string(),
            "recent-unwrapped".to_string(),
        ];
        if let Some(n) = lines {
            v.push("--lines".to_string());
            v.push(n.to_string());
        }
        v
    }

    fn build_cmd(&self, argv: &[String]) -> Command {
        let mut cmd = Command::new(herdr_binary());
        cmd.env("HERDR_SESSION", &self.name);
        for a in argv {
            cmd.arg(a);
        }
        cmd
    }
}

#[derive(Debug, Deserialize)]
struct SessionListOutput {
    sessions: Vec<SessionEntry>,
}

#[derive(Debug, Deserialize)]
struct SessionEntry {
    name: String,
    running: bool,
}

#[derive(Debug, Deserialize)]
struct AgentListEnvelope {
    result: AgentListResult,
}

#[derive(Debug, Deserialize)]
struct AgentListResult {
    agents: Vec<AgentListEntry>,
}

#[derive(Debug, Deserialize)]
struct AgentListEntry {
    agent: String,
    pane_id: String,
}

/// Resolve the `herdr` binary. Probes candidate paths once, caches the
/// result for the process lifetime. Falls back to bare `"herdr"` so the
/// resulting `Command` still attempts PATH lookup and surfaces a useful
/// error from the child process rather than from this function.
///
/// systemd `--user` services start with a stripped PATH — same reasoning
/// as `zellij_binary()` in zellij_session.rs.
pub fn herdr_binary() -> &'static str {
    static RESOLVED: OnceLock<String> = OnceLock::new();
    RESOLVED
        .get_or_init(|| {
            for candidate in herdr_candidates() {
                if std::process::Command::new(&candidate)
                    .arg("--version")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    return candidate;
                }
            }
            "herdr".to_string()
        })
        .as_str()
}

/// Candidate locations to probe, in priority order. Deliberately generic —
/// no Aria-specific paths (e.g. `~/.aria/bin`). If a deployment pins a
/// specific herdr binary, it does so by putting that path first in the
/// *systemd unit's* PATH, the same way Aria's own fleet units pin theirs;
/// this function has no opinion about that.
pub fn herdr_candidates() -> Vec<String> {
    let mut out = vec!["herdr".to_string()];
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".cargo/bin/herdr").to_string_lossy().into_owned());
        out.push(home.join(".local/bin/herdr").to_string_lossy().into_owned());
    }
    out.push("/usr/local/bin/herdr".into());
    out.push("/usr/bin/herdr".into());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_argv_shape() {
        let s = HerdrSession::new("vega");
        let argv = s.prompt_argv("w1:p1", "hello world");
        assert_eq!(
            argv,
            vec![
                "agent",
                "prompt",
                "w1:p1",
                "hello world",
                "--wait",
                "--until",
                "working",
                "--until",
                "blocked",
                "--timeout",
                "15000",
            ]
        );
    }

    #[test]
    fn send_keys_argv_single_key() {
        let s = HerdrSession::new("vega");
        let argv = s.send_keys_argv("w1:p1", &["ctrl+c"]);
        assert_eq!(argv, vec!["pane", "send-keys", "w1:p1", "ctrl+c"]);
    }

    #[test]
    fn read_argv_default_has_no_lines_flag() {
        let s = HerdrSession::new("vega");
        let argv = s.read_argv("w1:p1", None);
        assert_eq!(
            argv,
            vec!["pane", "read", "w1:p1", "--source", "recent-unwrapped"]
        );
    }

    #[test]
    fn read_argv_with_lines() {
        let s = HerdrSession::new("vega");
        let argv = s.read_argv("w1:p1", Some(200));
        assert_eq!(argv.last().unwrap(), "200");
        assert!(argv.contains(&"--lines".to_string()));
    }

    #[test]
    fn is_alive_parses_session_list_json() {
        let json = r#"{"sessions":[
            {"name":"vega","running":true},
            {"name":"gwyn","running":false}
        ]}"#;
        let parsed: SessionListOutput = serde_json::from_str(json).unwrap();
        assert!(parsed
            .sessions
            .iter()
            .any(|s| s.name == "vega" && s.running));
        assert!(!parsed
            .sessions
            .iter()
            .any(|s| s.name == "gwyn" && s.running));
    }

    #[test]
    fn agent_list_envelope_parses_real_shape() {
        // Captured live 2026-08-18 from `HERDR_SESSION=merlinlabs herdr agent list`.
        let json = r#"{"id":"cli:agent:list","result":{"agents":[{"agent":"claude",
            "agent_status":"idle","cwd":"/home/merlin/code/aria/profiles/merlinlabs",
            "focused":true,"foreground_cwd":"/tmp","pane_id":"w1:p1","revision":1,
            "state_change_seq":1,"tab_id":"w1:t1","terminal_id":"term_x",
            "terminal_title":"merlinlabs","terminal_title_stripped":"merlinlabs",
            "workspace_id":"w1"}],"type":"agent_list"}}"#;
        let envelope: AgentListEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.result.agents.len(), 1);
        assert_eq!(envelope.result.agents[0].agent, "claude");
        assert_eq!(envelope.result.agents[0].pane_id, "w1:p1");
    }

    #[test]
    fn herdr_candidates_includes_bare_and_local_bin() {
        let c = herdr_candidates();
        assert_eq!(c[0], "herdr");
        assert!(c.iter().any(|p| p.ends_with(".local/bin/herdr")));
        assert!(!c.iter().any(|p| p.contains(".aria/bin")));
    }
}
