//! `AgentRuntime` for agents hosted in a Herdr session.
//!
//! Mirrors zellij_hosted.rs's "thin facade" pattern exactly: this runtime
//! owns no agent process — the Herdr session is externally managed by
//! systemd (see the `aria` repo's `integrations/herdr/aria-<profile>.service`
//! units). This runtime only knows how to poke at an already-running pane.
//!
//! Unlike ZellijHostedRuntime, there is no plugin-routing path here.
//! Zellij's WASM plugin control channel exists specifically to get an
//! atomic-submit-with-completion-signal on top of a fundamentally blind
//! paste+Enter primitive. `herdr agent prompt --wait` already IS that —
//! there is nothing to route around.
//!
//! ## Attach surfaces
//!
//! `attach_pty` bails — handled outside the trait by `edgeplane agent
//! attach`, backed by `herdr_bridge.rs` (mirrors zellij_bridge.rs).
//!
//! ## Task-mode
//!
//! `inject_task` bails — HerdrHosted is persistent-only, same as ZellijHosted.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use edgeplaned_core::agent_runtime::AgentRuntime;
use edgeplaned_core::progress::ProgressEvent;
use edgeplaned_core::types::{
    AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult,
    TaskSpec,
};
use futures::stream::BoxStream;
use tokio::sync::Mutex;

use crate::herdr_session::HerdrSession;
use crate::shared::merge_capabilities;

/// Per-agent state cached at `launch()` and read at `signal()`.
struct AgentSession {
    herdr_session: String,
    mutex: Arc<Mutex<()>>,
}

pub struct HerdrHostedRuntime {
    capabilities: Vec<Capability>,
    version: String,
    sessions: Arc<Mutex<HashMap<String, AgentSession>>>,
}

impl HerdrHostedRuntime {
    pub fn new() -> Self {
        Self::with_extra_capabilities(Vec::new())
    }

    pub fn with_extra_capabilities(extra: Vec<Capability>) -> Self {
        let builtins = vec![
            Capability::new("herdr_hosted"),
            Capability::new("interactive_attach"),
            Capability::new("send_keys"),
        ];
        Self {
            capabilities: merge_capabilities(builtins, extra),
            version: "herdr_hosted 0.1 (Phase E)".into(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for HerdrHostedRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentRuntime for HerdrHostedRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::HerdrHosted
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// Verify the externally-managed Herdr session is reachable and
    /// register the agent in the per-agent session map. Does NOT spawn
    /// anything — lifecycle is owned by systemd.
    async fn launch(&self, ctx: LaunchContext) -> Result<AgentHandle> {
        let herdr_session = ctx.herdr_session.clone().ok_or_else(|| {
            anyhow!(
                "HerdrHostedRuntime::launch: agent {} has no herdr_session in \
                 LaunchContext. This means the daemon did not resolve \
                 AgentLaunchContext.herdr_session before calling launch — \
                 check that the agent has an `agent_launch_context` row in the registry.",
                ctx.agent_id
            )
        })?;

        let session = HerdrSession::new(&herdr_session);
        if !session.is_alive() {
            tracing::warn!(
                "HerdrHostedRuntime::launch: agent {} bound to Herdr session '{}', \
                 but the session is not currently running. signal() calls will fail \
                 until the session comes back up.",
                ctx.agent_id,
                herdr_session
            );
        } else {
            tracing::info!(
                "HerdrHostedRuntime::launch: agent {} attached to live Herdr session '{}'",
                ctx.agent_id,
                herdr_session
            );
        }

        self.sessions.lock().await.insert(
            ctx.agent_id.clone(),
            AgentSession {
                herdr_session,
                mutex: Arc::new(Mutex::new(())),
            },
        );

        Ok(AgentHandle {
            agent_id: ctx.agent_id,
            runtime_kind: RuntimeKind::HerdrHosted,
            pid: 0,
        })
    }

    async fn inject_task(
        &self,
        _handle: &AgentHandle,
        _task: &TaskSpec,
    ) -> Result<BoxStream<'static, ProgressEvent>> {
        bail!(
            "HerdrHostedRuntime is persistent-only; inject_task is not supported. \
             Use `edgeplane agent signal` (AgentSignal::UserInput) to deliver a prompt."
        )
    }

    /// Deliver a signal to the agent's Herdr pane. Per-agent mutex
    /// serialises concurrent calls so two signals can't interleave.
    async fn signal(&self, handle: &AgentHandle, signal: AgentSignal) -> Result<()> {
        let (herdr_session, mutex) = {
            let map = self.sessions.lock().await;
            let agent = map.get(&handle.agent_id).ok_or_else(|| {
                anyhow!(
                    "HerdrHostedRuntime::signal: agent {} not registered. \
                     Was launch() called on this agent?",
                    handle.agent_id
                )
            })?;
            (agent.herdr_session.clone(), agent.mutex.clone())
        };
        let _guard = mutex.lock().await;

        let session = HerdrSession::new(&herdr_session);
        let pane_id = session.discover_pane_id();

        match signal {
            AgentSignal::UserInput { text } => session.send_prompt(&pane_id, &text),
            AgentSignal::PeerMessage {
                from_agent_id,
                body,
                ..
            } => {
                let body_text = match &body {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let formatted = format!("[from {from_agent_id}]: {body_text}");
                session.send_prompt(&pane_id, &formatted)
            }
            AgentSignal::Cancel => session.send_keys(&pane_id, &["ctrl+c"]),
        }
    }

    async fn collect_result(&self, handle: &AgentHandle) -> Result<TaskResult> {
        Ok(TaskResult {
            task_id: handle.agent_id.clone(),
            success: true,
            exit_code: 0,
            artifact_path: None,
            summary: "HerdrHosted has no task lifecycle".into(),
        })
    }

    async fn attach_pty(&self, _handle: &AgentHandle) -> Result<PtySession> {
        bail!(
            "HerdrHostedRuntime does not support PTY attach via this trait method; \
             use `edgeplane agent attach <id>` (execs `herdr session attach`) instead"
        )
    }

    async fn shutdown(&self, handle: AgentHandle) -> Result<()> {
        tracing::debug!(
            "HerdrHostedRuntime::shutdown: removing agent {} from session map \
             (no process to terminate — Herdr session is externally managed)",
            handle.agent_id
        );
        self.sessions.lock().await.remove(&handle.agent_id);
        Ok(())
    }

    async fn ensure_installed(&self) -> Result<()> {
        for candidate in crate::herdr_session::herdr_candidates() {
            let probe = tokio::process::Command::new(&candidate)
                .arg("--version")
                .output()
                .await;
            if let Ok(out) = probe
                && out.status.success()
            {
                tracing::debug!("HerdrHostedRuntime: found herdr at {}", candidate);
                return Ok(());
            }
        }
        bail!(
            "herdr binary not found in any of the candidate locations \
             (PATH, ~/.cargo/bin, ~/.local/bin, /usr/local/bin, /usr/bin)."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_herdr_hosted() {
        let rt = HerdrHostedRuntime::new();
        assert_eq!(rt.kind(), RuntimeKind::HerdrHosted);
    }

    #[test]
    fn capabilities_include_herdr_hosted_marker() {
        let rt = HerdrHostedRuntime::new();
        let names: Vec<&str> = rt.capabilities().iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"herdr_hosted"));
    }

    #[tokio::test]
    async fn launch_without_herdr_session_bails() {
        let rt = HerdrHostedRuntime::new();
        let ctx = LaunchContext {
            agent_id: "test".into(),
            ..Default::default()
        };
        match rt.launch(ctx).await {
            Ok(_) => panic!("launch should bail when herdr_session is None"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("no herdr_session"), "msg: {msg}");
                assert!(msg.contains("agent_launch_context"), "msg: {msg}");
            }
        }
    }

    #[tokio::test]
    async fn signal_before_launch_bails() {
        let rt = HerdrHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::HerdrHosted,
            pid: 0,
        };
        let err = rt
            .signal(&handle, AgentSignal::UserInput { text: "hi".into() })
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("not registered"));
    }

    #[tokio::test]
    async fn inject_task_bails_with_routing_hint() {
        let rt = HerdrHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::HerdrHosted,
            pid: 0,
        };
        let task = TaskSpec {
            id: "t".into(),
            mission_id: "".into(),
            domain_id: "".into(),
            title: "".into(),
            description: "".into(),
            input_json: "{}".into(),
            required_capabilities: vec![],
            produces: serde_json::Value::Null,
            consumes: serde_json::Value::Null,
            agent_profile: None,
            domain_roster: vec![],
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
        let rt = HerdrHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::HerdrHosted,
            pid: 0,
        };
        match rt.attach_pty(&handle).await {
            Ok(_) => panic!("attach_pty should bail"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("edgeplane agent attach"), "msg: {msg}");
            }
        }
    }

    #[tokio::test]
    async fn shutdown_clears_session_entry() {
        let rt = HerdrHostedRuntime::new();
        rt.sessions.lock().await.insert(
            "test".into(),
            AgentSession {
                herdr_session: "test-session".into(),
                mutex: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            },
        );
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::HerdrHosted,
            pid: 0,
        };
        rt.shutdown(handle).await.unwrap();
        assert!(!rt.sessions.lock().await.contains_key("test"));
    }
}
