//! Plugin-backed control path for ZellijHosted agents (feature-flagged).
//!
//! Routes `inject`/`cancel` through the `edgeplane-zrpc` Zellij plugin via
//! `zellij pipe` instead of the legacy `paste → 300ms sleep → send-keys Enter`
//! subprocess chain in [`crate::zellij_session`]. The win is focus-free
//! delivery with no fixed sleep and no focus race across the shared session
//! tree. The wire protocol lives in [`edgeplane_zrpc_proto`].
//!
//! Transport: `zellij --session <s> pipe --plugin file:<wasm> --name zrpc --
//! <ndjson-request>`; the plugin replies on the command's STDOUT, one
//! [`Response`] line correlated by request id.
//!
//! ## Rollout
//!
//! Gated by [`PluginRouting`], read from the environment so it can be enabled
//! per-profile (`research` first) without a recompile:
//! * `EDGEPLANE_ZRPC_PLUGIN_PATH` — path to the installed `edgeplane_zrpc.wasm`
//! * `EDGEPLANE_ZRPC_SESSIONS`    — comma-separated allowlist of session names
//!
//! Both must be set for a session to use the plugin path; otherwise the legacy
//! `zellij_session` path is used unchanged.
//!
//! ## Testability
//!
//! Pure surfaces ([`PluginRouting`], [`ZellijPluginClient::pipe_argv`],
//! [`parse_response`]) are unit-tested here. Live subprocess execution
//! (`inject`/`cancel`) is exercised in pre-merge integration against a real
//! Zellij session, not in default `cargo test` (same convention as
//! `zellij_session`).

use std::collections::HashSet;

use anyhow::{Result, bail};
use edgeplane_zrpc_proto::{Request, Response};

use crate::zellij_session::zellij_binary;

/// Pipe name the `edgeplane-zrpc` plugin listens on (must match the plugin's
/// `CONTROL_PIPE`).
pub const ZRPC_PIPE_NAME: &str = "zrpc";

/// Feature-flag routing for the plugin-backed control path.
#[derive(Debug, Clone, Default)]
pub struct PluginRouting {
    sessions: HashSet<String>,
    plugin_path: Option<String>,
}

impl PluginRouting {
    /// Read routing from the environment (see module docs).
    pub fn from_env() -> Self {
        Self::from_parts(
            std::env::var("EDGEPLANE_ZRPC_SESSIONS").as_deref().unwrap_or(""),
            std::env::var("EDGEPLANE_ZRPC_PLUGIN_PATH").ok(),
        )
    }

    /// Build from explicit parts (the env-independent core, for tests).
    pub fn from_parts(sessions_csv: &str, plugin_path: Option<String>) -> Self {
        let sessions = sessions_csv
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        let plugin_path = plugin_path.filter(|p| !p.trim().is_empty());
        Self {
            sessions,
            plugin_path,
        }
    }

    /// True iff a plugin path is configured AND `session` is allowlisted.
    pub fn enabled_for(&self, session: &str) -> bool {
        self.plugin_path.is_some() && self.sessions.contains(session)
    }

    /// The configured wasm path, if any.
    pub fn plugin_path(&self) -> Option<&str> {
        self.plugin_path.as_deref()
    }
}

/// Drives the `edgeplane-zrpc` plugin in one Zellij session via `zellij pipe`.
///
/// Pipes **by name** to the already-running plugin. The plugin MUST be
/// pre-loaded via the session's Zellij config (`plugins {}` + `load_plugins
/// {}`) — the install tooling writes that config. The on-demand
/// `--plugin file:<wasm>` "first-message load" form is deliberately NOT used:
/// it fails to instantiate on the running 0.44.3 fleet (verified 2026-05-30,
/// erroring "could not find exported function" — identically for the
/// known-good `zellij-aria-fleet` plugin), whereas a config-preloaded plugin
/// accepts pipes fine by name.
pub struct ZellijPluginClient {
    session: String,
}

impl ZellijPluginClient {
    pub fn new(session: impl Into<String>) -> Self {
        Self {
            session: session.into(),
        }
    }

    /// argv for `zellij --session <s> pipe --name zrpc -- <payload>`.
    pub fn pipe_argv(&self, payload: &str) -> Vec<String> {
        vec![
            "--session".into(),
            self.session.clone(),
            "pipe".into(),
            "--name".into(),
            ZRPC_PIPE_NAME.into(),
            "--".into(),
            payload.into(),
        ]
    }

    /// Send one request and return the correlated response. Live subprocess —
    /// integration-tested, not in default `cargo test`.
    async fn request(&self, req: Request) -> Result<Response> {
        let line = serde_json::to_string(&req)?;
        let argv = self.pipe_argv(&line);
        let out = tokio::process::Command::new(zellij_binary())
            .args(&argv)
            .env_remove("ZELLIJ")
            .env_remove("ZELLIJ_SESSION_NAME")
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("zellij pipe failed for session {}: {e}", self.session))?;
        anyhow::ensure!(
            out.status.success(),
            "zellij pipe exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        parse_response(&String::from_utf8_lossy(&out.stdout), &req.id)
    }

    /// Focus-free inject of `text` into `pane_id`.
    pub async fn inject(&self, pane_id: &str, text: &str) -> Result<()> {
        let resp = self.request(Request::inject(new_id(), pane_id, text)).await?;
        into_unit(resp)
    }

    /// Interrupt whatever is running in `pane_id`.
    pub async fn cancel(&self, pane_id: &str) -> Result<()> {
        let resp = self.request(Request::cancel(new_id(), pane_id)).await?;
        into_unit(resp)
    }
}

/// Collapse an ok/err [`Response`] into `Result<()>`.
fn into_unit(resp: Response) -> Result<()> {
    if resp.ok {
        Ok(())
    } else {
        bail!("zrpc error: {}", resp.error.unwrap_or_else(|| "unknown".into()))
    }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Find the response line correlated to `id` in the plugin's NDJSON STDOUT.
/// Ignores unrelated/blank/garbage lines; errors if no match is present.
pub fn parse_response(stdout: &str, id: &str) -> Result<Response> {
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(resp) = serde_json::from_str::<Response>(line)
            && resp.id == id
        {
            return Ok(resp);
        }
    }
    bail!(
        "no zrpc response for request id {id} in plugin output ({} bytes)",
        stdout.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PluginRouting ───────────────────────────────────────────────────

    #[test]
    fn routing_disabled_without_plugin_path() {
        let r = PluginRouting::from_parts("research,work", None);
        assert!(!r.enabled_for("research"));
    }

    #[test]
    fn routing_disabled_when_session_not_listed() {
        let r = PluginRouting::from_parts("research", Some("/p/x.wasm".into()));
        assert!(!r.enabled_for("work"));
    }

    #[test]
    fn routing_enabled_when_listed_and_path_present() {
        let r = PluginRouting::from_parts("research,work", Some("/p/x.wasm".into()));
        assert!(r.enabled_for("research"));
        assert!(r.enabled_for("work"));
        assert_eq!(r.plugin_path(), Some("/p/x.wasm"));
    }

    #[test]
    fn routing_trims_and_ignores_blank_csv_entries() {
        let r = PluginRouting::from_parts("  research , , work ,", Some("/p/x.wasm".into()));
        assert!(r.enabled_for("research"));
        assert!(r.enabled_for("work"));
        assert!(!r.enabled_for(""));
    }

    #[test]
    fn routing_treats_blank_path_as_unset() {
        let r = PluginRouting::from_parts("research", Some("   ".into()));
        assert!(!r.enabled_for("research"));
        assert_eq!(r.plugin_path(), None);
    }

    // ── pipe_argv ───────────────────────────────────────────────────────

    #[test]
    fn pipe_argv_pipes_by_name_without_plugin_flag() {
        let c = ZellijPluginClient::new("research");
        let argv = c.pipe_argv(r#"{"id":"1","method":"cancel"}"#);
        assert_eq!(
            argv,
            vec![
                "--session",
                "research",
                "pipe",
                "--name",
                "zrpc",
                "--",
                r#"{"id":"1","method":"cancel"}"#,
            ]
        );
        // The on-demand load form must NOT be used (fails on 0.44.3).
        assert!(!argv.iter().any(|a| a == "--plugin"));
    }

    // ── parse_response ──────────────────────────────────────────────────

    #[test]
    fn parse_response_finds_matching_id() {
        let stdout = concat!(
            r#"{"id":"a","ok":true,"result":{"injected":true}}"#,
            "\n",
            r#"{"id":"b","ok":false,"error":"nope"}"#,
            "\n"
        );
        let r = parse_response(stdout, "b").unwrap();
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("nope"));
    }

    #[test]
    fn parse_response_skips_blank_and_garbage_lines() {
        let stdout = "\nnot json\n  \n{\"id\":\"a\",\"ok\":true}\n";
        let r = parse_response(stdout, "a").unwrap();
        assert!(r.ok);
        assert_eq!(r.id, "a");
    }

    #[test]
    fn parse_response_errors_when_absent() {
        let stdout = r#"{"id":"x","ok":true}"#;
        assert!(parse_response(stdout, "missing").is_err());
    }
}
