/// AgentRuntime implementation for the Goose CLI.
///
/// Uses `--output-format stream-json` for structured real-time output.
/// Each JSONL line is parsed into a typed `ProgressEvent`.
///
/// Goose stream-json format:
///   {"type":"message","message":{"role":"assistant","content":[...]}}
///   {"type":"notification","extension_id":"...","log":{"message":"..."}}
///   {"type":"notification","extension_id":"...","progress":{"progress":0.5,"total":1.0,"message":"..."}}
///   {"type":"error","error":"..."}
///   {"type":"complete","total_tokens":1234}
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::stream::BoxStream;
use edgeplaned_core::agent_runtime::AgentRuntime;
use edgeplaned_core::paths;
use edgeplaned_core::progress::{ProgressEvent, ProgressEventType};
use edgeplaned_core::types::{
    AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult,
    TaskSpec,
};

use crate::shared::merge_capabilities;
use std::sync::OnceLock;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct GooseRuntime {
    capabilities: Vec<Capability>,
    version: String,
    install_done: OnceLock<()>,
}

impl GooseRuntime {
    pub fn new() -> Self {
        Self::with_extra_capabilities(Vec::new())
    }

    pub fn with_extra_capabilities(extra: Vec<Capability>) -> Self {
        let builtins = vec![
            Capability::new("goose"),
            Capability::new("mcp"),
            Capability::new("tool_use"),
            Capability::new("recipe"),
        ];
        GooseRuntime {
            capabilities: merge_capabilities(builtins, extra),
            version: detect_version(),
            install_done: OnceLock::new(),
        }
    }

    fn render_harness(&self) -> Result<()> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow!("cannot determine HOME directory"))?;
        let target = home.join(".config").join("goose").join("CAPABILITIES.md");
        crate::harness::write_capabilities_block(&target)
            .with_context(|| format!("rendering goose harness to {}", target.display()))?;
        tracing::info!("goose harness rendered to {}", target.display());
        Ok(())
    }
}

impl Default for GooseRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn detect_version() -> String {
    std::process::Command::new("goose")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

/// Build the prompt to send to goose from a TaskSpec.
/// Uses the shared builder which injects agent profile and domain roster.
fn build_prompt(task: &TaskSpec) -> String {
    crate::shared::build_prompt(task)
}

/// Parse a single stream-json line from goose into zero or more ProgressEvents.
///
/// goose `--output-format stream-json` emits JSONL with these main types:
///   {"type":"message","message":{"role":"assistant","content":[...]}}
///   {"type":"notification","extension_id":"...","log":{"message":"..."}}
///   {"type":"notification","extension_id":"...","progress":{...}}
///   {"type":"error","error":"..."}
///   {"type":"complete","total_tokens":1234}
fn parse_goose_stream_line(line: &str) -> Vec<ProgressEvent> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        // Not JSON (e.g. debug output); emit as info.
        if !line.trim().is_empty() {
            return vec![ProgressEvent::info(line.trim().to_string())];
        }
        return vec![];
    };

    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match kind {
        "message" => {
            // Extract content items from the assistant message.
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
        "notification" => {
            let mut events = vec![];
            // log.message → Info event
            if let Some(msg) = v.pointer("/log/message").and_then(|m| m.as_str()) {
                if !msg.trim().is_empty() {
                    events.push(ProgressEvent::info(truncate(msg, 200)));
                }
            }
            // progress → StepStarted with step="progress"
            if let Some(progress_obj) = v.get("progress") {
                let msg = progress_obj
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("in progress");
                events.push(ProgressEvent {
                    event_type: ProgressEventType::StepStarted,
                    phase: Some("running".into()),
                    step: Some("progress".into()),
                    summary: truncate(msg, 200),
                    payload: progress_obj.clone(),
                });
            }
            events
        }
        "error" => {
            let err = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown error");
            vec![ProgressEvent::error(
                truncate(err, 200),
                serde_json::json!({ "detail": err }),
            )]
        }
        "complete" => {
            let total_tokens = v.get("total_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
            vec![ProgressEvent {
                event_type: ProgressEventType::PhaseFinished,
                phase: Some("running".into()),
                step: None,
                summary: "goose task complete".into(),
                payload: serde_json::json!({ "total_tokens": total_tokens }),
            }]
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
impl AgentRuntime for GooseRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Custom("goose".into())
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn launch(&self, ctx: LaunchContext) -> Result<AgentHandle> {
        // goose is stateless — each inject_task spawns a fresh `goose run` process.
        // Launch just validates the binary exists and creates the work dir.
        std::fs::create_dir_all(&ctx.work_dir)?;

        // Quick check that `goose` is on PATH.
        let output = std::process::Command::new("goose")
            .arg("--version")
            .output();
        if output.is_err() {
            return Err(anyhow!(
                "goose CLI not found in PATH. Install goose and ensure it is on PATH."
            ));
        }

        tracing::info!(
            "goose agent {} ready in {}",
            ctx.agent_id,
            ctx.work_dir.display()
        );

        Ok(AgentHandle {
            agent_id: ctx.agent_id,
            runtime_kind: RuntimeKind::Custom("goose".into()),
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

        // Work dir for the goose process — isolated per agent.
        let work_dir = paths::mcd_work_dir().join(&agent_id);

        std::fs::create_dir_all(&work_dir)?;
        tracing::info!(
            "goose injecting task {task_id}: {}",
            &prompt[..prompt.len().min(80)]
        );

        // Resolve LiteLLM connection details from env.
        let litellm_host = std::env::var("EP_LITELLM_HOST")
            .unwrap_or_else(|_| "http://litellm:4000".into());
        let litellm_api_key = std::env::var("EP_LITELLM_API_KEY").ok();

        let mut cmd = Command::new("goose");
        cmd.arg("run")
            .arg("--text")
            .arg(&prompt)
            .arg("--quiet")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--no-session")
            .arg("--max-turns")
            .arg("50")
            // Isolate goose's config to the agent's work dir.
            .env("XDG_CONFIG_HOME", &work_dir)
            .env("GOOSE_PROVIDER", "litellm")
            .env("LITELLM_HOST", &litellm_host)
            .env("GOOSE_MODEL", std::env::var("EP_GOOSE_MODEL").unwrap_or_else(|_| "local-agent".into()))
            .env("GOOSE_MODE", "Auto")
            .env("EP_MESH_AGENT_ID", &agent_id)
            .env("EP_MESH_TASK_ID", &task_id)
            .current_dir(&work_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Conditionally set the API key only if it is present and non-empty.
        if let Some(key) = litellm_api_key {
            if !key.is_empty() {
                cmd.env("LITELLM_API_KEY", key);
            }
        }

        // Propagate the edgeplaned capability socket path so agents can reach `edgeplaned run`.
        if let Ok(socket) = std::env::var("EP_MESH_SOCKET") {
            cmd.env("EP_MESH_SOCKET", socket);
        }

        // Inject edgeplane binary dir so agents can invoke `edgeplane` without an absolute path.
        let ep_dir = crate::shared::ep_bin_dir();
        if !ep_dir.is_empty() {
            cmd.env("PATH", crate::shared::prepend_to_path(&ep_dir, &std::env::var("PATH").unwrap_or_default()));
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
                                for ev in parse_goose_stream_line(&l) {
                                    yield ev;
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                yield ProgressEvent::error(
                                    "stdout read error",
                                    serde_json::json!({ "error": e.to_string() }),
                                );
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
                        format!("goose exited with {status}"),
                        serde_json::json!({ "exit_code": status.code() }),
                    );
                }
                _ => {}
            }
        };

        Ok(Box::pin(stream))
    }

    async fn signal(&self, handle: &AgentHandle, signal: AgentSignal) -> Result<()> {
        // goose run is single-shot; signals are noted but not forwarded.
        tracing::info!("Signal to goose agent {}: {:?}", handle.agent_id, signal);
        Ok(())
    }

    async fn collect_result(&self, handle: &AgentHandle) -> Result<TaskResult> {
        Ok(TaskResult {
            task_id: handle.agent_id.clone(),
            success: true,
            exit_code: 0,
            artifact_path: None,
            summary: "goose task finished".into(),
        })
    }

    async fn attach_pty(&self, handle: &AgentHandle) -> Result<PtySession> {
        let work_dir = paths::mcd_work_dir().join(&handle.agent_id);
        std::fs::create_dir_all(&work_dir)?;
        let session = crate::shared::spawn_interactive_pty("goose", &work_dir, 24, 80)?;
        tracing::info!("PTY session opened for goose agent {}", handle.agent_id);
        Ok(session)
    }

    async fn shutdown(&self, handle: AgentHandle) -> Result<()> {
        tracing::info!("Shutting down goose agent {}", handle.agent_id);
        Ok(())
    }

    async fn ensure_installed(&self) -> Result<()> {
        if self.install_done.get().is_some() {
            return Ok(());
        }

        // Goose is pre-installed at bootstrap; verify presence only.
        let ok = tokio::process::Command::new("goose")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            return Err(anyhow!(
                "goose CLI not found or returned non-zero exit. \
                 Install from https://github.com/block/goose — \
                 run: curl -fsSL https://github.com/block/goose/releases/latest/download/download.sh | sh"
            ));
        }

        tokio::task::block_in_place(|| self.render_harness())?;

        let _ = self.install_done.set(());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use edgeplaned_core::progress::ProgressEventType;

    #[test]
    fn message_with_text_emits_step_started() {
        let line = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Working on it…"}]}}"#;
        let events = parse_goose_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::StepStarted);
        assert!(events[0].summary.contains("Working on it"));
        assert_eq!(events[0].step.as_deref(), Some("thinking"));
    }

    #[test]
    fn message_with_tool_use_emits_step_with_tool_name() {
        let line = r#"{"type":"message","message":{"role":"assistant","content":[{"type":"tool_use","name":"shell","id":"t1","input":{}}]}}"#;
        let events = parse_goose_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::StepStarted);
        assert!(events[0].step.as_deref().unwrap_or("").contains("shell"));
    }

    #[test]
    fn message_with_multiple_content_items() {
        let line = r#"{"type":"message","message":{"role":"assistant","content":[
            {"type":"text","text":"hello"},
            {"type":"tool_use","name":"read_file","id":"t2","input":{}}
        ]}}"#;
        let events = parse_goose_stream_line(line);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, ProgressEventType::StepStarted);
        assert_eq!(events[1].event_type, ProgressEventType::StepStarted);
    }

    #[test]
    fn notification_with_log_emits_info() {
        let line = r#"{"type":"notification","extension_id":"developer","log":{"message":"Running tests"}}"#;
        let events = parse_goose_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::Info);
        assert!(events[0].summary.contains("Running tests"));
    }

    #[test]
    fn notification_with_progress_emits_step_started() {
        let line = r#"{"type":"notification","extension_id":"shell","progress":{"progress":0.5,"total":1.0,"message":"halfway"}}"#;
        let events = parse_goose_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::StepStarted);
        assert_eq!(events[0].step.as_deref(), Some("progress"));
        assert!(events[0].summary.contains("halfway"));
    }

    #[test]
    fn notification_with_log_and_progress_emits_both() {
        let line = r#"{"type":"notification","extension_id":"shell","log":{"message":"info"},"progress":{"progress":0.1,"total":1.0,"message":"starting"}}"#;
        let events = parse_goose_stream_line(line);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn error_event_emits_error() {
        let line = r#"{"type":"error","error":"something went wrong"}"#;
        let events = parse_goose_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::Error);
        assert!(events[0].summary.contains("something went wrong"));
    }

    #[test]
    fn complete_event_emits_phase_finished() {
        let line = r#"{"type":"complete","total_tokens":1234}"#;
        let events = parse_goose_stream_line(line);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::PhaseFinished);
        assert_eq!(events[0].phase.as_deref(), Some("running"));
        assert_eq!(events[0].payload["total_tokens"], 1234);
    }

    #[test]
    fn blank_line_emits_nothing() {
        assert!(parse_goose_stream_line("").is_empty());
        assert!(parse_goose_stream_line("   ").is_empty());
    }

    #[test]
    fn non_json_line_emits_info() {
        let events = parse_goose_stream_line("plain text debug output");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, ProgressEventType::Info);
    }

    #[test]
    fn unknown_type_emits_nothing() {
        let events = parse_goose_stream_line(r#"{"type":"future_unknown_type"}"#);
        assert!(events.is_empty());
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

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_installed_does_not_panic() {
        let runtime = GooseRuntime::new();
        let _ = runtime.ensure_installed().await; // Ok or Err, but no panic
    }
}
