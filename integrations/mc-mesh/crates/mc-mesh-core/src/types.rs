use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which agent runtime implementation to use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    ClaudeCode,
    Codex,
    Gemini,
    Custom(String),
}

impl std::fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeKind::ClaudeCode => write!(f, "claude_code"),
            RuntimeKind::Codex => write!(f, "codex"),
            RuntimeKind::Gemini => write!(f, "gemini"),
            RuntimeKind::Custom(s) => write!(f, "{s}"),
        }
    }
}

/// A capability string, e.g. "code.edit", "test.run", "claude_code".
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability(pub String);

impl Capability {
    pub fn new(s: impl Into<String>) -> Self {
        Capability(s.into())
    }
}

/// Spec for a task as received from the backend, enriched with agent context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub kluster_id: String,
    pub mission_id: String,
    pub title: String,
    pub description: String,
    pub input_json: String,
    pub required_capabilities: Vec<String>,
    pub produces: serde_json::Value,
    pub consumes: serde_json::Value,
    /// This agent's own profile (name, role, instructions, scope, constraints).
    /// Injected by the daemon before calling inject_task.
    pub agent_profile: Option<serde_json::Value>,
    /// Concise roster of other agents in this mission.
    /// Each entry: {id, name, role, description, scope, capabilities, status, hostname}.
    pub mission_roster: Vec<serde_json::Value>,
}

/// Context passed to `AgentRuntime::launch`.
#[derive(Debug, Clone, Default)]
pub struct LaunchContext {
    pub agent_id: String,
    pub mission_id: String,
    /// Working directory the agent will start in.
    pub work_dir: std::path::PathBuf,
    /// Base URL of the MissionControl backend.
    pub backend_url: String,
    /// Bearer token for authenticating to the backend.
    pub backend_token: String,
    /// Environment variables to inject.
    pub env: Vec<(String, String)>,
    /// This agent's profile (name, role, instructions, scope, constraints).
    /// Injected into every task prompt so the agent knows who it is.
    pub profile: Option<serde_json::Value>,
    /// Concise roster of other agents in the mission.
    /// Injected into every task prompt so the agent can reason about delegation.
    pub roster: Vec<serde_json::Value>,
    /// If true, the runtime should attempt to enable RTK (Rust Token Killer) hooks
    /// for output compression before spawning the agent process.
    pub with_rtk: bool,
}

/// A handle to a running agent runtime process.
#[derive(Debug)]
pub struct AgentHandle {
    pub agent_id: String,
    pub runtime_kind: RuntimeKind,
    /// PID of the spawned child process (best-effort, may be 0 for PTY-wrapped procs).
    pub pid: u32,
}

/// A signal delivered to a running agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentSignal {
    /// A peer message to deliver to the agent.
    PeerMessage {
        from_agent_id: String,
        channel: String,
        body: serde_json::Value,
    },
    /// User-supplied input for a `needs_input` prompt.
    UserInput { text: String },
    /// Cancellation request.
    Cancel,
}

/// Final result of a completed task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub success: bool,
    pub exit_code: i32,
    pub artifact_path: Option<std::path::PathBuf>,
    pub summary: String,
}

/// Bidirectional PTY session returned from `AgentRuntime::attach_pty`.
///
/// `output` receives bytes from the PTY master (terminal output to display).
/// `input`  sends bytes to  the PTY master (keystrokes from the user).
pub struct PtySession {
    pub output: tokio::sync::mpsc::Receiver<Vec<u8>>,
    pub input: tokio::sync::mpsc::Sender<Vec<u8>>,
    pub rows: u16,
    pub cols: u16,
}

/// Legacy one-directional alias (kept for call sites that only need output).
pub type PtyStream = tokio::sync::mpsc::Receiver<Vec<u8>>;

/// MeshAgent record as returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshAgentRecord {
    pub id: String,
    pub mission_id: String,
    pub runtime_kind: String,
    pub status: String,
    pub current_task_id: Option<String>,
}

/// MeshTask record as returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshTaskRecord {
    pub id: String,
    pub kluster_id: String,
    pub mission_id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub claim_policy: String,
    pub required_capabilities: Vec<String>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    /// Lease ID returned by the backend on claim; used to authenticate
    /// heartbeat/complete/fail calls and detect stolen leases (409).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_lease_id: Option<String>,
}
