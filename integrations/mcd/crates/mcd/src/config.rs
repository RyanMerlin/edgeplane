/// mcd daemon configuration.
///
/// Loaded from `/etc/mcd/agent.yaml` or `~/.mc/config.yaml`
/// (whichever exists first). All fields can be overridden by CLI flags.
///
/// Token and backend_url fall back to mc's shared session.json / config.json
/// so mcd and mc stay in sync without duplicating credentials.
use anyhow::{Context, Result};
use mcd_core::paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// MissionControl backend base URL.
    pub backend_url: String,
    /// Bearer token for the backend. Optional — falls back to mc's session.json.
    #[serde(default)]
    pub token: String,
    /// Local directory used as the working root for agent processes.
    #[serde(default = "default_work_dir")]
    pub work_dir: PathBuf,
    /// Missions (and their klusters to watch) this daemon manages.
    #[serde(default)]
    pub missions: Vec<MissionEntry>,
    /// Seconds without a backend response before the offline watchdog triggers.
    #[serde(default = "default_grace")]
    pub offline_grace_secs: u64,
    /// Offline policy: "strict" | "safe_readonly" | "autonomous"
    #[serde(default = "default_policy")]
    pub offline_policy: String,
    /// Unix socket path for the local control interface.
    #[serde(default = "default_socket")]
    pub control_socket: PathBuf,
    /// Runtime node ID assigned by mc-controlplane at registration.
    /// When set, the daemon sends periodic node heartbeats (including Tailscale info).
    #[serde(default)]
    pub node_id: Option<String>,
    /// HMAC secret shared with mc-controlplane, used to validate inbound
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
}

fn default_attach_bind() -> String {
    "0.0.0.0:8009".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionEntry {
    pub mission_id: String,
    /// Agents enrolled in this mission, managed by this daemon.
    #[serde(default)]
    pub agents: Vec<AgentEntry>,
}

/// Whether an agent runs in short-lived task mode (`claude -p` per task) or
/// as a long-running interactive session managed by `session_supervisor`.
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

/// Shared session fields written by `mc auth login`.
#[derive(Deserialize)]
struct McSession {
    token: String,
    base_url: String,
}

/// Shared persistent config written by `mc auth login`.
#[derive(Deserialize)]
struct McConfig {
    base_url: Option<String>,
}

fn read_mc_session() -> Option<McSession> {
    let content = std::fs::read_to_string(paths::session_file_path()).ok()?;
    serde_json::from_str(&content).ok()
}

fn read_mc_base_url() -> Option<String> {
    let content = std::fs::read_to_string(paths::mc_home_dir().join("config.json")).ok()?;
    let cfg: McConfig = serde_json::from_str(&content).ok()?;
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
    PathBuf::from("/run/mcd/control.sock")
}

impl DaemonConfig {
    /// Load from the first config file found, falling back to mc's shared credentials.
    pub fn load_or_default() -> Self {
        let mut cfg = Self::try_load().unwrap_or_else(|| DaemonConfig {
            backend_url: String::new(),
            token: String::new(),
            work_dir: default_work_dir(),
            missions: vec![],
            offline_grace_secs: default_grace(),
            offline_policy: default_policy(),
            control_socket: default_socket(),
            node_id: None,
            attach_secret: None,
            attach_bind_addr: default_attach_bind(),
        });
        cfg.resolve_credentials();
        cfg
    }

    /// Fill in missing token / backend_url from env vars and mc's shared files.
    fn resolve_credentials(&mut self) {
        // Token: env → config.yaml → mc session.json
        if self.token.is_empty() {
            if let Ok(t) = std::env::var("MC_TOKEN") {
                self.token = t;
            } else if let Some(s) = read_mc_session() {
                self.token = s.token;
                // Also pick up base_url from session if not set
                if self.backend_url.is_empty() {
                    self.backend_url = s.base_url;
                }
            }
        }

        // backend_url: env → config.yaml → mc config.json → localhost fallback
        if self.backend_url.is_empty() {
            if let Ok(u) = std::env::var("MC_BASE_URL") {
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
            PathBuf::from("/etc/mcd/agent.yaml"),
            paths::mcd_config_path(),
        ];
        for path in &candidates {
            if path.exists() {
                if let Ok(cfg) = Self::from_path(path) {
                    return Some(cfg);
                }
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
