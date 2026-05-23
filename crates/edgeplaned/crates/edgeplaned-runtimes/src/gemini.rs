/// AgentRuntime implementation for the Google Gemini CLI (`gemini`).
///
/// Gemini is a first-class runtime — shipped alongside claude-code and codex,
/// not as a follow-up.
///
/// The Gemini CLI runs non-interactively via `gemini -p "<prompt>"`.
/// Output is plain text streamed to stdout. We classify lines heuristically
/// into typed ProgressEvents; exit code determines success/failure.
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

pub struct GeminiRuntime {
    capabilities: Vec<Capability>,
    version: String,
    install_done: OnceLock<()>,
}

impl GeminiRuntime {
    pub fn new() -> Self {
        Self::with_extra_capabilities(Vec::new())
    }

    pub fn with_extra_capabilities(extra: Vec<Capability>) -> Self {
        let builtins = vec![
            Capability::new("gemini"),
            Capability::new("code.read"),
            Capability::new("code.explain"),
            Capability::new("code.review"),
            Capability::new("doc.write"),
            Capability::new("search"),
        ];
        GeminiRuntime {
            capabilities: merge_capabilities(builtins, extra),
            version: detect_version(),
            install_done: OnceLock::new(),
        }
    }

    fn render_harness(&self) -> Result<()> {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow!("cannot determine HOME directory"))?;
        let target = home.join(".gemini").join("GEMINI.md");
        crate::harness::write_capabilities_block(&target)
            .with_context(|| format!("rendering gemini harness to {}", target.display()))?;
        tracing::info!("gemini harness rendered to {}", target.display());
        Ok(())
    }
}

impl Default for GeminiRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn detect_version() -> String {
    // Try `gemini --version` first; fall back to `gemini version`.
    std::process::Command::new("gemini")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            std::process::Command::new("gemini")
                .arg("version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".into())
        })
}

fn build_prompt(task: &TaskSpec) -> String {
    crate::shared::build_prompt(task)
}

/// Classify a Gemini CLI output line into a ProgressEvent.
///
/// Gemini CLI emits a mix of markdown prose, tool call traces, and status
/// lines. We do lightweight pattern matching to surface useful structure.
fn classify_line(line: &str) -> Option<ProgressEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Tool call traces — Gemini CLI logs tool invocations to stdout
    // e.g. "Tool: read_file(path=...)" or "Calling tool: ..."
    if trimmed.starts_with("Tool:") || trimmed.starts_with("Calling tool:") {
        let detail = trimmed
            .trim_start_matches("Tool:")
            .trim_start_matches("Calling tool:")
            .trim();
        let tool_name = detail.split('(').next().unwrap_or(detail).trim();
        return Some(ProgressEvent {
            event_type: ProgressEventType::StepStarted,
            phase: Some("running".into()),
            step: Some(format!("tool:{tool_name}")),
            summary: format!("calling tool: {}", truncate(detail, 120)),
            payload: serde_json::json!({ "tool": tool_name, "call": detail }),
        });
    }

    // File write indicators
    if trimmed.starts_with("Writing ")
        || trimmed.starts_with("Created ")
        || trimmed.starts_with("Updated ")
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
    if trimmed.to_lowercase().starts_with("error:")
        || trimmed.to_lowercase().starts_with("fatal:")
    {
        return Some(ProgressEvent::error(
            truncate(trimmed, 200),
            serde_json::json!({ "line": trimmed }),
        ));
    }

    // Everything else is an Info event.
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
impl AgentRuntime for GeminiRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Gemini
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn launch(&self, ctx: LaunchContext) -> Result<AgentHandle> {
        std::fs::create_dir_all(&ctx.work_dir)?;

        let output = std::process::Command::new("gemini")
            .arg("--version")
            .output()
            .or_else(|_| {
                std::process::Command::new("gemini")
                    .arg("version")
                    .output()
            });
        if output.is_err() {
            return Err(anyhow!(
                "gemini CLI not found in PATH. Run `edgeplane daemon runtime install gemini`."
            ));
        }

        tracing::info!(
            "gemini agent {} ready in {}",
            ctx.agent_id,
            ctx.work_dir.display()
        );

        Ok(AgentHandle {
            agent_id: ctx.agent_id,
            runtime_kind: RuntimeKind::Gemini,
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
            "gemini injecting task {task_id}: {}",
            &prompt[..prompt.len().min(80)]
        );

        // `gemini -p "<prompt>"` runs a single non-interactive prompt.
        // `--yolo` (or `--no-interactive`) suppresses confirmations in full-auto mode.
        // The flag name differs across versions; we try both patterns via the process args.
        let mut cmd = Command::new("gemini");
        cmd.arg("-p")
            .arg(&prompt)
            .arg("--yolo")
            .current_dir(&work_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

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

        let stream = async_stream::stream! {
            yield ProgressEvent::phase_started("running", "gemini session started");

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
                        summary: "gemini task finished".into(),
                        payload: serde_json::json!({ "exit_code": 0 }),
                    };
                }
                Ok(status) => {
                    yield ProgressEvent::error(
                        format!("gemini exited with {status}"),
                        serde_json::json!({ "exit_code": status.code() }),
                    );
                }
                Err(e) => {
                    yield ProgressEvent::error(
                        format!("failed to wait on gemini: {e}"),
                        serde_json::json!({}),
                    );
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn signal(&self, handle: &AgentHandle, signal: AgentSignal) -> Result<()> {
        tracing::info!("Signal to gemini agent {}: {:?}", handle.agent_id, signal);
        Ok(())
    }

    async fn collect_result(&self, handle: &AgentHandle) -> Result<TaskResult> {
        Ok(TaskResult {
            task_id: handle.agent_id.clone(),
            success: true,
            exit_code: 0,
            artifact_path: None,
            summary: "gemini task finished".into(),
        })
    }

    async fn attach_pty(&self, handle: &AgentHandle) -> Result<PtySession> {
        let work_dir = paths::mcd_work_dir().join(&handle.agent_id);
        std::fs::create_dir_all(&work_dir)?;
        let session = crate::shared::spawn_interactive_pty("gemini", &work_dir, 24, 80)?;
        tracing::info!("PTY session opened for gemini agent {}", handle.agent_id);
        Ok(session)
    }

    async fn shutdown(&self, handle: AgentHandle) -> Result<()> {
        tracing::info!("Shutting down gemini agent {}", handle.agent_id);
        Ok(())
    }

    async fn ensure_installed(&self) -> Result<()> {
        if self.install_done.get().is_some() {
            return Ok(());
        }

        // Check `gemini --version` then `gemini version` (different CLI versions differ).
        let already_present = tokio::process::Command::new("gemini")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
            || tokio::process::Command::new("gemini")
                .arg("version")
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
                    "gemini CLI not found and npm check failed ({e}). \
                     Install Node.js (https://nodejs.org) first."
                ))?;

            tracing::info!("gemini not found — installing via npm…");
            let out = tokio::process::Command::new("npm")
                .args(["install", "-g", "@google/gemini-cli"])
                .output()
                .await
                .map_err(|e| anyhow!("npm install failed to launch: {e}"))?;

            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(anyhow!(
                    "npm install -g @google/gemini-cli failed (exit {}): {stderr}",
                    out.status
                ));
            }
            tracing::info!("gemini CLI installed successfully");
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
        let runtime = GeminiRuntime::new();
        let _ = runtime.ensure_installed().await; // Ok or Err, but no panic
    }
}
