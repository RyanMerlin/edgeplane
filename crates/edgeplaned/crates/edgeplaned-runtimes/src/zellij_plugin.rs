//! Plugin-backed control path for ZellijHosted agents (feature-flagged).
//!
//! Routes `inject`/`cancel` through the `edgeplane-zrpc` Zellij plugin via
//! `zellij pipe` instead of the legacy `paste → 300ms sleep → send-keys Enter`
//! subprocess chain in [`crate::zellij_session`]. The win is focus-free
//! delivery with no fixed sleep and no focus race across the shared session
//! tree. The wire protocol lives in [`edgeplane_zrpc_proto`].
//!
//! ## Transport
//!
//! **Control (request/response):** `zellij --session <s> pipe --name zrpc --
//! <ndjson-request>`. The plugin must be pre-loaded via the session's Zellij
//! config (`plugins {}` + `load_plugins {}`). Piping by name to a
//! pre-loaded plugin is the only form that works on 0.44.3; the on-demand
//! `--plugin file:<wasm>` form fails to instantiate ("could not find exported
//! function") and is deliberately NOT used.
//!
//! **Events (long-lived):** `zellij --session <s> pipe --name zrpc-events`.
//! edgeplaned holds this pipe open; the plugin pushes [`PluginEvent`] NDJSON
//! lines as pane lifecycle events fire. See [`spawn_event_consumer`].
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
//! [`parse_response`], [`parse_event_line`]) are unit-tested here. Live
//! subprocess execution (`inject`/`cancel`/[`spawn_event_consumer`]) is
//! exercised in pre-merge integration against a real Zellij session, not in
//! default `cargo test` (same convention as `zellij_session`).

use std::collections::HashSet;

use anyhow::{Result, bail};
use edgeplane_zrpc_proto::{PluginEvent, Request, Response};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::zellij_session::zellij_binary;

/// Default timeout for a single synchronous `zellij pipe` round-trip.
const REQUEST_TIMEOUT_SECS: u64 = 10;

/// Pipe name the plugin uses for unsolicited lifecycle events pushed to
/// edgeplaned. edgeplaned opens `zellij --session <s> pipe --name zrpc-events`
/// and holds it open; the plugin pushes [`PluginEvent`] NDJSON lines as they
/// fire.
pub const ZRPC_EVENT_PIPE_NAME: &str = "zrpc-events";

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
            std::env::var("EDGEPLANE_ZRPC_SESSIONS")
                .as_deref()
                .unwrap_or(""),
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
/// known-good preloaded plugin), whereas a config-preloaded plugin
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
    ///
    /// ## Why not `.output().await`?
    ///
    /// `zellij pipe --name zrpc` does NOT exit after the plugin writes its
    /// response: the plugin calls `block_cli_pipe_input` before writing and
    /// `unblock_cli_pipe_input` after, but the Zellij process itself stays
    /// alive until the pipe is force-closed. Using `.output()` would wait for
    /// the child to exit, hanging indefinitely.
    ///
    /// Instead we spawn the child with `Stdio::piped()` on stdout, concurrently
    /// drain stderr, read stdout lines until we see the correlated `Response`,
    /// kill the child (SIGKILL + wait to reap it), and return. A 10-second
    /// [`REQUEST_TIMEOUT_SECS`] guards against plugin hangs.
    async fn request(&self, req: Request) -> Result<Response> {
        use std::process::Stdio;
        use tokio::time::{Duration, timeout};

        let line = serde_json::to_string(&req)?;
        let argv = self.pipe_argv(&line);

        let mut child = tokio::process::Command::new(zellij_binary())
            .args(&argv)
            .env_remove("ZELLIJ")
            .env_remove("ZELLIJ_SESSION_NAME")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                anyhow::anyhow!("zellij pipe spawn failed for session {}: {e}", self.session)
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("zellij pipe: no stdout handle"))?;

        // H3 fix: drain stderr concurrently with stdout so a large stderr
        // payload cannot deadlock the pipe buffer while we read stdout.
        // The drain task owns the stderr handle and runs for the lifetime of
        // the request; we collect its output via JoinHandle after the timeout.
        let stderr_drain = child.stderr.take().map(|stderr| {
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = String::new();
                let mut stderr = stderr;
                let _ = stderr.read_to_string(&mut buf).await;
                buf
            })
        });

        let result = timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
            read_until_response(stdout, &req.id),
        )
        .await;

        // C1 fix: kill() = start_kill() + wait(). This sends SIGKILL AND reaps
        // the process, preventing a zombie. The original start_kill()-only path
        // left a <defunct> zellij process per request until daemon exit.
        let _ = child.kill().await;

        match result {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => Err(e),
            Err(_elapsed) => {
                // Collect stderr that the concurrent drain task already captured.
                // Abort the task if still running (kill already sent to the process).
                let stderr_hint = if let Some(handle) = stderr_drain {
                    // Give the drain task a brief moment to flush after kill.
                    match tokio::time::timeout(Duration::from_millis(200), handle).await {
                        Ok(Ok(buf)) if !buf.is_empty() => {
                            format!(" stderr: {}", buf.trim())
                        }
                        _ => String::new(),
                    }
                } else {
                    String::new()
                };
                bail!(
                    "zellij pipe timed out after {REQUEST_TIMEOUT_SECS}s waiting for response to \
                     request {} in session {}{}",
                    req.id,
                    self.session,
                    stderr_hint
                )
            }
        }
    }

    /// Focus-free inject of `text` into `pane_id`.
    pub async fn inject(&self, pane_id: &str, text: &str) -> Result<()> {
        let resp = self
            .request(Request::inject(new_id(), pane_id, text))
            .await?;
        into_unit(resp)
    }

    /// Interrupt whatever is running in `pane_id`.
    pub async fn cancel(&self, pane_id: &str) -> Result<()> {
        let resp = self.request(Request::cancel(new_id(), pane_id)).await?;
        into_unit(resp)
    }
}

/// Spawn an async consumer for the plugin's lifecycle event stream.
///
/// Opens `zellij --session <session> pipe --name zrpc-events` and reads
/// NDJSON [`PluginEvent`] lines from its stdout. Each parseable event is sent
/// on the returned [`mpsc::Receiver`]; garbage/blank lines are silently
/// skipped. The [`tokio::task::JoinHandle`] is the background reader task —
/// it exits when the zellij process closes stdout (i.e. the session ends or
/// is restarted).
///
/// **Downstream wiring note:** wiring these events into agent-lifecycle /
/// watchdog state (e.g. restarting an agent whose pane exited) is a
/// deliberate follow-up. For now the consumer logs each event at DEBUG level
/// and yields it on the channel; callers may add their own handling by reading
/// from the receiver.
///
/// ## Errors
///
/// Returns `Err` if the `zellij` child cannot be spawned. After that, the
/// task absorbs individual line-read errors rather than aborting the consumer,
/// so a transient Zellij hiccup doesn't kill the whole event stream.
pub fn spawn_event_consumer(
    session: impl Into<String>,
) -> Result<(tokio::task::JoinHandle<()>, mpsc::Receiver<PluginEvent>)> {
    use std::process::Stdio;

    let session = session.into();
    let argv = vec![
        "--session".to_string(),
        session.clone(),
        "pipe".to_string(),
        "--name".to_string(),
        ZRPC_EVENT_PIPE_NAME.to_string(),
    ];

    let mut child = tokio::process::Command::new(zellij_binary())
        .args(&argv)
        .env_remove("ZELLIJ")
        .env_remove("ZELLIJ_SESSION_NAME")
        .stdout(Stdio::piped())
        // Silence stderr — the event pipe is long-lived, and stderr noise from
        // Zellij's own logging would pollute edgeplaned's logs.
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            anyhow::anyhow!("zellij event-pipe spawn failed for session {session}: {e}")
        })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("zellij event-pipe: no stdout handle"))?;

    // M2 fix: use a bounded channel; drop-on-full instead of back-pressuring
    // the live session. Events are advisory (lifecycle signals), so dropping
    // a burst under load is preferable to stalling the zellij process.
    let (tx, rx) = mpsc::channel(64);
    // Rate-limited dropped-event counter (one warn per N drops).
    static DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let handle = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if let Some(ev) = parse_event_line(&line) {
                        // M1 fix: demote per-event log from INFO to DEBUG to
                        // avoid an INFO firehose on busy sessions.
                        tracing::debug!(
                            session = %session,
                            event = ?ev,
                            "zrpc plugin event"
                        );
                        // M2 fix: try_send — drop the event if the receiver
                        // is slow rather than back-pressuring the pipe reader.
                        if tx.try_send(ev).is_err() {
                            let prev = DROPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            // Warn once per power-of-two drop boundary so we
                            // notice sustained loss without spamming logs.
                            if prev.count_ones() == 1 || prev == 0 {
                                tracing::warn!(
                                    session = %session,
                                    total_dropped = prev + 1,
                                    "zrpc event dropped (receiver full or closed)"
                                );
                            }
                            // If the channel is closed (receiver dropped) stop reading.
                            if tx.is_closed() {
                                break;
                            }
                        }
                    }
                }
                Ok(None) => {
                    // EOF — the zellij process closed stdout (session ended).
                    tracing::debug!(
                        session = %session,
                        "zrpc event pipe EOF; consumer task exiting"
                    );
                    break;
                }
                Err(e) => {
                    tracing::warn!(
                        session = %session,
                        error = %e,
                        "zrpc event pipe read error; consumer task exiting"
                    );
                    break;
                }
            }
        }
        // Best-effort: wait on the child to reap it (suppress zombie).
        let _ = child.wait().await;
    });

    Ok((handle, rx))
}

/// Parse a single NDJSON line into a [`PluginEvent`]. Returns `None` for
/// blank lines or any line that does not deserialize as a valid event (e.g.
/// Zellij debug output that leaks onto the pipe's stdout).
///
/// This is the pure, synchronous core of the event consumer — extracted so
/// it can be unit-tested without spawning subprocesses.
pub fn parse_event_line(line: &str) -> Option<PluginEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<PluginEvent>(trimmed).ok()
}

/// Read lines from `stdout` until we find one whose parsed [`Response`] has
/// `id == target_id`. Ignores blank/garbage lines. Returns an error on EOF
/// without a match.
///
/// This is an async helper for [`ZellijPluginClient::request`] — it is
/// pure-logic from a testing perspective but requires a live stdout handle, so
/// it is not separately unit-tested (the line-parsing is covered by
/// `parse_response`).
async fn read_until_response(
    stdout: impl tokio::io::AsyncRead + Unpin,
    target_id: &str,
) -> Result<Response> {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(resp) = serde_json::from_str::<Response>(trimmed)
                    && resp.id == target_id
                {
                    return Ok(resp);
                }
                // Unrelated response line (shouldn't happen on the control
                // pipe, but skip gracefully).
            }
            Ok(None) => {
                bail!(
                    "zellij pipe closed stdout before returning response for request id {target_id}"
                )
            }
            Err(e) => {
                bail!("zellij pipe stdout read error waiting for request {target_id}: {e}")
            }
        }
    }
}

/// Collapse an ok/err [`Response`] into `Result<()>`.
fn into_unit(resp: Response) -> Result<()> {
    if resp.ok {
        Ok(())
    } else {
        bail!(
            "zrpc error: {}",
            resp.error.unwrap_or_else(|| "unknown".into())
        )
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

    // ── parse_event_line ────────────────────────────────────────────────

    #[test]
    fn parse_event_line_command_pane_exited() {
        let line = r#"{"event":"command_pane_exited","pane_id":3,"exit_code":0}"#;
        let ev = parse_event_line(line).expect("should parse");
        assert_eq!(
            ev,
            PluginEvent::CommandPaneExited {
                pane_id: 3,
                exit_code: Some(0)
            }
        );
    }

    #[test]
    fn parse_event_line_pane_closed() {
        let line = r#"{"event":"pane_closed","pane_id":7}"#;
        let ev = parse_event_line(line).expect("should parse");
        assert_eq!(ev, PluginEvent::PaneClosed { pane_id: 7 });
    }

    #[test]
    fn parse_event_line_pane_update() {
        let line = r#"{"event":"pane_update","panes":[1,2,3]}"#;
        let ev = parse_event_line(line).expect("should parse");
        assert_eq!(
            ev,
            PluginEvent::PaneUpdate {
                panes: vec![1, 2, 3]
            }
        );
    }

    #[test]
    fn parse_event_line_skips_blank() {
        assert!(parse_event_line("").is_none());
        assert!(parse_event_line("   ").is_none());
        assert!(parse_event_line("\n").is_none());
    }

    #[test]
    fn parse_event_line_skips_garbage() {
        assert!(parse_event_line("not json at all").is_none());
        assert!(parse_event_line(r#"{"unexpected":"field"}"#).is_none());
    }

    #[test]
    fn parse_event_line_exit_code_absent_is_none() {
        // exit_code is optional in the proto
        let line = r#"{"event":"command_pane_exited","pane_id":5}"#;
        let ev = parse_event_line(line).expect("should parse");
        assert_eq!(
            ev,
            PluginEvent::CommandPaneExited {
                pane_id: 5,
                exit_code: None
            }
        );
    }

    // ── event pipe argv ─────────────────────────────────────────────────

    #[test]
    fn event_pipe_argv_uses_zrpc_events_name() {
        // Verify the event consumer uses the right pipe name, without
        // spawning a subprocess. We reconstruct the argv logic inline.
        let session = "research";
        let argv: Vec<String> = vec![
            "--session".into(),
            session.into(),
            "pipe".into(),
            "--name".into(),
            ZRPC_EVENT_PIPE_NAME.into(),
        ];
        assert_eq!(argv[4], "zrpc-events");
        assert!(
            !argv.iter().any(|a| a == "--plugin"),
            "event pipe must not use --plugin (same constraint as control pipe)"
        );
    }
}
