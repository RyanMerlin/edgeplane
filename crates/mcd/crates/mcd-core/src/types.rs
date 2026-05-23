use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which agent runtime implementation to use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Stateless `claude -p` per task — task-mode only.
    ClaudeCode,
    /// `claude-agent-acp` over JSON-RPC/stdio — supports both task-mode
    /// (one-shot prompt per task) and persistent-mode (long-lived session
    /// driven by the supervisor). The intended runtime for the Aria fleet.
    ClaudeAgentAcp,
    Codex,
    Gemini,
    /// A long-running agent hosted in a Zellij pane. The supervisor talks
    /// to the agent through `zellij action` subprocess invocations against
    /// the named session — there is no PTY owned by mcd. The per-agent
    /// session name lives in `AgentLaunchContext.zellij_session` (not on
    /// this variant, since the runtime impl is a node-wide singleton).
    /// See `mcd-runtimes/src/zellij_hosted.rs`.
    ZellijHosted,
    Custom(String),
}

impl std::fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeKind::ClaudeCode => write!(f, "claude_code"),
            RuntimeKind::ClaudeAgentAcp => write!(f, "claude_agent_acp"),
            RuntimeKind::Codex => write!(f, "codex"),
            RuntimeKind::Gemini => write!(f, "gemini"),
            RuntimeKind::ZellijHosted => write!(f, "zellij_hosted"),
            RuntimeKind::Custom(s) => write!(f, "{s}"),
        }
    }
}

/// How an agent's working/state directory should be allocated when the
/// daemon launches it.
///
/// The agent record carries the *spec* (declarative); the daemon resolves it
/// to a concrete `LaunchContext.work_dir` at launch time. The two variants
/// have different lifecycles:
///
/// - `Persistent` — created once at the given path, lives forever, used by
///   long-running ZellijHosted profile agents and any other agent whose
///   conversation state must survive restarts.
/// - `Ephemeral` — `mkdtemp` a fresh directory per launch, reaped on agent
///   exit (after the optional `ttl_minutes` post-mortem grace period).
///   Used for task-mode agents (Goose batch, Codex one-shot, etc.).
///
/// The reaper that cleans `Ephemeral` dirs lives in mcd (not mcd-core) and
/// is wired in Phase 2 of the daemon-absorption plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateDirSpec {
    Persistent {
        path: std::path::PathBuf,
    },
    Ephemeral {
        /// Minutes to keep the dir alive after agent exit for post-mortem
        /// inspection. `None` falls back to the daemon-global default
        /// (`mcd.reaper.default_ttl_minutes`, 60).
        #[serde(skip_serializing_if = "Option::is_none")]
        ttl_minutes: Option<u32>,
    },
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
    pub mission_id: String,
    pub domain_id: String,
    pub title: String,
    pub description: String,
    pub input_json: String,
    pub required_capabilities: Vec<String>,
    pub produces: serde_json::Value,
    pub consumes: serde_json::Value,
    /// This agent's own profile (name, role, instructions, scope, constraints).
    /// Injected by the daemon before calling inject_task.
    pub agent_profile: Option<serde_json::Value>,
    /// Concise roster of other agents in this domain.
    /// Each entry: {id, name, role, description, scope, capabilities, status, hostname}.
    pub domain_roster: Vec<serde_json::Value>,
    /// Last `phase_finished` summary from each upstream dependency, fetched
    /// by the task loop before inject. Empty when the task has no
    /// dependencies or none have produced a phase_finished event yet.
    #[serde(default)]
    pub dependency_results: Vec<DependencyResult>,
    /// Peer messages that arrived while no task was running. Single-shot
    /// runtimes (claude_code -p, goose run) have no stdin to inject into
    /// once spawned, so the relay buffers PeerMessage signals here and
    /// splices them into the next inject's prompt as `[PENDING MESSAGES]`.
    #[serde(default)]
    pub pending_messages: Vec<PendingPeerMessage>,
}

/// One upstream dependency's terminal summary, ready to splice into the
/// downstream task's prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyResult {
    pub task_id: String,
    pub title: String,
    pub summary: String,
    pub finished_at: String,
}

/// A peer message buffered for delivery on the next task inject.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPeerMessage {
    pub from_agent_id: String,
    pub channel: String,
    pub body: serde_json::Value,
    /// RFC3339 timestamp of when the relay buffered this message.
    pub received_at: String,
}

/// Context passed to `AgentRuntime::launch`.
#[derive(Debug, Clone, Default)]
pub struct LaunchContext {
    pub agent_id: String,
    pub domain_id: String,
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
    /// Concise roster of other agents in the domain.
    /// Injected into every task prompt so the agent can reason about delegation.
    pub roster: Vec<serde_json::Value>,
    /// If true, the runtime should attempt to enable RTK (Rust Token Killer) hooks
    /// for output compression before spawning the agent process.
    pub with_rtk: bool,
    /// Aria vault folder this agent writes to (`operator`, `work`, etc.).
    /// `None` for task agents that have no implicit vault scope.
    /// Injected into the launched process's environment as `ARIA_VAULT_FOLDER`.
    pub vault_folder: Option<String>,
    /// Declarative spec for how `work_dir` should be allocated. `None` means
    /// the caller has already populated `work_dir` and no special lifecycle
    /// handling is needed. When present, `Persistent` resolves to its path
    /// (idempotent `mkdir -p`); `Ephemeral` causes the daemon to `mkdtemp`
    /// before launch and reap after exit.
    pub state_dir_spec: Option<StateDirSpec>,
    /// Name of the Zellij session this agent runs in. Populated by the
    /// daemon from `AgentLaunchContext.zellij_session` only when
    /// `runtime_kind == ZellijHosted`. `None` for all other runtimes.
    pub zellij_session: Option<String>,
}

/// Events emitted by mcd's unit-health (Phase 5 watchdog) loop. Broadcast
/// over a `tokio::sync::broadcast::Sender<SupervisorEvent>` so multiple
/// consumers (mgmt-gateway streaming, future TUI, controlplane web
/// portal) can subscribe without contending.
///
/// `at` is RFC3339 UTC. Wire format is JSON via serde — mgmt-gateway
/// emits newline-delimited JSON frames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SupervisorEvent {
    /// systemctl is-active reported failed/inactive for an agent we
    /// supervise. Emitted once per dead-detection (not every tick).
    UnitDeadDetected {
        agent_id: String,
        source: String,
        systemd_service: String,
        at: String,
    },
    /// mcd issued `systemctl --user restart`. `result` is "started"
    /// (success), "failed" (systemctl non-zero), or "throttled"
    /// (within retry window — no restart actually fired).
    UnitRestarted {
        agent_id: String,
        source: String,
        systemd_service: String,
        reason: String, // "dead" | "nightly" | "manual"
        result: String, // "started" | "failed" | "throttled"
        exit_code: Option<i64>,
        at: String,
    },
    /// Operator paused the supervision loop for this agent via
    /// `mc agent supervise pause`. Auto-restart is suspended until
    /// they `resume`.
    SupervisePaused {
        agent_id: String,
        source: String,
        at: String,
    },
    /// Operator resumed.
    SuperviseResumed {
        agent_id: String,
        source: String,
        at: String,
    },
    /// The configurable nightly restart hour fired for one agent.
    NightlyRestartFired {
        agent_id: String,
        source: String,
        systemd_service: String,
        at: String,
    },
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
/// `resize` requests a TTY size change; bounded — drop on full to coalesce.
pub struct PtySession {
    pub output: tokio::sync::mpsc::Receiver<Vec<u8>>,
    pub input: tokio::sync::mpsc::Sender<Vec<u8>>,
    pub resize: tokio::sync::mpsc::Sender<(u16, u16)>,
    pub rows: u16,
    pub cols: u16,
}

/// Legacy one-directional alias (kept for call sites that only need output).
pub type PtyStream = tokio::sync::mpsc::Receiver<Vec<u8>>;

/// MeshAgent record as returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshAgentRecord {
    pub id: String,
    pub domain_id: String,
    pub runtime_kind: String,
    pub status: String,
    pub current_task_id: Option<String>,
}

/// MeshTask record as returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshTaskRecord {
    pub id: String,
    pub mission_id: String,
    pub domain_id: String,
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
    /// Task IDs this task depends on (must all be finished before this is ready).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Semantic outputs this task declares it will produce.
    #[serde(default)]
    pub produces: serde_json::Value,
    /// Semantic inputs this task requires from prior tasks.
    #[serde(default)]
    pub consumes: serde_json::Value,
}
