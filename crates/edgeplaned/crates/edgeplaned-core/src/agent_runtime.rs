use crate::progress::ProgressEvent;
use crate::types::{AgentHandle, AgentSignal, Capability, LaunchContext, PtySession, RuntimeKind, TaskResult, TaskSpec};
use anyhow::Result;
use async_trait::async_trait;
use futures::stream::BoxStream;

/// The core abstraction that makes claude-code / codex / gemini pluggable.
///
/// Implementors wrap a specific CLI runtime and translate its output into the
/// structured edgeplaned progress event model.  The trait is small enough that a
/// user can add a custom runtime by implementing ~7 async methods.
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    /// Which runtime kind this implementation provides.
    fn kind(&self) -> RuntimeKind;

    /// Human-readable version string (populated after launch).
    fn version(&self) -> &str;

    /// Capabilities this runtime advertises to the scheduler.
    fn capabilities(&self) -> &[Capability];

    /// Spawn the runtime as a supervised child process.
    ///
    /// Must not block; returns immediately with a handle to the spawned process.
    async fn launch(&self, ctx: LaunchContext) -> Result<AgentHandle>;

    /// Inject a task into an already-running agent session.
    ///
    /// Returns a stream of typed progress events.  The stream ends when the
    /// task is complete (success or failure).  The daemon forwards events to
    /// the backend as they arrive.
    async fn inject_task(
        &self,
        handle: &AgentHandle,
        task: &TaskSpec,
    ) -> Result<BoxStream<'static, ProgressEvent>>;

    /// Signal a running agent — deliver a peer message, user input, or cancel.
    async fn signal(&self, handle: &AgentHandle, signal: AgentSignal) -> Result<()>;

    /// Collect the final `TaskResult` once the progress stream has closed.
    async fn collect_result(&self, handle: &AgentHandle) -> Result<TaskResult>;

    /// Open an interactive PTY for human attach (`edgeplane daemon attach`).
    ///
    /// Returns a bidirectional `PtySession`.  The caller reads terminal output
    /// from `session.output` and forwards keystrokes via `session.input`.
    async fn attach_pty(&self, handle: &AgentHandle) -> Result<PtySession>;

    /// Graceful shutdown of the child process.
    async fn shutdown(&self, handle: AgentHandle) -> Result<()>;

    /// Check that the agent CLI is available on PATH and install it if needed.
    ///
    /// Called by the daemon before `launch()`. Returns an error if the CLI
    /// cannot be found or installed.  Default no-op is fine for runtimes
    /// that manage their own availability externally.
    async fn ensure_installed(&self) -> Result<()> {
        Ok(())
    }
}

/// Dynamic dispatch alias for convenience.
pub type DynAgentRuntime = Box<dyn AgentRuntime>;
