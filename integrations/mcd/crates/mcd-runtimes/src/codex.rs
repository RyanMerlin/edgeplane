/// AgentRuntime implementation for the OpenAI Codex CLI (`codex`).
///
/// Codex CLI does not emit structured JSON; it streams plain text to stdout.
/// We run `codex --approval-mode full-auto --quiet "<prompt>"` and map each
/// stdout line to a typed ProgressEvent. Exit code determines success/failure.
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::stream::BoxStream;
use mcd_core::agent_runtime::AgentRuntime;
use mcd_core::paths;
use mcd_core::progress::{ProgressEvent, ProgressEventType};
use mcd_core::types::{
    AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult,
    TaskSpec,
};

use crate::shared::merge_capabilities;
use std::sync::OnceLock;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub struct CodexRuntime {
    capabilities: Vec<Capability>,
    version: String,
    install_done: OnceLock<()>,
}

impl CodexRuntime {
    pub fn new() -> Self {
        Self::with_extra_capabilities(Vec::new())
    }

    pub fn with_extra_capabilities(extra: Vec<Capability>) -> Self {
        let builtins = vec![
            Capability::new("codex"),
            Capability::new("code.read"),
            Capability::new("code.edit"),
            Capability::new("code.refactor"),
            Capability::new("test.write"),
        ];
        CodexRuntime {
            capabilities: merge_capabilities(builtins, extra),
            version: detect_version(),
            install_done: OnceLock::new(),
        }
    }

    fn render_harness(&self) -> Result<()> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow!("cannot determine HOME directory"))?;
        let target = home.join(".codex").join("instructions.md");
        crate::harness::write_capabilities_block(&target)
            .with_context(|| format!("rendering codex harness to {}", target.display()))?;
        tracing::info!("codex harness rendered to {}", target.display());
        Ok(())
    }
}

impl Default for CodexRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn detect_version() -> String {
    std::process::Command::new("codex")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn build_prompt(task: &TaskSpec) -> String {
    crate::shared::build_prompt(task)
}

/// Heuristically classify a codex output line into a ProgressEvent.
///
/// Codex emits a mix of reasoning prose, shell command lines, and status
/// messages. We do lightweight pattern matching to surface useful structure
/// without being brittle about it.
fn classify_line(line: &str) -> Option<ProgressEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Shell command invocations: lines starting with $ or `cmd`
    if trimmed.starts_with("$ ") || trimmed.starts_with("` ") {
        let cmd = trimmed.trim_start_matches("$ ").trim_start_matches("` ");
        return Some(ProgressEvent {
            event_type: ProgressEventType::StepStarted,
            phase: Some("running".into()),
            step: Some(format!("shell:{}", cmd.split_whitespace().next().unwrap_or("?"))),
            summary: format!("running: {}", truncate(cmd, 120)),
            payload: serde_json::json!({ "command": cmd }),
        });
    }

    // File write indicators
    if trimmed.starts_with("Writing ")
        || trimmed.starts_with("Created ")
        || trimmed.starts_with("Updated ")
        || trimmed.starts_with("Saved ")
    {
        return Some(ProgressEvent {
            event_type: ProgressEventType::StepFinished,
            phase: Some("running".into()),
            step: Some("file_write".into()),
            summary: truncate(trimmed, 160),
            payload: serde_json::json!({ "line": trimmed }),
        });
    }

    // Explicit error markers
    if trimmed.to_lowercase().starts_with("error:") || trimmed.to_lowercase().starts_with("fatal:") {
        return Some(ProgressEvent::error(
            truncate(trimmed, 200),
            serde_json::json!({ "line": trimmed }),
        ));
    }

    // Everything else is an Info event
    Some(ProgressEvent::info(truncate(trimmed, 200)))
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
impl AgentRuntime for CodexRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Codex
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn launch(&self, ctx: LaunchContext) -> Result<AgentHandle> {
        std::fs::create_dir_all(&ctx.work_dir)?;

        let output = std::process::Command::new("codex")
            .arg("--version")
            .output();
        if output.is_err() {
            return Err(anyhow!(
                "codex CLI not found in PATH. Run `mc daemon runtime install codex`."
            ));
        }

        tracing::info!(
            "codex agent {} ready in {}",
            ctx.agent_id,
            ctx.work_dir.display()
        );

        Ok(AgentHandle {
            agent_id: ctx.agent_id,
            runtime_kind: RuntimeKind::Codex,
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

        tracing::info!(
            "codex injecting task {task_id}: {}",
            &prompt[..prompt.len().min(80)]
        );

        // `codex --approval-mode full-auto` runs non-interactively.
        // `--quiet` suppresses the spinner/ANSI chrome so we get clean lines.
        let mut cmd = Command::new("codex");
        cmd.arg("--approval-mode")
            .arg("full-auto")
            .arg("--quiet")
            .arg(&prompt)
            .current_dir(&work_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Propagate the mcd capability socket path so agents can reach `mcd run`.
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

        let stream = async_stream::stream! {
            yield ProgressEvent::phase_started("running", "codex session started");

            let mut stdout_lines = BufReader::new(stdout).lines();
            let mut stderr_lines = BufReader::new(stderr).lines();
            let mut done = false;

            while !done {
                tokio::select! {
                    line = stdout_lines.next_line() => {
                        match line {
                            Ok(Some(l)) => {
                                if let Some(ev) = classify_line(&l) {
                                    yield ev;
                                }
                            }
                            Ok(None) => { done = true; }
                            Err(e) => {
                                yield ProgressEvent::error(
                                    "stdout read error",
                                    serde_json::json!({ "error": e.to_string() }),
                                );
                                done = true;
                            }
                        }
                    }
                    line = stderr_lines.next_line() => {
                        if let Ok(Some(l)) = line {
                            let trimmed = l.trim();
                            if !trimmed.is_empty() {
                                yield ProgressEvent {
                                    event_type: ProgressEventType::Warning,
                                    phase: Some("running".into()),
                                    step: None,
                                    summary: truncate(trimmed, 200),
                                    payload: serde_json::json!({ "stderr": trimmed }),
                                };
                            }
                        }
                    }
                }
            }

            match child.wait().await {
                Ok(status) if status.success() => {
                    yield ProgressEvent {
                        event_type: ProgressEventType::PhaseFinished,
                        phase: Some("running".into()),
                        step: None,
                        summary: "codex task finished".into(),
                        payload: serde_json::json!({ "exit_code": 0 }),
                    };
                }
                Ok(status) => {
                    yield ProgressEvent::error(
                        format!("codex exited with {status}"),
                        serde_json::json!({ "exit_code": status.code() }),
                    );
                }
                Err(e) => {
                    yield ProgressEvent::error(
                        format!("failed to wait on codex: {e}"),
                        serde_json::json!({}),
                    );
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn signal(&self, handle: &AgentHandle, signal: AgentSignal) -> Result<()> {
        tracing::info!("Signal to codex agent {}: {:?}", handle.agent_id, signal);
        Ok(())
    }

    async fn collect_result(&self, handle: &AgentHandle) -> Result<TaskResult> {
        Ok(TaskResult {
            task_id: handle.agent_id.clone(),
            success: true,
            exit_code: 0,
            artifact_path: None,
            summary: "codex task finished".into(),
        })
    }

    async fn attach_pty(&self, handle: &AgentHandle) -> Result<PtySession> {
        let work_dir = paths::mcd_work_dir().join(&handle.agent_id);
        std::fs::create_dir_all(&work_dir)?;
        let session = crate::shared::spawn_interactive_pty("codex", &work_dir, 24, 80)?;
        tracing::info!("PTY session opened for codex agent {}", handle.agent_id);
        Ok(session)
    }

    async fn shutdown(&self, handle: AgentHandle) -> Result<()> {
        tracing::info!("Shutting down codex agent {}", handle.agent_id);
        Ok(())
    }

    async fn ensure_installed(&self) -> Result<()> {
        if self.install_done.get().is_some() {
            return Ok(());
        }

        let already_present = tokio::process::Command::new("codex")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !already_present {
            tokio::process::Command::new("npm")
                .arg("--version")
                .output()
                .await
                .map_err(|e| anyhow!(
                    "codex CLI not found and npm check failed ({e}). \
                     Install Node.js (https://nodejs.org) first."
                ))?;

            tracing::info!("codex not found — installing via npm…");
            let out = tokio::process::Command::new("npm")
                .args(["install", "-g", "@openai/codex"])
                .output()
                .await
                .map_err(|e| anyhow!("npm install failed to launch: {e}"))?;

            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(anyhow!(
                    "npm install -g @openai/codex failed (exit {}): {stderr}",
                    out.status
                ));
            }
            tracing::info!("codex installed successfully");
        }

        tokio::task::block_in_place(|| self.render_harness())?;
        let _ = self.install_done.set(());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn ensure_installed_does_not_panic() {
        let runtime = CodexRuntime::new();
        let _ = runtime.ensure_installed().await; // Ok or Err, but no panic
    }
}
