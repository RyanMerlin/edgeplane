/// edgeplaned daemon configuration.
///
/// Loaded from `/etc/edgeplaned/agent.yaml` or `~/.ep/config.yaml`
/// (whichever exists first). All fields can be overridden by CLI flags.
///
/// Token and backend_url fall back to edgeplane's shared session.json / config.json
/// so edgeplaned and edgeplane stay in sync without duplicating credentials.
use anyhow::{Context, Result};
use edgeplaned_core::paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Edgeplane backend base URL.
    pub backend_url: String,
    /// Bearer token for the backend. Optional — falls back to edgeplane's session.json.
    #[serde(default)]
    pub token: String,
    /// Local directory used as the working root for agent processes.
    #[serde(default = "default_work_dir")]
    pub work_dir: PathBuf,
    /// Domains (and their missions to watch) this daemon manages.
    #[serde(default)]
    pub domains: Vec<DomainEntry>,
    /// Seconds without a backend response before the offline watchdog triggers.
    #[serde(default = "default_grace")]
    pub offline_grace_secs: u64,
    /// Offline policy: "strict" | "safe_readonly" | "autonomous"
    #[serde(default = "default_policy")]
    pub offline_policy: String,
    /// Unix socket path for the local control interface.
    #[serde(default = "default_socket")]
    pub control_socket: PathBuf,
    /// Runtime node ID assigned by edgeplane-tower at registration.
    /// When set, the daemon sends periodic node heartbeats (including Tailscale info).
    #[serde(default)]
    pub node_id: Option<String>,
    /// HMAC secret shared with edgeplane-tower, used to validate inbound
    /// attach-WS connections proxied from the controlplane (Phase 2a/2b).
    /// If `None`, the attach WS server still binds but rejects every
    /// connection — default-deny when no secret is configured.
    #[serde(default)]
    pub attach_secret: Option<String>,
    /// Address the attach-WS server binds to. Default `0.0.0.0:8009`.
    /// Reachable on the Tailscale interface; the controlplane dials this
    /// address using `tailscale_fqdn` from node registration.
    #[serde(default = "default_attach_bind")]
    pub attach_bind_addr: String,
    /// Default domain that persistent agents attach to on daemon startup.
    /// Pushed from edgeplane-tower at `edgeplane daemon profile add` time (via the
    /// `home` field in the node-register response), or set manually.
    /// Ephemeral agents (session_mode: Task) ignore this field entirely.
    #[serde(default)]
    pub home_domain_id: Option<String>,
    // ── Task worker (P2) ───────────────────────────────────────────────────
    //
    // Controls the `task_worker` module that polls for claimable MeshTasks,
    // enrolls ephemeral subagents, spawns `claude -p`, and cleans up on exit.
    // All fields have defaults so existing configs need no changes.

    /// Whether the task worker polling loop is active. Set to `false` to
    /// disable ephemeral subagent spawning on this node (useful during
    /// rollout or debugging). Default: `true`.
    #[serde(default = "default_task_worker_enabled")]
    pub task_worker_enabled: bool,

    /// How often (seconds) the task worker polls for claimable tasks.
    /// Default: 30.
    #[serde(default = "default_task_worker_poll_interval_secs")]
    pub task_worker_poll_interval_secs: u64,

    /// Maximum number of ephemeral subagent processes that may run concurrently
    /// on this node. Tasks beyond this cap remain in `ready` status and are
    /// picked up when a slot frees. Default: 3.
    #[serde(default = "default_task_worker_max_concurrent")]
    pub task_worker_max_concurrent: usize,

    /// Binary name (or full path) used to spawn the `claude -p` subagent.
    /// Override if `claude` is not in PATH or if you want a wrapper script.
    /// Default: `"claude"`.
    #[serde(default = "default_task_worker_subagent_command")]
    pub task_worker_subagent_command: String,

    // ── Task worker triage (P3) ────────────────────────────────────────────
    //
    // Controls the triage loop that examines unscoped tasks in the intake
    // mission and either routes them to a profile (via child meshtask) or
    // marks them as blocked + optionally invokes a deployment-specific
    // surface command to alert a human.

    /// Enable the triage loop (P3). Set to `false` to disable without
    /// disabling the P2 claimer loop. Default: `true`.
    #[serde(default = "default_triage_enabled")]
    pub task_worker_triage_enabled: bool,

    /// How often (seconds) the triage loop polls the intake mission for
    /// unscoped tasks. Deliberately slower than P2's claim interval.
    /// Default: 60.
    #[serde(default = "default_triage_poll_interval_secs")]
    pub task_worker_triage_poll_interval_secs: u64,

    /// Minimum goose confidence score to auto-route a task to a profile.
    /// Tasks below this threshold are marked `blocked` and (optionally)
    /// surfaced via `task_worker_surface_command`. Range: 0.0–1.0. Default: 0.85.
    #[serde(default = "default_triage_confidence_threshold")]
    pub task_worker_triage_confidence_threshold: f64,

    /// Maximum number of tasks to triage per cycle. Caps goose subprocess
    /// load; remaining tasks are picked up on the next poll. Default: 5.
    #[serde(default = "default_max_triage_per_cycle")]
    pub task_worker_max_triage_per_cycle: usize,

    /// Timeout (seconds) for each triage subprocess call during triage.
    /// If it exceeds this, the task is treated as low-confidence and
    /// surfaced for human triage. Default: 30.
    #[serde(default = "default_goose_timeout_secs")]
    pub task_worker_goose_timeout_secs: u64,

    /// Optional command (program + args) invoked when the triage loop blocks
    /// a task for human review. The command receives 3 additional args
    /// appended at the end: `<task_id> <title> <reason>`. Stdout/stderr is
    /// captured to the edgeplaned log; exit code is logged.
    ///
    /// If `None` (default), edgeplaned only marks the task as `blocked` — operators
    /// discover via `edgeplane task ls --status blocked` (MC-native discovery).
    ///
    /// Example deployment that surfaces to an external system:
    /// ```toml
    /// task_worker_surface_command = ["/usr/local/bin/triage-surface.sh"]
    /// # or inline using any notification tool:
    /// task_worker_surface_command = [
    ///   "notify-send", "Triage Required", "--urgency=critical"
    /// ]
    /// ```
    ///
    /// The command should be a deployment-specific concern; MC itself ships
    /// without any default surface so it remains decoupled from Aria, Slack,
    /// vault paths, or any particular human-interface convention.
    #[serde(default)]
    pub task_worker_surface_command: Option<Vec<String>>,

    /// Binary (and optional leading args) used to invoke the goose triage
    /// subprocess. Set to `["goose", "run", "--"]` for a standalone goose
    /// install, or any other command that accepts a prompt as the final
    /// argument and returns `{"ok": bool, "data": {...}}` on stdout.
    ///
    /// `EP_GOOSE_BIN` env var takes precedence when set.
    ///
    /// Default: `["goose"]`
    #[serde(default = "default_goose_bin")]
    pub task_worker_goose_bin: Vec<String>,

    // ── Task worker capability enforcement (P4) ────────────────────────────
    //
    // Controls how `required_capabilities` on a MeshTask are translated into
    // `--allowed-tools` restrictions on the spawned `claude -p` subprocess.

    /// When `true` (strict mode): tasks that declare no `required_capabilities`
    /// are immediately failed with an error — the dispatcher must declare blast
    /// radius. When `false` (default, lenient mode): tasks with no capabilities
    /// fall back to `task_worker_default_capabilities`. Default: `false`.
    #[serde(default = "default_strict_capabilities")]
    pub task_worker_strict_capabilities: bool,

    /// Capability set applied when strict mode is off and a task declares no
    /// `required_capabilities`. Must be valid entries in the v1 capability
    /// vocabulary (`shell:read`, `fs:read`, `fs:write`, etc.). If any entry
    /// is invalid, the spawner logs a warning and falls back to `["fs:read"]`
    /// only. Default: `["fs:read", "shell:read"]`.
    #[serde(default = "default_default_capabilities")]
    pub task_worker_default_capabilities: Vec<String>,
}

fn default_task_worker_enabled() -> bool {
    true
}

fn default_task_worker_poll_interval_secs() -> u64 {
    30
}

fn default_task_worker_max_concurrent() -> usize {
    3
}

fn default_task_worker_subagent_command() -> String {
    "claude".to_string()
}

fn default_triage_enabled() -> bool {
    true
}

fn default_triage_poll_interval_secs() -> u64 {
    60
}

fn default_triage_confidence_threshold() -> f64 {
    0.85
}

fn default_max_triage_per_cycle() -> usize {
    5
}

fn default_goose_timeout_secs() -> u64 {
    30
}

fn default_goose_bin() -> Vec<String> {
    // Respect the legacy env var for backwards-compat, then fall back to
    // the `goose` binary from PATH — no `aria` coupling for OSS installs.
    std::env::var("EP_GOOSE_BIN")
        .map(|v| v.split_whitespace().map(String::from).collect())
        .unwrap_or_else(|_| vec!["goose".to_string()])
}

fn default_strict_capabilities() -> bool {
    false
}

fn default_default_capabilities() -> Vec<String> {
    vec!["fs:read".to_string(), "shell:read".to_string()]
}

fn default_attach_bind() -> String {
    "0.0.0.0:8009".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEntry {
    pub domain_id: String,
    /// Agents enrolled in this domain, managed by this daemon.
    #[serde(default)]
    pub agents: Vec<AgentEntry>,
}

/// Whether an agent runs in short-lived task mode or as a long-running session.
///
/// - `Persistent`: attaches to `home_domain_id` on daemon startup. Can be
///   temporarily reassigned to a working domain. Never starts without a home.
/// - `Task` (ephemeral): only connects when dispatched. MUST arrive with
///   `domain_id`, `mission_id`, and `task_id` all set. Rejected otherwise.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    #[default]
    Task,
    Persistent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    /// The MeshAgent id as assigned by the backend.
    pub agent_id: String,
    /// Runtime kind: claude_code | codex | gemini
    pub runtime_kind: String,
    /// Whether this agent is a short-lived task worker (default) or a
    /// long-running interactive session managed by `session_supervisor`.
    #[serde(default)]
    pub session_mode: SessionMode,
    /// Extra capabilities to expose for task-claim matching, in addition to the
    /// runtime's built-in capability list. Strings are matched against
    /// `TaskSpec.required_capabilities` verbatim.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Optional path to a profile directory used by persistent-session agents
    /// (CLAUDE.md / launch context). Read by future session-supervisor work; no
    /// effect on task-mode agents today.
    #[serde(default)]
    pub profile_path: Option<std::path::PathBuf>,
}

/// Shared persistent config written by `edgeplane auth login`.
#[derive(Deserialize)]
struct EdgeplaneConfig {
    base_url: Option<String>,
}

/// Read the token from the active profile in edgeplaned's own state.json.
/// This is the machine credential stored during `edgeplane daemon profile add`.
fn read_state_profile_token() -> Option<String> {
    let content = std::fs::read_to_string(paths::state_file_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    let active = v.get("active_profile")?.as_str()?;
    let token = v.get("profiles")?.get(active)?.get("auth")?.get("token")?.as_str()?;
    if token.is_empty() { None } else { Some(token.to_string()) }
}

/// Read the node JWT and tower URL from /etc/edgeplane/node.json (written by
/// `edgeplaned register`). Returns (node_jwt, tower_url) if the file exists
/// and can be parsed.
fn read_node_credential() -> Option<(String, String)> {
    let content = std::fs::read_to_string(crate::register::NODE_CREDENTIAL_PATH).ok()?;
    let cred: crate::register::NodeCredential = serde_json::from_str(&content).ok()?;
    if cred.node_jwt.is_empty() { return None; }
    Some((cred.node_jwt, cred.tower_url))
}

fn read_mc_base_url() -> Option<String> {
    let content = std::fs::read_to_string(edgeplaned_paths::cli_config_path()).ok()?;
    let cfg: EdgeplaneConfig = serde_json::from_str(&content).ok()?;
    cfg.base_url
}

fn default_work_dir() -> PathBuf {
    paths::mcd_work_dir()
}

fn default_grace() -> u64 {
    30
}

fn default_policy() -> String {
    "strict".into()
}

fn default_socket() -> PathBuf {
    PathBuf::from("/run/edgeplaned/control.sock")
}

impl DaemonConfig {
    /// Load from the first config file found, falling back to edgeplane's shared credentials.
    pub fn load_or_default() -> Self {
        let mut cfg = Self::try_load().unwrap_or_else(|| DaemonConfig {
            backend_url: String::new(),
            token: String::new(),
            work_dir: default_work_dir(),
            domains: vec![],
            offline_grace_secs: default_grace(),
            offline_policy: default_policy(),
            control_socket: default_socket(),
            node_id: None,
            attach_secret: None,
            attach_bind_addr: default_attach_bind(),
            home_domain_id: None,
            task_worker_enabled: default_task_worker_enabled(),
            task_worker_poll_interval_secs: default_task_worker_poll_interval_secs(),
            task_worker_max_concurrent: default_task_worker_max_concurrent(),
            task_worker_subagent_command: default_task_worker_subagent_command(),
            task_worker_triage_enabled: default_triage_enabled(),
            task_worker_triage_poll_interval_secs: default_triage_poll_interval_secs(),
            task_worker_triage_confidence_threshold: default_triage_confidence_threshold(),
            task_worker_max_triage_per_cycle: default_max_triage_per_cycle(),
            task_worker_goose_timeout_secs: default_goose_timeout_secs(),
            task_worker_strict_capabilities: default_strict_capabilities(),
            task_worker_default_capabilities: default_default_capabilities(),
            task_worker_surface_command: None,
            task_worker_goose_bin: default_goose_bin(),
        });
        cfg.resolve_credentials();
        cfg
    }

    /// Fill in missing token / backend_url from stored credentials.
    ///
    /// Priority: config.yaml → /etc/edgeplane/node.json (node JWT) → active profile in state.json
    /// EP_TOKEN env is intentionally not read — that variable has been removed.
    fn resolve_credentials(&mut self) {
        if self.token.is_empty() {
            // Prefer the RS256 node JWT written by `edgeplaned register`.
            if let Some((jwt, tower_url)) = read_node_credential() {
                self.token = jwt;
                if self.backend_url.is_empty() {
                    self.backend_url = tower_url;
                }
            } else if let Some(t) = read_state_profile_token() {
                self.token = t;
            }
        }

        // backend_url: config.yaml → EP_BASE_URL env → edgeplane config.json → localhost fallback
        if self.backend_url.is_empty() {
            if let Ok(u) = std::env::var("EP_BASE_URL") {
                self.backend_url = u;
            } else if let Some(u) = read_mc_base_url() {
                self.backend_url = u;
            } else {
                self.backend_url = "http://localhost:8008".into();
            }
        }
    }

    fn try_load() -> Option<Self> {
        let candidates = [
            PathBuf::from("/etc/edgeplaned/agent.yaml"),
            paths::mcd_config_path(),
        ];
        for path in &candidates {
            if path.exists()
                && let Ok(cfg) = Self::from_path(path) {
                    return Some(cfg);
                }
        }
        None
    }

    pub fn from_path(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_yaml::from_str(&text)
            .with_context(|| format!("parsing {}", path.display()))
    }

    #[allow(dead_code)]
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_yaml::to_string(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn user_config_path() -> PathBuf {
        paths::mcd_config_path()
    }
}
