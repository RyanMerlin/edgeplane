//! `AgentRuntime` for `claude-agent-acp` over the Agent Client Protocol.
//!
//! Drives the Node-based [`@agentclientprotocol/claude-agent-acp`] (formerly
//! `@zed-industries/claude-code-acp`) over JSON-RPC/stdio via the pure-Rust
//! [`edgeplaned_acp`] client. Every `inject_task` opens a fresh ACP session,
//! sends one prompt, streams `session/update` notifications back as typed
//! [`ProgressEvent`]s, and shuts the session down — task-mode parity with the
//! existing CLI runtimes.
//!
//! The reusable building block at the bottom of this module — [`AcpSession`]
//! — is also what the persistent-session supervisor (Layer 3) will hold open
//! across many prompts.
//!
//! [`@agentclientprotocol/claude-agent-acp`]: https://www.npmjs.com/package/@agentclientprotocol/claude-agent-acp

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use edgeplaned_acp::{
    Agent, ContentBlock, SessionUpdate, SpawnOpts, consts::PROTOCOL_VERSION, schema,
};
use edgeplaned_core::agent_runtime::AgentRuntime;
use edgeplaned_core::paths;
use edgeplaned_core::progress::{ProgressEvent, ProgressEventType};
use edgeplaned_core::types::{
    AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult,
    TaskSpec,
};
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::broadcast::error::RecvError;

use crate::shared::{build_prompt, merge_capabilities};

/// npm package name of the agent. The Zed-namespaced alias still resolves but
/// the canonical name has been the `@agentclientprotocol` one since early 2026.
const AGENT_NPM_PKG: &str = "@agentclientprotocol/claude-agent-acp";
/// Override: full path to the agent's `dist/index.js`. Bypasses the search.
const ENV_ACP_JS: &str = "EP_MESH_ACP_JS";

pub struct ClaudeAgentAcpRuntime {
    capabilities: Vec<Capability>,
    version: String,
    /// Resolved node path. `None` if `node` was missing at construction —
    /// `ensure_installed` retries.
    node_path: OnceLock<PathBuf>,
    /// Resolved `dist/index.js` path. Populated by `ensure_installed`.
    acp_js: OnceLock<PathBuf>,
    install_done: OnceLock<()>,
}

impl ClaudeAgentAcpRuntime {
    pub fn new() -> Self {
        Self::with_extra_capabilities(Vec::new())
    }

    /// Build a runtime with the built-in capabilities plus any extras from
    /// per-agent config.
    pub fn with_extra_capabilities(extra: Vec<Capability>) -> Self {
        let builtins = vec![
            Capability::new("claude_agent_acp"),
            Capability::new("acp"),
            Capability::new("code.read"),
            Capability::new("code.edit"),
            Capability::new("code.plan"),
            Capability::new("test.run"),
        ];
        let node_path = OnceLock::new();
        if let Ok(p) = which::which("node") {
            let _ = node_path.set(p);
        }
        Self {
            capabilities: merge_capabilities(builtins, extra),
            version: detect_version(),
            node_path,
            acp_js: OnceLock::new(),
            install_done: OnceLock::new(),
        }
    }
}

impl Default for ClaudeAgentAcpRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn detect_version() -> String {
    // Best-effort: the agent itself reports its version in the InitializeResponse.
    // Without spawning, just report the runtime kind.
    "claude_agent_acp (resolved at first launch)".into()
}

// ── AgentRuntime impl ────────────────────────────────────────────────────────

#[async_trait]
impl AgentRuntime for ClaudeAgentAcpRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::ClaudeAgentAcp
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn launch(&self, ctx: LaunchContext) -> Result<AgentHandle> {
        std::fs::create_dir_all(&ctx.work_dir)?;
        // ACP sessions are spawned per-task in task-mode; `launch` just
        // verifies prerequisites and prepares the work dir.
        self.ensure_installed().await?;
        tracing::info!(
            "claude_agent_acp agent {} ready in {}",
            ctx.agent_id,
            ctx.work_dir.display()
        );
        Ok(AgentHandle {
            agent_id: ctx.agent_id,
            runtime_kind: RuntimeKind::ClaudeAgentAcp,
            pid: 0,
        })
    }

    async fn inject_task(
        &self,
        handle: &AgentHandle,
        task: &TaskSpec,
    ) -> Result<BoxStream<'static, ProgressEvent>> {
        let prompt = build_prompt(task);
        let task_id = task.id.clone();
        let agent_id = handle.agent_id.clone();
        let work_dir = paths::mcd_work_dir().join(&agent_id);
        std::fs::create_dir_all(&work_dir)?;

        let opts = self.spawn_opts(&work_dir)?;

        tracing::info!(
            "claude_agent_acp injecting task {task_id}: {}",
            &prompt[..prompt.len().min(80)]
        );

        // Task-mode sessions are ephemeral and not inspected interactively;
        // no remote-control prefix needed.
        let session = AcpSession::open(opts, work_dir, None).await?;

        // Convert prompt + session into a ProgressEvent stream that closes
        // the session when the stream completes.
        let stream = async_stream::stream! {
            yield ProgressEvent::phase_started("running", "claude_agent_acp session opened");
            let mut events = session.prompt(prompt);
            while let Some(ev) = events.next().await {
                yield ev;
            }
            drop(events); // release the borrow before shutdown takes ownership
            if let Err(e) = session.shutdown().await {
                yield ProgressEvent::error(
                    format!("acp shutdown: {e}"),
                    serde_json::json!({ "detail": e.to_string() }),
                );
            }
        };

        Ok(Box::pin(stream))
    }

    async fn signal(&self, handle: &AgentHandle, signal: AgentSignal) -> Result<()> {
        // Task-mode has no long-running session to signal between tasks; the
        // persistent-session supervisor (Layer 3) handles cancellation
        // directly via AcpSession::cancel.
        tracing::info!(
            "Signal to claude_agent_acp agent {}: {:?} (no-op in task-mode)",
            handle.agent_id,
            signal
        );
        Ok(())
    }

    async fn collect_result(&self, handle: &AgentHandle) -> Result<TaskResult> {
        Ok(TaskResult {
            task_id: handle.agent_id.clone(),
            success: true,
            exit_code: 0,
            artifact_path: None,
            summary: "claude_agent_acp task finished".into(),
        })
    }

    async fn attach_pty(&self, _handle: &AgentHandle) -> Result<PtySession> {
        // ACP is not a byte-stream protocol; PTY attach makes no sense here.
        // The attach surface for ACP agents is the controlplane WS proxy that
        // relays JSON-RPC messages — see attach_ws.rs (frame protocol will be
        // text/JSON, not binary, when we wire ACP fully).
        bail!(
            "claude_agent_acp does not support PTY attach; \
             use the ACP attach path (controlplane WS proxy) instead"
        )
    }

    async fn shutdown(&self, handle: AgentHandle) -> Result<()> {
        // Each task already shut down its own session in inject_task's stream
        // close. Nothing to do here in task-mode.
        tracing::info!(
            "claude_agent_acp agent {} shutdown (no-op)",
            handle.agent_id
        );
        Ok(())
    }

    async fn ensure_installed(&self) -> Result<()> {
        if self.install_done.get().is_some() {
            return Ok(());
        }

        // Step 1: node must be available.
        let node = self
            .node_path
            .get()
            .cloned()
            .or_else(|| which::which("node").ok())
            .ok_or_else(|| {
                anyhow!(
                    "node binary not found in PATH. \
                     Install Node.js (https://nodejs.org) then re-run the daemon."
                )
            })?;
        let _ = self.node_path.set(node.clone());

        // Step 2: locate dist/index.js. Try env override first, then npm
        // global root, then a couple of common manual install spots.
        if let Some(p) = locate_acp_js().await {
            let _ = self.acp_js.set(p);
            let _ = self.install_done.set(());
            return Ok(());
        }

        // Step 3: not installed — install globally via npm.
        tracing::info!("{AGENT_NPM_PKG} not found — installing via npm…");
        let out = tokio::process::Command::new("npm")
            .args(["install", "-g", AGENT_NPM_PKG])
            .output()
            .await
            .map_err(|e| anyhow!("npm install -g {AGENT_NPM_PKG} failed to launch: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(anyhow!(
                "npm install -g {AGENT_NPM_PKG} failed (exit {}): {stderr}",
                out.status
            ));
        }

        // Step 4: re-locate after install.
        let p = locate_acp_js().await.ok_or_else(|| {
            anyhow!("npm install succeeded but {AGENT_NPM_PKG} dist/index.js still not found")
        })?;
        let _ = self.acp_js.set(p);
        let _ = self.install_done.set(());
        Ok(())
    }
}

impl ClaudeAgentAcpRuntime {
    /// Build [`SpawnOpts`] with resolved `node` + `dist/index.js` paths and
    /// the given working directory. `ensure_installed` must have been called
    /// successfully first; otherwise this errors.
    pub fn spawn_opts(&self, cwd: &std::path::Path) -> Result<SpawnOpts> {
        let node = self
            .node_path
            .get()
            .cloned()
            .ok_or_else(|| anyhow!("node not resolved — call ensure_installed first"))?;
        let acp_js = self.acp_js.get().cloned().ok_or_else(|| {
            anyhow!("acp dist/index.js not resolved — call ensure_installed first")
        })?;
        let mut opts = SpawnOpts::claude_code_acp(node, acp_js);
        opts.cwd = Some(cwd.to_path_buf());
        // Prefer the system claude CLI over the binary bundled in the ACP npm
        // package — the bundled binary lags behind the system install and may
        // be missing remote-control support or recent bug fixes.
        // Probe candidates in order; systemd services run with a stripped PATH
        // so which::which() is unreliable — check known locations explicitly.
        // EP_ACP_CLAUDE_EXECUTABLE overrides all of this for testing.
        let system_claude = std::env::var("EP_ACP_CLAUDE_EXECUTABLE").ok().or_else(|| {
            let candidates = [
                // versioned symlink written by the claude CLI updater
                dirs::home_dir()
                    .map(|h| h.join(".local/share/claude/versions"))
                    .and_then(|base| {
                        // pick the highest version directory
                        std::fs::read_dir(&base).ok().and_then(|rd| {
                            let mut entries: Vec<_> = rd
                                .flatten()
                                .filter(|e| e.path().join("claude").exists())
                                .collect();
                            entries.sort_by_key(|e| e.file_name());
                            entries.last().map(|e| e.path().join("claude"))
                        })
                    }),
                // standard user-local install
                dirs::home_dir().map(|h| h.join(".local/bin/claude")),
                // cargo-installed (some dev setups)
                dirs::home_dir().map(|h| h.join(".cargo/bin/claude")),
            ];
            candidates
                .into_iter()
                .flatten()
                .find(|p| p.exists())
                .map(|p| p.to_string_lossy().into_owned())
        });
        if let Some(exe) = system_claude {
            tracing::debug!("ACP sessions will use claude executable: {exe}");
            opts.env.insert("CLAUDE_CODE_EXECUTABLE".into(), exe);
        }
        Ok(opts)
    }
}

// ── AcpSession (reusable) ────────────────────────────────────────────────────

/// One ACP session — a spawned agent process plus a created session id.
///
/// Used by the AgentRuntime impl above for task-mode (open → prompt → close)
/// and by the persistent-session supervisor (held open across many prompts).
pub struct AcpSession {
    agent: Agent,
    session_id: schema::SessionId,
}

impl AcpSession {
    /// Spawn the agent, run the initialize handshake, and create a session
    /// rooted at `cwd`. Returns once the session id is in hand and the agent
    /// is ready to accept prompts.
    /// `remote_control_prefix` — when `Some`, injects `--remote-control` and
    /// `--remote-control-session-name-prefix <prefix>` into the claude process
    /// via `_meta.claudeCode.options.extraArgs` in `session/new`. The session
    /// then appears in the Claude app under that prefix, making it addressable
    /// for interactive inspection of fleet ACP sessions.
    pub async fn open(
        opts: SpawnOpts,
        cwd: PathBuf,
        remote_control_prefix: Option<String>,
    ) -> Result<Self> {
        let agent = Agent::spawn(opts).await.context("acp spawn")?;

        let init = tokio::time::timeout(
            Duration::from_secs(15),
            agent.initialize(schema::InitializeRequest {
                meta: None,
                protocol_version: schema::ProtocolVersion(PROTOCOL_VERSION as u16),
                client_info: Some(schema::Implementation {
                    meta: None,
                    name: "edgeplaned".into(),
                    title: None,
                    version: env!("CARGO_PKG_VERSION").into(),
                }),
                client_capabilities: schema::ClientCapabilities::default(),
            }),
        )
        .await
        .map_err(|_| anyhow!("acp initialize timed out"))??;
        tracing::debug!(
            "acp initialize ok: protocol_version={} agent={:?}",
            init.protocol_version.0,
            init.agent_info.as_ref().map(|i| &i.name)
        );

        // Build _meta.claudeCode.options.extraArgs when a remote-control prefix
        // is supplied. The ACP node package reads this path and passes the
        // key/value pairs as CLI flags to the claude binary it spawns.
        let session_meta = remote_control_prefix.map(|prefix| {
            let mut m = serde_json::Map::new();
            m.insert(
                "claudeCode".to_string(),
                serde_json::json!({
                    "options": {
                        "extraArgs": {
                            "remote-control-session-name-prefix": prefix
                        }
                    }
                }),
            );
            m
        });

        let new_session = tokio::time::timeout(
            Duration::from_secs(60),
            agent.new_session(schema::NewSessionRequest {
                meta: session_meta,
                cwd: cwd.to_string_lossy().into_owned(),
                mcp_servers: vec![],
            }),
        )
        .await
        .map_err(|_| anyhow!("acp session/new timed out"))??;

        Ok(Self {
            agent,
            session_id: new_session.session_id,
        })
    }

    /// Send a prompt and return a stream of [`ProgressEvent`]s.
    ///
    /// The stream emits events as `session/update` notifications arrive,
    /// terminates with a final event derived from the prompt's stop reason,
    /// then closes. The session itself stays open after the stream closes;
    /// drop or call [`AcpSession::shutdown`] to terminate it.
    pub fn prompt(&self, prompt_text: String) -> BoxStream<'static, ProgressEvent> {
        let agent = self.agent.clone();
        let session_id = self.session_id.clone();
        let mut updates = agent.subscribe_session_updates();

        let stream = async_stream::stream! {
            let prompt_fut = agent.prompt(schema::PromptRequest {
                meta: None,
                session_id,
                prompt: vec![ContentBlock::text(prompt_text)],
            });
            tokio::pin!(prompt_fut);

            loop {
                tokio::select! {
                    biased;
                    res = &mut prompt_fut => {
                        match res {
                            Ok(resp) => {
                                yield finalize_event(&resp.stop_reason);
                                break;
                            }
                            Err(e) => {
                                yield ProgressEvent::error(
                                    format!("acp prompt rpc: {e}"),
                                    serde_json::json!({ "detail": e.to_string() }),
                                );
                                break;
                            }
                        }
                    }
                    recv = updates.recv() => {
                        match recv {
                            Ok(notif) => {
                                if let Some(ev) = update_to_progress(&notif.update) {
                                    yield ev;
                                }
                            }
                            Err(RecvError::Lagged(n)) => {
                                tracing::warn!("acp viewer lagged {n} updates; continuing");
                            }
                            Err(RecvError::Closed) => {
                                // Channel closed before the prompt resolved —
                                // wait for prompt_fut to surface the actual
                                // error or success.
                            }
                        }
                    }
                }
            }
        };

        Box::pin(stream)
    }

    /// Subscribe to raw `session/update` notifications. Used by the
    /// persistent-session supervisor to fan agent output out to viewers
    /// over the attach registry.
    pub fn subscribe_updates(
        &self,
    ) -> tokio::sync::broadcast::Receiver<edgeplaned_acp::wire::SessionNotification> {
        self.agent.subscribe_session_updates()
    }

    /// Cancel any in-flight prompt for this session. Fire-and-forget per the
    /// ACP spec (cancellation is a notification).
    pub async fn cancel(&self) -> Result<()> {
        self.agent.cancel(self.session_id.clone()).await?;
        Ok(())
    }

    /// Graceful shutdown — closes stdin, waits, then SIGKILL. Returns the
    /// child exit code.
    pub async fn shutdown(self) -> Result<i32> {
        let code = self.agent.shutdown().await?;
        Ok(code)
    }
}

// ── translation helpers ──────────────────────────────────────────────────────

fn update_to_progress(update: &SessionUpdate) -> Option<ProgressEvent> {
    match update {
        SessionUpdate::AgentMessageChunk { content } => {
            let text = content.as_text()?;
            if text.trim().is_empty() {
                return None;
            }
            Some(ProgressEvent {
                event_type: ProgressEventType::StepStarted,
                phase: Some("running".into()),
                step: Some("responding".into()),
                summary: truncate(text, 200),
                payload: serde_json::json!({ "text": text }),
            })
        }
        SessionUpdate::AgentThoughtChunk { content } => {
            let text = content.as_text()?;
            if text.trim().is_empty() {
                return None;
            }
            Some(ProgressEvent {
                event_type: ProgressEventType::StepStarted,
                phase: Some("running".into()),
                step: Some("thinking".into()),
                summary: truncate(text, 200),
                payload: serde_json::json!({ "text": text }),
            })
        }
        SessionUpdate::ToolCall(value) => {
            let title = value
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("tool");
            let id = value
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(ProgressEvent {
                event_type: ProgressEventType::StepStarted,
                phase: Some("running".into()),
                step: Some(format!("tool:{title}")),
                summary: format!("calling tool: {title}"),
                payload: serde_json::json!({ "tool_call_id": id, "raw": value }),
            })
        }
        SessionUpdate::ToolCallUpdate(value) => {
            // Surface only completion-status changes as events; everything
            // else is logged at debug.
            let status = value.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if matches!(status, "completed" | "failed" | "cancelled") {
                let title = value
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                Some(ProgressEvent {
                    event_type: ProgressEventType::StepStarted,
                    phase: Some("running".into()),
                    step: Some(format!("tool:{title}:{status}")),
                    summary: format!("tool {title} {status}"),
                    payload: value.clone(),
                })
            } else {
                tracing::debug!("acp tool_call_update ignored: status={status}");
                None
            }
        }
        SessionUpdate::Plan(value) => Some(ProgressEvent {
            event_type: ProgressEventType::Info,
            phase: Some("running".into()),
            step: Some("plan".into()),
            summary: "plan updated".into(),
            payload: value.clone(),
        }),
        // Metadata streams — log only.
        SessionUpdate::UserMessageChunk { .. }
        | SessionUpdate::AvailableCommandsUpdate(_)
        | SessionUpdate::CurrentModeUpdate(_)
        | SessionUpdate::ConfigOptionUpdate(_)
        | SessionUpdate::SessionInfoUpdate(_)
        | SessionUpdate::UsageUpdate(_) => None,
    }
}

fn finalize_event(stop: &schema::StopReason) -> ProgressEvent {
    match stop {
        schema::StopReason::EndTurn => ProgressEvent {
            event_type: ProgressEventType::PhaseFinished,
            phase: Some("running".into()),
            step: None,
            summary: "agent finished turn".into(),
            payload: serde_json::json!({ "stop_reason": "end_turn" }),
        },
        schema::StopReason::MaxTokens => ProgressEvent {
            event_type: ProgressEventType::PhaseFinished,
            phase: Some("running".into()),
            step: None,
            summary: "stopped: token limit reached".into(),
            payload: serde_json::json!({ "stop_reason": "max_tokens" }),
        },
        schema::StopReason::MaxTurnRequests => ProgressEvent {
            event_type: ProgressEventType::PhaseFinished,
            phase: Some("running".into()),
            step: None,
            summary: "stopped: max turn requests reached".into(),
            payload: serde_json::json!({ "stop_reason": "max_turn_requests" }),
        },
        schema::StopReason::Refusal => ProgressEvent::error(
            "agent refused the prompt",
            serde_json::json!({ "stop_reason": "refusal" }),
        ),
        schema::StopReason::Cancelled => ProgressEvent {
            event_type: ProgressEventType::PhaseFinished,
            phase: Some("running".into()),
            step: None,
            summary: "cancelled".into(),
            payload: serde_json::json!({ "stop_reason": "cancelled" }),
        },
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let boundary = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
    format!("{}…", &s[..boundary])
}

// ── dist/index.js resolution ─────────────────────────────────────────────────

/// Search for `<...>/dist/index.js` of the ACP agent. Order:
///
/// 1. `EP_MESH_ACP_JS` env var (full path).
/// 2. `npm root -g` joined with the canonical and legacy package paths.
/// 3. `~/.npm-global/lib/node_modules/...` for user-mode global installs.
async fn locate_acp_js() -> Option<PathBuf> {
    if let Ok(p) = std::env::var(ENV_ACP_JS) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }

    let candidates: Vec<PathBuf> = {
        let mut out = Vec::new();
        if let Some(root) = npm_global_root().await {
            out.push(root.join(format!("{AGENT_NPM_PKG}/dist/index.js")));
            out.push(root.join("@zed-industries/claude-code-acp/dist/index.js"));
        }
        if let Some(home) = dirs::home_dir() {
            let user_root = home.join(".npm-global/lib/node_modules");
            out.push(user_root.join(format!("{AGENT_NPM_PKG}/dist/index.js")));
            out.push(user_root.join("@zed-industries/claude-code-acp/dist/index.js"));
        }
        out
    };

    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

async fn npm_global_root() -> Option<PathBuf> {
    let out = tokio::process::Command::new("npm")
        .args(["root", "-g"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        return None;
    }
    Some(PathBuf::from(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_message_chunk_translates_to_step_started() {
        let update = SessionUpdate::AgentMessageChunk {
            content: ContentBlock::text("hello"),
        };
        let ev = update_to_progress(&update).unwrap();
        assert_eq!(ev.event_type, ProgressEventType::StepStarted);
        assert_eq!(ev.step.as_deref(), Some("responding"));
        assert!(ev.summary.contains("hello"));
    }

    #[test]
    fn agent_thought_chunk_uses_thinking_step() {
        let update = SessionUpdate::AgentThoughtChunk {
            content: ContentBlock::text("hmm"),
        };
        let ev = update_to_progress(&update).unwrap();
        assert_eq!(ev.step.as_deref(), Some("thinking"));
    }

    #[test]
    fn empty_chunk_yields_no_event() {
        let update = SessionUpdate::AgentMessageChunk {
            content: ContentBlock::text(""),
        };
        assert!(update_to_progress(&update).is_none());
    }

    #[test]
    fn tool_call_translates_with_title() {
        let update = SessionUpdate::ToolCall(serde_json::json!({
            "title": "read_file",
            "toolCallId": "tc-1",
            "status": "in_progress",
        }));
        let ev = update_to_progress(&update).unwrap();
        assert_eq!(ev.event_type, ProgressEventType::StepStarted);
        assert!(ev.step.as_deref().unwrap_or("").contains("read_file"));
    }

    #[test]
    fn tool_call_update_only_emits_on_terminal_status() {
        let in_progress = SessionUpdate::ToolCallUpdate(serde_json::json!({
            "status": "in_progress",
            "title": "x",
        }));
        assert!(update_to_progress(&in_progress).is_none());

        let completed = SessionUpdate::ToolCallUpdate(serde_json::json!({
            "status": "completed",
            "title": "x",
        }));
        assert!(update_to_progress(&completed).is_some());

        let failed = SessionUpdate::ToolCallUpdate(serde_json::json!({
            "status": "failed",
            "title": "x",
        }));
        assert!(update_to_progress(&failed).is_some());
    }

    #[test]
    fn metadata_updates_yield_no_event() {
        for u in [
            SessionUpdate::UserMessageChunk {
                content: ContentBlock::text("u"),
            },
            SessionUpdate::AvailableCommandsUpdate(serde_json::json!({})),
            SessionUpdate::CurrentModeUpdate(serde_json::json!({})),
            SessionUpdate::ConfigOptionUpdate(serde_json::json!({})),
            SessionUpdate::SessionInfoUpdate(serde_json::json!({})),
            SessionUpdate::UsageUpdate(serde_json::json!({})),
        ] {
            assert!(
                update_to_progress(&u).is_none(),
                "unexpected event for {u:?}"
            );
        }
    }

    #[test]
    fn stop_reason_end_turn_finalizes_phase() {
        let ev = finalize_event(&schema::StopReason::EndTurn);
        assert_eq!(ev.event_type, ProgressEventType::PhaseFinished);
    }

    #[test]
    fn stop_reason_refusal_emits_error() {
        let ev = finalize_event(&schema::StopReason::Refusal);
        assert_eq!(ev.event_type, ProgressEventType::Error);
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_long_string_with_ellipsis() {
        let r = truncate("abcdefghij", 5);
        assert!(r.starts_with("abcde"));
        assert!(r.contains('…'));
    }
}
