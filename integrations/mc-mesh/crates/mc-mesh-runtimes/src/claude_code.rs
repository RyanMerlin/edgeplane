/// AgentRuntime implementation for the Claude Code CLI (`claude -p`).
///
/// Uses `--output-format stream-json` for structured real-time output.
/// Each JSONL line is parsed into a typed `ProgressEvent`.
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::stream::BoxStream;
use mc_mesh_core::agent_runtime::AgentRuntime;
use mc_mesh_core::paths;
use mc_mesh_core::progress::{ProgressEvent, ProgressEventType};
use mc_mesh_core::types::{
    AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult,
    TaskSpec,
};
use std::io::{Read, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct ClaudeCodeRuntime {
    capabilities: Vec<Capability>,
    version: String,
    install_done: OnceLock<()>,
    // SAFETY: This runtime instance is used for exactly one agent session. `with_rtk`
    // is written once in `launch()` before `inject_task()` is ever called, so
    // the Relaxed ordering is sufficient.
    with_rtk: AtomicBool,
    /// Tracks whether RTK hooks have already been verified/installed for this runtime
    /// instance. Avoids running a subprocess on every task injection.
    rtk_hooks_done: OnceLock<()>,
}

impl ClaudeCodeRuntime {
    pub fn new() -> Self {
        ClaudeCodeRuntime {
            capabilities: vec![
                Capability::new("claude_code"),
                Capability::new("code.read"),
                Capability::new("code.edit"),
                Capability::new("code.plan"),
                Capability::new("test.run"),
            ],
            version: detect_version(),
            install_done: OnceLock::new(),
            with_rtk: AtomicBool::new(false),
            rtk_hooks_done: OnceLock::new(),
        }
    }

    fn render_harness(&self) -> Result<()> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow!("cannot determine HOME directory"))?;
        let target = home.join(".claude").join("CLAUDE.md");
        crate::harness::write_capabilities_block(&target)
            .with_context(|| format!("rendering claude harness to {}", target.display()))?;
        tracing::info!("claude harness rendered to {}", target.display());
        Ok(())
    }
}

impl Default for ClaudeCodeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn detect_version() -> String {
    std::process::Command::new("claude")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// Build the prompt to send to claude from a TaskSpec.
/// Uses the shared builder which injects agent profile and mission roster.
fn build_prompt(task: &TaskSpec) -> String {
    crate::shared::build_prompt(task)
}

/// Parse a single stream-json line into zero or more ProgressEvents.
///
/// claude `--output-format stream-json` emits JSONL with these main types:
///   {"type":"system","subtype":"init",...}  — session init
///   {"type":"assistant","message":{...},...} — assistant turn
///   {"type":"result","subtype":"success"|"error","result":"..."}
fn parse_stream_line(line: &str) -> Vec<ProgressEvent> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        // Not JSON (e.g. debug output); emit as info.
        if !line.trim().is_empty() {
            return vec![ProgressEvent::info(line.trim().to_string())];
        }
        return vec![];
    };

    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match kind {
        "system" => {
            // Session init — emit phase_started
            vec![ProgressEvent::phase_started("running", "claude session started")]
        }
        "assistant" => {
            // Extract text content from the message.
            let mut events = vec![];
            if let Some(content) = v
                .pointer("/message/content")
                .and_then(|c| c.as_array())
            {
                for item in content {
                    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match item_type {
                        "text" => {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                if !text.trim().is_empty() {
                                    events.push(ProgressEvent {
                                        event_type: ProgressEventType::StepStarted,
                                        phase: Some("running".into()),
                                        step: Some("thinking".into()),
                                        summary: truncate(text, 200),
                                        payload: serde_json::json!({ "text": text }),
                                    });
                                }
                            }
                        }
                        "tool_use" => {
                            let tool_name = item
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("unknown");
                            events.push(ProgressEvent {
                                event_type: ProgressEventType::StepStarted,
                                phase: Some("running".into()),
                                step: Some(format!("tool:{tool_name}")),
                                summary: format!("calling tool: {tool_name}"),
                                payload: item.clone(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            events
        }
        "result" => {
            let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
            let result_text = v
                .get("result")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            if subtype == "error" {
                vec![ProgressEvent::error(
                    truncate(&result_text, 200),
                    serde_json::json!({ "detail": result_text }),
                )]
            } else {
                vec![
                    ProgressEvent {
                        event_type: ProgressEventType::PhaseFinished,
                        phase: Some("running".into()),
                        step: None,
                        summary: truncate(&result_text, 200),
                        payload: serde_json::json!({ "result": result_text }),
                    },
                ]
            }
        }
        _ => vec![],
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Slice at a codepoint boundary to avoid panicking on multi-byte chars.
    let boundary = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
    format!("{}…", &s[..boundary])
}

#[async_trait]
impl AgentRuntime for ClaudeCodeRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::ClaudeCode
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn launch(&self, ctx: LaunchContext) -> Result<AgentHandle> {
        // claude-code is stateless — each inject_task spawns a fresh `claude -p` process.
        // Launch just validates the binary exists and creates the work dir.
        std::fs::create_dir_all(&ctx.work_dir)?;

        // Capture rtk preference from launch context.
        self.with_rtk.store(ctx.with_rtk, Ordering::Relaxed);

        // Quick check that `claude` is on PATH.
        let output = std::process::Command::new("claude")
            .arg("--version")
            .output();
        if output.is_err() {
            return Err(anyhow!(
                "claude CLI not found in PATH. Run `mc mesh runtime install claude-code`."
            ));
        }

        tracing::info!(
            "claude-code agent {} ready in {}",
            ctx.agent_id,
            ctx.work_dir.display()
        );

        Ok(AgentHandle {
            agent_id: ctx.agent_id,
            runtime_kind: RuntimeKind::ClaudeCode,
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

        // Spawn `claude -p "{prompt}" --output-format stream-json --no-session-persistence`
        // in the agent's work dir.
        let work_dir = paths::mc_mesh_work_dir().join(&agent_id);

        tracing::info!("claude-code injecting task {task_id}: {}", &prompt[..prompt.len().min(80)]);

        // If RTK compression was requested, attempt to install hooks before spawning.
        // The OnceLock ensures we only run the subprocess once per runtime instance.
        'rtk_setup: {
            if !self.with_rtk.load(Ordering::Relaxed) {
                break 'rtk_setup;
            }
            if self.rtk_hooks_done.get().is_some() {
                break 'rtk_setup;
            }
            if !crate::shared::is_rtk_installed() {
                tracing::warn!(
                    "RTK requested (--with-rtk) but rtk binary not found in PATH; running without compression"
                );
                break 'rtk_setup;
            }
            if dirs::home_dir().is_none() {
                tracing::warn!("RTK: could not determine home directory; skipping hook setup");
                break 'rtk_setup;
            }
            let hooks_dir = dirs::home_dir().unwrap().join(".claude").join("hooks");
            if let Err(e) = std::fs::create_dir_all(&hooks_dir) {
                tracing::warn!("RTK: could not create hooks dir {}: {e}", hooks_dir.display());
                break 'rtk_setup;
            }
            match crate::shared::ensure_rtk_hooks().await {
                Ok(true) => {
                    tracing::info!("RTK hooks installed in {}", hooks_dir.display());
                    self.rtk_hooks_done.set(()).ok();
                }
                Ok(false) => {
                    tracing::debug!("RTK hooks already present in {}", hooks_dir.display());
                    self.rtk_hooks_done.set(()).ok();
                }
                Err(e) => tracing::warn!("RTK hooks setup failed, running without compression: {e:#}"),
            }
        }

        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg(&prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--no-session-persistence")
            .current_dir(&work_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Propagate the mc-mesh capability socket path so agents can reach `mc-mesh run`.
        if let Ok(socket) = std::env::var("MC_MESH_SOCKET") {
            cmd.env("MC_MESH_SOCKET", socket);
        }

        // Inject mc binary dir so agents can invoke `mc` without an absolute path.
        let mc_dir = crate::shared::mc_bin_dir();
        if !mc_dir.is_empty() {
            cmd.env("PATH", crate::shared::prepend_to_path(&mc_dir, &std::env::var("PATH").unwrap_or_default()));
        }

        let mut child = cmd.spawn()?;

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;

        // Stream stdout lines → ProgressEvents.
        let stream = async_stream::stream! {
            let mut stdout_lines = BufReader::new(stdout).lines();
            let mut stderr_lines = BufReader::new(stderr).lines();

            loop {
                tokio::select! {
                    line = stdout_lines.next_line() => {
                        match line {
                            Ok(Some(l)) => {
                                for ev in parse_stream_line(&l) {
                                    yield ev;
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                yield ProgressEvent::error("stdout read error", serde_json::json!({ "error": e.to_string() }));
                                break;
                            }
                        }
                    }
                    line = stderr_lines.next_line() => {
                        if let Ok(Some(l)) = line {
                            if !l.trim().is_empty() {
                                yield ProgressEvent {
                                    event_type: ProgressEventType::Warning,
                                    phase: Some("running".into()),
                                    step: None,
                                    summary: truncate(&l, 200),
                                    payload: serde_json::json!({ "stderr": l }),
                                };
                            }
                        }
                    }
                }
            }

            // Wait for the process and emit a final event on non-zero exit.
            match child.wait().await {
                Ok(status) if !status.success() => {
                    yield ProgressEvent::error(
                        format!("claude exited with {status}"),
                        serde_json::json!({ "exit_code": status.code() }),
                    );
                }
                _ => {}
            }
        };

        Ok(Box::pin(stream))
    }

    async fn signal(&self, handle: &AgentHandle, signal: AgentSignal) -> Result<()> {
        // claude -p is single-shot; signals are delivered via the next inject_task call.
        tracing::info!("Signal to claude-code agent {}: {:?}", handle.agent_id, signal);
        Ok(())
    }

    async fn collect_result(&self, handle: &AgentHandle) -> Result<TaskResult> {
        Ok(TaskResult {
            task_id: handle.agent_id.clone(),
            success: true,
            exit_code: 0,
            artifact_path: None,
            summary: "claude-code task finished".into(),
        })
    }

    async fn attach_pty(&self, handle: &AgentHandle) -> Result<PtySession> {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};

        let work_dir = paths::mc_mesh_work_dir().join(&handle.agent_id);
        std::fs::create_dir_all(&work_dir)?;

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new("claude");
        cmd.cwd(&work_dir);
        // Launch interactive claude (no -p flag = interactive TUI mode).
        let _child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut master_reader = pair.master.try_clone_reader()?;
        let mut master_writer = pair.master.take_writer()?;

        let (out_tx, out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
        let (in_tx, mut in_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

        // PTY output → channel (blocking I/O on a dedicated thread).
        tokio::task::spawn_blocking(move || {
            let mut buf = [0u8; 4096];
            loop {
                match master_reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // Channel → PTY input (blocking I/O on a dedicated thread).
        tokio::task::spawn_blocking(move || {
            loop {
                match in_rx.blocking_recv() {
                    None => break,
                    Some(bytes) => {
                        if master_writer.write_all(&bytes).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        tracing::info!("PTY session opened for claude-code agent {}", handle.agent_id);
        Ok(PtySession { output: out_rx, input: in_tx, rows: 24, cols: 80 })
    }

    async fn shutdown(&self, handle: AgentHandle) -> Result<()> {
        tracing::info!("Shutting down claude-code agent {}", handle.agent_id);
        Ok(())
    }

    async fn ensure_installed(&self) -> Result<()> {
        // Return immediately if already verified during this daemon lifetime.
        if self.install_done.get().is_some() {
            return Ok(());
        }

        // Step 1: check if claude is already on PATH.
        let already_present = tokio::process::Command::new("claude")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !already_present {
            // Step 2: ensure npm is available before attempting install.
            tokio::process::Command::new("npm")
                .arg("--version")
                .output()
                .await
                .map_err(|e| anyhow!(
                    "claude CLI not found and npm check failed ({e}). \
                     Install Node.js (https://nodejs.org) then re-run the daemon."
                ))?;

            tracing::info!("claude not found — installing via npm…");
            let out = tokio::process::Command::new("npm")
                .args(["install", "-g", "@anthropic-ai/claude-code"])
                .output()
                .await
                .map_err(|e| anyhow!("npm install failed to launch: {e}"))?;

            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(anyhow!(
                    "npm install -g @anthropic-ai/claude-code failed (exit {}): {stderr}",
                    out.status
                ));
            }
            tracing::info!("claude-code installed successfully");
        }

        // Step 3: render harness config.
        tokio::task::block_in_place(|| self.render_harness())?;

        // Mark done so subsequent calls are no-ops.
        let _ = self.install_done.set(());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_mesh_core::progress::ProgressEventType;

    #[test]
    fn system_line_emits_phase_started() {
        let events = parse_stream_line(r#"{"type":"system","subtype":"init"}"#);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::PhaseStarted);
    }

    #[test]
    fn assistant_text_emits_step_started() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Thinking…"}]}}"#;
        let events = parse_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::StepStarted);
        assert!(events[0].summary.contains("Thinking"));
    }

    #[test]
    fn assistant_tool_use_emits_step_with_tool_name() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"read_file","id":"t1","input":{}}]}}"#;
        let events = parse_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::StepStarted);
        assert!(events[0].step.as_deref().unwrap_or("").contains("read_file"));
    }

    #[test]
    fn assistant_multiple_content_items() {
        let line = r#"{"type":"assistant","message":{"content":[
            {"type":"text","text":"hello"},
            {"type":"tool_use","name":"write_file","id":"t2","input":{}}
        ]}}"#;
        let events = parse_stream_line(line);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn result_success_emits_phase_finished() {
        let line = r#"{"type":"result","subtype":"success","result":"done"}"#;
        let events = parse_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::PhaseFinished);
    }

    #[test]
    fn result_error_emits_error() {
        let line = r#"{"type":"result","subtype":"error","result":"something went wrong"}"#;
        let events = parse_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::Error);
    }

    #[test]
    fn unknown_type_emits_nothing() {
        let events = parse_stream_line(r#"{"type":"unknown_future_type"}"#);
        assert!(events.is_empty());
    }

    #[test]
    fn non_json_line_emits_info() {
        let events = parse_stream_line("plain text output");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::Info);
    }

    #[test]
    fn blank_line_emits_nothing() {
        assert!(parse_stream_line("").is_empty());
        assert!(parse_stream_line("   ").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_installed_does_not_panic() {
        let runtime = ClaudeCodeRuntime::new();
        let _ = runtime.ensure_installed().await; // Ok or Err, but no panic
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_adds_ellipsis() {
        let result = truncate("abcdefghij", 5);
        assert!(result.starts_with("abcde"));
        assert!(result.contains('…'));
    }

}
