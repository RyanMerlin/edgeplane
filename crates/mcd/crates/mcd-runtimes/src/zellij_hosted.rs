//! `AgentRuntime` for agents hosted in a Zellij pane.
//!
//! Phase 2 of the daemon-absorption plan: real impls landed for `launch`
//! and `signal`. The runtime owns no agent process — the Zellij session is
//! externally managed by `aria-watchdog-rs` and systemd (Phase 5 absorbs
//! watchdog responsibilities into mcd). This runtime is a thin facade
//! that addresses the right pane via `zellij action` subprocesses.
//!
//! ## Attach surfaces
//!
//! `attach_pty` bails permanently — Zellij is a pane multiplexer, not a PTY.
//! CLI attach (`zellij attach <session>`) and web attach
//! (`http://127.0.0.1:8082/<session>`) are handled outside the trait by
//! the `mc agent attach` surface in Phase 3.
//!
//! ## Task-mode
//!
//! `inject_task` bails — ZellijHosted is persistent-only. Use `signal()`
//! with `AgentSignal::UserInput { text }` to deliver a prompt.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use futures::stream::BoxStream;
use mcd_core::agent_runtime::AgentRuntime;
use mcd_core::progress::ProgressEvent;
use mcd_core::types::{
    AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult,
    TaskSpec,
};
use tokio::sync::Mutex;

use crate::shared::merge_capabilities;
use crate::zellij_session::ZellijSession;

/// Per-agent state cached at `launch()` and read at `signal()`.
///
/// Holds the resolved Zellij session name and a serialisation mutex so two
/// concurrent `signal()` calls on the same agent can't interleave their
/// paste/Enter pairs (which would clobber the prompt).
struct AgentSession {
    zellij_session: String,
    mutex: Arc<Mutex<()>>,
}

pub struct ZellijHostedRuntime {
    capabilities: Vec<Capability>,
    version: String,
    sessions: Arc<Mutex<HashMap<String, AgentSession>>>,
}

impl ZellijHostedRuntime {
    pub fn new() -> Self {
        Self::with_extra_capabilities(Vec::new())
    }

    pub fn with_extra_capabilities(extra: Vec<Capability>) -> Self {
        let builtins = vec![
            Capability::new("zellij_hosted"),
            Capability::new("interactive_attach"),
            Capability::new("send_keys"),
        ];
        Self {
            capabilities: merge_capabilities(builtins, extra),
            version: "zellij_hosted 0.1 (Phase 2)".into(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for ZellijHostedRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentRuntime for ZellijHostedRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::ZellijHosted
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// Verify the externally-managed Zellij session is reachable and
    /// register the agent in the per-agent session map. Does NOT spawn
    /// anything — the session lifecycle is owned by systemd + aria-watchdog
    /// (Phase 5 absorbs watchdog into mcd).
    async fn launch(&self, ctx: LaunchContext) -> Result<AgentHandle> {
        let zellij_session = ctx
            .zellij_session
            .clone()
            .ok_or_else(|| {
                anyhow!(
                    "ZellijHostedRuntime::launch: agent {} has no zellij_session in \
                     LaunchContext. This means the daemon did not resolve \
                     AgentLaunchContext.zellij_session before calling launch — \
                     check that the agent has an `agent_launch_context` row in the registry.",
                    ctx.agent_id
                )
            })?;

        let session = ZellijSession::new(&zellij_session);
        if !session.is_alive() {
            // Not fatal — Phase 5 will own restart, and at startup the
            // session may not be up yet. Log loudly so operators see it.
            tracing::warn!(
                "ZellijHostedRuntime::launch: agent {} bound to Zellij session '{}', \
                 but the session is not currently running. signal() calls will fail \
                 until the session comes back up.",
                ctx.agent_id,
                zellij_session
            );
        } else {
            tracing::info!(
                "ZellijHostedRuntime::launch: agent {} attached to live Zellij session '{}'",
                ctx.agent_id,
                zellij_session
            );
        }

        self.sessions.lock().await.insert(
            ctx.agent_id.clone(),
            AgentSession {
                zellij_session,
                mutex: Arc::new(Mutex::new(())),
            },
        );

        Ok(AgentHandle {
            agent_id: ctx.agent_id,
            runtime_kind: RuntimeKind::ZellijHosted,
            pid: 0,
        })
    }

    /// Task-mode is not supported for ZellijHosted. Prompts go through
    /// `signal()` with `AgentSignal::UserInput`.
    async fn inject_task(
        &self,
        _handle: &AgentHandle,
        _task: &TaskSpec,
    ) -> Result<BoxStream<'static, ProgressEvent>> {
        bail!(
            "ZellijHostedRuntime is persistent-only; inject_task is not supported. \
             Use `mc agent signal` (AgentSignal::UserInput) to deliver a prompt."
        )
    }

    /// Deliver a signal to the agent's Zellij pane.
    ///
    /// - `UserInput { text }`        → paste text + 300ms + Enter
    /// - `PeerMessage { … }`         → format as `[from <id>]: <body>`, then same path
    /// - `Cancel`                    → send `Ctrl c` to interrupt whatever is running
    ///
    /// Per-agent mutex serialises concurrent calls so two signals can't
    /// interleave their paste/Enter pairs.
    async fn signal(&self, handle: &AgentHandle, signal: AgentSignal) -> Result<()> {
        // Resolve session + mutex without holding the outer map lock during the send.
        let (zellij_session, mutex) = {
            let map = self.sessions.lock().await;
            let agent = map.get(&handle.agent_id).ok_or_else(|| {
                anyhow!(
                    "ZellijHostedRuntime::signal: agent {} not registered. \
                     Was launch() called on this agent?",
                    handle.agent_id
                )
            })?;
            (agent.zellij_session.clone(), agent.mutex.clone())
        };
        let _guard = mutex.lock().await;

        let session = ZellijSession::new(&zellij_session);
        match signal {
            AgentSignal::UserInput { text } => session.send_prompt(&text),
            AgentSignal::PeerMessage {
                from_agent_id, body, ..
            } => {
                let body_text = match &body {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let formatted = format!("[from {from_agent_id}]: {body_text}");
                session.send_prompt(&formatted)
            }
            AgentSignal::Cancel => session.send_keys(
                crate::zellij_session::DEFAULT_PANE_ID,
                &["Ctrl c"],
            ),
        }
    }

    /// ZellijHosted is persistent — there's no per-task result. The
    /// stream-based `inject_task` is also unsupported, so this method is
    /// effectively dead code for this runtime, but the trait requires it.
    async fn collect_result(&self, handle: &AgentHandle) -> Result<TaskResult> {
        Ok(TaskResult {
            task_id: handle.agent_id.clone(),
            success: true,
            exit_code: 0,
            artifact_path: None,
            summary: "ZellijHosted has no task lifecycle".into(),
        })
    }

    /// Zellij is a pane multiplexer, not a PTY. Attach surfaces for
    /// ZellijHosted agents are handled outside this trait — see the
    /// module-level doc comment.
    async fn attach_pty(&self, _handle: &AgentHandle) -> Result<PtySession> {
        bail!(
            "ZellijHostedRuntime does not support PTY attach; \
             use `mc agent attach <id>` (exec zellij attach) or \
             `mc agent attach --web <id>` (return zellij web URL) instead"
        )
    }

    /// No-op — mcd does not own the Zellij session lifecycle. systemd +
    /// aria-watchdog (Phase 5: mcd's own supervisor) own start/stop.
    async fn shutdown(&self, handle: AgentHandle) -> Result<()> {
        tracing::debug!(
            "ZellijHostedRuntime::shutdown: removing agent {} from session map \
             (no process to terminate — Zellij session is externally managed)",
            handle.agent_id
        );
        self.sessions.lock().await.remove(&handle.agent_id);
        Ok(())
    }

    /// Verify `zellij` is available. Surfaces missing-binary at startup
    /// instead of at first signal.
    ///
    /// systemd `--user` services run with a stripped PATH that doesn't
    /// include `~/.cargo/bin` or `~/.local/bin` where users typically
    /// install zellij. Probe known candidate locations explicitly rather
    /// than relying on PATH or `which::which` (same pattern as the ACP
    /// runtime's claude probe).
    async fn ensure_installed(&self) -> Result<()> {
        for candidate in crate::zellij_session::zellij_candidates() {
            let probe = tokio::process::Command::new(&candidate)
                .arg("--version")
                .output()
                .await;
            if let Ok(out) = probe {
                if out.status.success() {
                    tracing::debug!("ZellijHostedRuntime: found zellij at {}", candidate);
                    return Ok(());
                }
            }
        }
        bail!(
            "zellij binary not found in any of the candidate locations \
             (PATH, ~/.cargo/bin, ~/.local/bin, /usr/local/bin, /usr/bin). \
             Install with `cargo install zellij` or your package manager."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_zellij_hosted() {
        let rt = ZellijHostedRuntime::new();
        assert_eq!(rt.kind(), RuntimeKind::ZellijHosted);
    }

    #[test]
    fn capabilities_include_zellij_hosted_marker() {
        let rt = ZellijHostedRuntime::new();
        let names: Vec<&str> = rt.capabilities().iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"zellij_hosted"));
    }

    #[tokio::test]
    async fn launch_without_zellij_session_bails() {
        let rt = ZellijHostedRuntime::new();
        let ctx = LaunchContext {
            agent_id: "test".into(),
            ..Default::default()
        };
        match rt.launch(ctx).await {
            Ok(_) => panic!("launch should bail when zellij_session is None"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("no zellij_session"), "msg: {msg}");
                assert!(msg.contains("agent_launch_context"), "msg: {msg}");
            }
        }
    }

    #[tokio::test]
    async fn signal_before_launch_bails() {
        let rt = ZellijHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::ZellijHosted,
            pid: 0,
        };
        let err = rt
            .signal(&handle, AgentSignal::UserInput { text: "hi".into() })
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not registered"), "msg: {msg}");
    }

    #[tokio::test]
    async fn inject_task_bails_with_routing_hint() {
        let rt = ZellijHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::ZellijHosted,
            pid: 0,
        };
        let task = TaskSpec {
            id: "t".into(),
            kluster_id: "".into(),
            mission_id: "".into(),
            title: "".into(),
            description: "".into(),
            input_json: "{}".into(),
            required_capabilities: vec![],
            produces: serde_json::Value::Null,
            consumes: serde_json::Value::Null,
            agent_profile: None,
            mission_roster: vec![],
            dependency_results: vec![],
            pending_messages: vec![],
        };
        match rt.inject_task(&handle, &task).await {
            Ok(_) => panic!("inject_task should bail"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("persistent-only"), "msg: {msg}");
                assert!(msg.contains("UserInput"), "msg: {msg}");
            }
        }
    }

    #[tokio::test]
    async fn attach_pty_bails_with_routing_hint() {
        let rt = ZellijHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::ZellijHosted,
            pid: 0,
        };
        match rt.attach_pty(&handle).await {
            Ok(_) => panic!("attach_pty should bail"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("mc agent attach"), "msg: {msg}");
            }
        }
    }

    #[tokio::test]
    async fn shutdown_clears_session_entry() {
        let rt = ZellijHostedRuntime::new();
        // Seed an entry directly to avoid the is_alive subprocess that
        // launch() would make.
        rt.sessions.lock().await.insert(
            "test".into(),
            AgentSession {
                zellij_session: "test-session".into(),
                mutex: Arc::new(Mutex::new(())),
            },
        );
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::ZellijHosted,
            pid: 0,
        };
        rt.shutdown(handle).await.unwrap();
        assert!(!rt.sessions.lock().await.contains_key("test"));
    }
}
