//! `AgentRuntime` for agents hosted in a Zellij pane.
//!
//! Phase 1 of the daemon-absorption plan — **stub only**. The struct exists,
//! `kind()` / `version()` / `capabilities()` / `ensure_installed()` are real,
//! but every method that would actually drive the agent (`launch`,
//! `inject_task`, `signal`, `collect_result`, `shutdown`) bails with a
//! "not yet implemented (Phase 2)" error.
//!
//! Phase 2 will fill these in by porting `aria-rs/src/fleet/mod.rs`'s
//! subprocess wrappers around `zellij action` (paste + send-keys for
//! `signal`, dump-screen + classify_state for idle detection, etc.).
//!
//! `attach_pty` returns `Err` permanently — Zellij is a pane multiplexer,
//! not a PTY. Attach surfaces for ZellijHosted agents are:
//! - CLI: `mc agent attach <id>` → `zellij attach <session>` (handled
//!   outside the runtime trait by the attach gateway)
//! - Web: `mc agent attach --web <id>` → `http://127.0.0.1:8082/<session>`
//!   (also handled outside the trait, by resolving
//!   `AgentLaunchContext.zellij_session` and printing the URL)

use anyhow::{Result, bail};
use async_trait::async_trait;
use futures::stream::BoxStream;
use mcd_core::agent_runtime::AgentRuntime;
use mcd_core::progress::ProgressEvent;
use mcd_core::types::{
    AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult,
    TaskSpec,
};

use crate::shared::merge_capabilities;

pub struct ZellijHostedRuntime {
    capabilities: Vec<Capability>,
    version: String,
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
            version: "zellij_hosted (Phase 1 stub — not yet operational)".into(),
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

    async fn launch(&self, _ctx: LaunchContext) -> Result<AgentHandle> {
        bail!(
            "ZellijHostedRuntime: launch() not yet implemented (Phase 2). \
             Agent record was imported by Phase 1 but the runtime that drives \
             it lands in the next phase of the daemon-absorption plan."
        )
    }

    async fn inject_task(
        &self,
        _handle: &AgentHandle,
        _task: &TaskSpec,
    ) -> Result<BoxStream<'static, ProgressEvent>> {
        bail!("ZellijHostedRuntime: inject_task() not yet implemented (Phase 2)")
    }

    async fn signal(&self, _handle: &AgentHandle, _signal: AgentSignal) -> Result<()> {
        bail!("ZellijHostedRuntime: signal() not yet implemented (Phase 2)")
    }

    async fn collect_result(&self, _handle: &AgentHandle) -> Result<TaskResult> {
        bail!("ZellijHostedRuntime: collect_result() not yet implemented (Phase 2)")
    }

    async fn attach_pty(&self, _handle: &AgentHandle) -> Result<PtySession> {
        // Zellij is a pane multiplexer, not a PTY. Attach surfaces for
        // ZellijHosted agents are handled outside this trait — see the
        // module-level doc comment for the routing.
        bail!(
            "ZellijHostedRuntime does not support PTY attach; \
             use `mc agent attach <id>` (exec zellij attach) or \
             `mc agent attach --web <id>` (return zellij web URL) instead"
        )
    }

    async fn shutdown(&self, _handle: AgentHandle) -> Result<()> {
        bail!("ZellijHostedRuntime: shutdown() not yet implemented (Phase 2)")
    }

    async fn ensure_installed(&self) -> Result<()> {
        // Real check — even the stub needs zellij on PATH so Phase 2 doesn't
        // surprise us later with a missing-binary error in production.
        let present = tokio::process::Command::new("zellij")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !present {
            bail!(
                "zellij is not on PATH. Install with `cargo install zellij` \
                 or your package manager. Required for the ZellijHosted \
                 runtime once Phase 2 lands."
            );
        }
        Ok(())
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
    fn version_string_signals_stub_status() {
        let rt = ZellijHostedRuntime::new();
        assert!(rt.version().contains("Phase 1"));
        assert!(rt.version().contains("stub"));
    }

    #[test]
    fn capabilities_include_zellij_hosted_marker() {
        let rt = ZellijHostedRuntime::new();
        let names: Vec<&str> = rt.capabilities().iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"zellij_hosted"));
    }

    #[tokio::test]
    async fn launch_bails_with_phase2_message() {
        let rt = ZellijHostedRuntime::new();
        let err = rt.launch(LaunchContext::default()).await.unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Phase 2"), "msg: {msg}");
    }

    #[tokio::test]
    async fn attach_pty_bails_with_routing_hint() {
        let rt = ZellijHostedRuntime::new();
        let handle = AgentHandle {
            agent_id: "test".into(),
            runtime_kind: RuntimeKind::ZellijHosted,
            pid: 0,
        };
        // PtySession doesn't impl Debug, so .unwrap_err() would fail to
        // compile — match the result instead.
        match rt.attach_pty(&handle).await {
            Ok(_) => panic!("attach_pty should have bailed"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(msg.contains("mc agent attach"), "msg: {msg}");
            }
        }
    }
}
