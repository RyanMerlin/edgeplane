use crate::{agent_context::AgentContext, schema_pack::SchemaPack};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf, time::Duration};
use thiserror::Error;
use url::Url;

/// Runtime configuration derived from CLI flags and env vars.
#[derive(Clone, Debug)]
pub struct EdgeplaneConfig {
    pub base_url: Url,
    pub token: Option<String>,
    pub timeout: Duration,
    pub allow_insecure: bool,
    pub agent_context: AgentContext,
    pub booster_wasm: Option<PathBuf>,
    pub booster_enabled: bool,
    pub booster_allow_short_circuit: bool,
    pub schema_pack: SchemaPack,
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("invalid base URL: {0}")]
    BaseUrl(#[from] url::ParseError),
    #[error("timeout must be greater than zero")]
    Timeout,
}

impl EdgeplaneConfig {
    pub fn from_parts(
        base_url: &str,
        token: Option<String>,
        agent_id: Option<String>,
        runtime_session_id: Option<String>,
        profile_name: Option<String>,
        timeout_secs: u64,
        allow_insecure: bool,
        booster_enabled: bool,
        booster_allow_short_circuit: bool,
        booster_wasm: Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        if timeout_secs == 0 {
            return Err(ConfigError::Timeout);
        }

        let base_url = Url::parse(base_url)?;
        let resolved_agent_id = resolve_agent_id(agent_id, base_url.as_str());
        Ok(Self {
            base_url,
            token,
            timeout: Duration::from_secs(timeout_secs),
            allow_insecure,
            agent_context: AgentContext::new(
                resolved_agent_id,
                runtime_session_id.filter(|value| !value.trim().is_empty()),
                profile_name.filter(|value| !value.trim().is_empty()),
            ),
            booster_wasm,
            booster_enabled,
            booster_allow_short_circuit,
            schema_pack: SchemaPack::load(),
        })
    }

    pub fn with_allow_insecure(&self, allow: bool) -> Self {
        let mut clone = self.clone();
        clone.allow_insecure = allow;
        clone
    }
}

fn resolve_agent_id(arg: Option<String>, base_url: &str) -> Option<String> {
    arg.filter(|value| !value.trim().is_empty())
        .or_else(read_agent_id_from_disk)
        .or_else(|| default_agent_id_from_session(base_url))
}

pub fn default_agent_id_from_session(base_url: &str) -> Option<String> {
    let session = crate::auth::load_saved_session(base_url)?;
    if let Some(email) = session.email {
        let value = email.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    let subject = session.subject.trim();
    if subject.is_empty() {
        None
    } else {
        Some(subject.to_string())
    }
}

/// Load a session token from `~/.ep/session.json` if one exists,
/// has not expired, and was created for `base_url`.
///
/// Called by `main.rs` when `--token` is absent and no EP_AGENT_TOKEN bootstrap was used.
pub fn load_session_token(base_url: &str) -> Option<String> {
    crate::auth::load_saved_session(base_url).map(|s| s.token)
}

// ── Persistent config file (~/.ep/config.json) ───────────────────

/// User-level persistent settings. Written by `edgeplane auth login`, read at startup.
/// Sensitive values (tokens) are NOT stored here — those live in session.json (chmod 600).
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct SavedConfig {
    /// The last EP_BASE_URL the user authenticated against.
    pub base_url: Option<String>,
}

pub fn config_file_path() -> PathBuf {
    edgeplaned_paths::cli_config_path()
}

pub fn load_saved_config() -> SavedConfig {
    let path = config_file_path();
    let content = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return SavedConfig::default(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save_config(cfg: &SavedConfig) -> std::io::Result<()> {
    let path = config_file_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(cfg).unwrap_or_default();
    fs::write(&path, &json)?;
    // config.json is not secret (no tokens), but still restrict to owner rw
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
    }
    Ok(())
}

pub fn ep_home_dir() -> PathBuf {
    edgeplaned_paths::ep_home_dir()
}

/// Resolve the command used to launch the running `edgeplane` binary, for
/// embedding into an agent's MCP-server config (`command` field).
///
/// Order: the absolute path of the current executable, then `edgeplane` on
/// PATH, then the bare name `edgeplane` (resolved by the agent at spawn time).
/// Centralized here so claude/codex/gemini config writers stay consistent.
pub fn resolve_ep_command() -> String {
    env::current_exe()
        .ok()
        .filter(|p| p.is_file())
        .or_else(|| which::which("edgeplane").ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "edgeplane".to_string())
}

pub fn servers_file_path() -> PathBuf {
    edgeplaned_paths::servers_path()
}

/// Resolution order:
/// 1. EP_SERVERS env var (manual override)
/// 2. ~/.ep/servers file (written by `edgeplane context discover` or edgeplane-tower bootstrap)
/// 3. EP_BASE_URL env var (legacy single-server compat)
/// 4. localhost:8008 (development fallback)
pub fn load_server_list() -> Vec<String> {
    if let Ok(val) = env::var("EP_SERVERS") {
        return parse_server_list(&val);
    }
    let path = servers_file_path();
    if path.exists() {
        if let Ok(contents) = fs::read_to_string(&path) {
            let servers: Vec<String> = contents
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            if !servers.is_empty() {
                return servers;
            }
        }
    }
    if let Ok(val) = env::var("EP_BASE_URL") {
        return vec![val];
    }
    vec!["http://localhost:8008".to_string()]
}

pub fn write_servers_file(servers: &[String]) -> std::io::Result<()> {
    let path = servers_file_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut content = String::from("# Edgeplane server endpoints\n");
    content.push_str("# Auto-generated by `edgeplane context discover` — edit only to override\n");
    for s in servers {
        content.push_str(s);
        content.push('\n');
    }
    fs::write(&path, &content)
}

fn parse_server_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn ensure_mc_dirs() -> std::io::Result<()> {
    fs::create_dir_all(edgeplaned_paths::config_dir())?;
    fs::create_dir_all(edgeplaned_paths::state_dir())?;
    Ok(())
}

pub fn agent_id_file() -> PathBuf {
    edgeplaned_paths::agent_id_path()
}

fn read_agent_id_from_disk() -> Option<String> {
    let path = agent_id_file();
    let contents = fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn persist_agent_id(agent_id: &str) -> std::io::Result<()> {
    let path = agent_id_file();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, format!("{}\n", agent_id))
}

