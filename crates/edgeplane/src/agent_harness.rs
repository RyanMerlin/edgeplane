//! `edgeplane launch` — one-shot agent launcher.
//!
//! Ensures the daemon is running, validates auth, generates and installs agent
//! config, then exec's the agent binary. Adding a new agent type requires only
//! a new `AgentDriver` impl + a `AgentKind` variant — no changes to `run()`.
//!
//! ## Token embedding
//!
//! By default `edgeplane launch` embeds the `EP_AGENT_TOKEN` value into the agent's config
//! file. This is convenient for static long-lived tokens.
//!
//! For OIDC / short-lived JWTs, use `--no-embed-token`: the token is omitted
//! from the written config and the agent process inherits `EP_AGENT_TOKEN` from the
//! shell environment at exec time. The MCP shim always prefers the env var over
//! any embedded value, so the agent will pick up refreshed tokens automatically
//! on every launch.
//!
//! Auto-detection: if `EP_AGENT_TOKEN` / `config.token` is absent, `--no-embed-token`
//! is implied and a notice is printed.

use crate::{
    auth,
    client::EdgeplaneClient,
    config::{EdgeplaneConfig, ep_home_dir},
    ep_info, ep_ok, ep_warn, ui,
};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Launch-shaping options for [`run_driver_agent`]. `edgeplane run` passes
/// `DriverOpts::default()`; finer-grained knobs stay available for callers that
/// need them (CI preflight, legacy global config, etc.).
#[derive(Debug, Default)]
pub struct DriverOpts {
    pub preflight_only: bool,
    pub skip_config_gen: bool,
    pub legacy_global_config: bool,
    pub allow_pin_mismatch: bool,
    pub no_embed_token: bool,
}

#[derive(Debug, Clone)]
enum AgentKind {
    Gemini,
    Openclaw,
    Custom,
}

impl AgentKind {
    fn driver(&self) -> Box<dyn AgentDriver> {
        match self {
            AgentKind::Gemini => Box::new(GeminiDriver),
            AgentKind::Openclaw => Box::new(OpenClawDriver),
            AgentKind::Custom => Box::new(CustomDriver),
        }
    }

    fn config_key(&self) -> &str {
        match self {
            AgentKind::Gemini => "gemini",
            AgentKind::Openclaw => "openclaw",
            AgentKind::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LaunchSessionRecord {
    pub(crate) runtime_session_id: String,
    pub(crate) agent: String,
    pub(crate) profile: String,
    pub(crate) instance_home: String,
    pub(crate) created_at: String,
}

// ── AgentDriver trait ────────────────────────────────────────────────────────

trait AgentDriver {
    /// Agent binary name (looked up on PATH).
    fn binary(&self) -> &str;
    /// Human-readable install hint shown when binary is missing.
    fn install_hint(&self) -> &str;
    /// Write rendered config to the agent's canonical location.
    ///
    /// `embed_token`: when false, omit `EP_AGENT_TOKEN` from the written config
    /// entirely; the agent process must inherit it from the environment.
    fn install_config(
        &self,
        staging_dir: &Path,
        base_url: &str,
        token: &str,
        embed_token: bool,
        target_home: &Path,
        target_mc_home: &Path,
    ) -> Result<()>;
    /// Build the Command to exec (binary + required flags).
    fn command(&self, extra_args: &[String], target_mc_home: &Path) -> std::process::Command;
}

// ── GeminiDriver ─────────────────────────────────────────────────────────────

struct GeminiDriver;

impl AgentDriver for GeminiDriver {
    fn binary(&self) -> &str {
        "gemini"
    }

    fn install_hint(&self) -> &str {
        "npm install -g @google/gemini-cli"
    }

    fn install_config(
        &self,
        _staging_dir: &Path,
        base_url: &str,
        token: &str,
        embed_token: bool,
        target_home: &Path,
        _target_mc_home: &Path,
    ) -> Result<()> {
        let config_path = target_home.join(".gemini").join("settings.json");

        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut root: serde_json::Value = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            serde_json::from_str(&content)
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
        } else {
            serde_json::Value::Object(Default::default())
        };

        let ep_entry = render_json_mcp_entry(
            include_str!("../../../distribution/templates/gemini.mcp.json.tmpl"),
            "embedded gemini template",
            base_url,
            token,
            embed_token,
        );
        let ep_entry = absolutize_mc_command(ep_entry);

        root.as_object_mut()
            .ok_or_else(|| anyhow!("~/.gemini/settings.json is not a JSON object"))?
            .entry("mcpServers")
            .or_insert_with(|| serde_json::Value::Object(Default::default()))
            .as_object_mut()
            .ok_or_else(|| anyhow!("~/.gemini/settings.json mcpServers is not an object"))?
            .insert("edgeplane".to_string(), ep_entry.clone());

        std::fs::write(&config_path, serde_json::to_string_pretty(&root)?)?;
        ep_ok!("gemini MCP config written → {}", config_path.display());
        if let Err(e) = seed_gemini_auth_state(target_home) {
            ep_warn!("could not seed Gemini auth state: {}", e);
        }
        if let Err(e) = write_gemini_project_config(&ep_entry) {
            ep_warn!("could not write project Gemini MCP config: {}", e);
        }
        Ok(())
    }

    fn command(&self, extra_args: &[String], _target_mc_home: &Path) -> std::process::Command {
        let mut cmd = resolved_command("gemini");
        cmd.args(extra_args);
        cmd
    }
}

fn write_gemini_project_config(ep_entry: &serde_json::Value) -> Result<()> {
    let current_dir = std::env::current_dir().context("unable to locate current directory")?;
    let config_path = current_dir.join(".gemini").join("settings.json");
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut root: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&content)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
    } else {
        serde_json::Value::Object(Default::default())
    };
    let project_entry = {
        let mut entry = ep_entry.clone();
        if let Some(obj) = entry.as_object_mut() {
            obj.remove("env");
        }
        entry
    };
    root.as_object_mut()
        .ok_or_else(|| anyhow!("{} is not a JSON object", config_path.display()))?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(Default::default()))
        .as_object_mut()
        .ok_or_else(|| anyhow!("{} mcpServers is not an object", config_path.display()))?
        .insert("edgeplane".to_string(), project_entry);
    std::fs::write(&config_path, serde_json::to_string_pretty(&root)?)?;
    ep_ok!(
        "gemini project MCP config written → {}",
        config_path.display()
    );
    Ok(())
}

// ── Shared JSON render helper ─────────────────────────────────────────────────

/// Render the `mcpServers.edgeplane` entry from a template.
///
/// Replaces `__BASE_URL__` and (if `embed_token`) `__TOKEN__` in the template,
/// then returns the inner `mcpServers.edgeplane` object.  When
/// `embed_token` is false the `EP_AGENT_TOKEN` key is removed from the `env` map.
fn render_json_mcp_entry(
    tmpl: &str,
    tmpl_name: &str,
    base_url: &str,
    token: &str,
    embed_token: bool,
) -> serde_json::Value {
    let rendered = tmpl
        .replace("__BASE_URL__", base_url)
        .replace("__TOKEN__", token);
    let mut full: serde_json::Value = serde_json::from_str(&rendered)
        .unwrap_or_else(|_| panic!("{} must be valid JSON", tmpl_name));
    if !embed_token {
        if let Some(env_obj) = full
            .pointer_mut("/mcpServers/edgeplane/env")
            .and_then(|v| v.as_object_mut())
        {
            env_obj.remove("EP_AGENT_TOKEN");
        }
    }
    full["mcpServers"]["edgeplane"].clone()
}

fn absolutize_mc_command(mut entry: serde_json::Value) -> serde_json::Value {
    let cmd = entry
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if cmd != "edgeplane" {
        return entry;
    }
    if let Some(obj) = entry.as_object_mut() {
        obj.insert(
            "command".to_string(),
            serde_json::Value::String(crate::config::resolve_ep_command()),
        );
    }
    entry
}

// ── OpenClawDriver / CustomDriver ────────────────────────────────────────────

struct OpenClawDriver;
struct CustomDriver;

impl AgentDriver for OpenClawDriver {
    fn binary(&self) -> &str {
        "openclaw"
    }

    fn install_hint(&self) -> &str {
        "See Edgeplane docs for OpenClaw installation"
    }

    fn install_config(
        &self,
        _staging_dir: &Path,
        base_url: &str,
        token: &str,
        embed_token: bool,
        _target_home: &Path,
        target_mc_home: &Path,
    ) -> Result<()> {
        install_acp_config("openclaw", base_url, token, embed_token, target_mc_home)
    }

    fn command(&self, extra_args: &[String], target_mc_home: &Path) -> std::process::Command {
        let config = target_mc_home.join("config").join("openclaw.acp.json");
        let mut cmd = resolved_command("openclaw");
        cmd.arg("--acp-config").arg(config);
        cmd.args(extra_args);
        cmd
    }
}

impl AgentDriver for CustomDriver {
    fn binary(&self) -> &str {
        "custom"
    }

    fn install_hint(&self) -> &str {
        "See Edgeplane docs for custom ACP-compatible agent installation"
    }

    fn install_config(
        &self,
        _staging_dir: &Path,
        base_url: &str,
        token: &str,
        embed_token: bool,
        _target_home: &Path,
        target_mc_home: &Path,
    ) -> Result<()> {
        install_acp_config("custom", base_url, token, embed_token, target_mc_home)
    }

    fn command(&self, extra_args: &[String], target_mc_home: &Path) -> std::process::Command {
        let config = target_mc_home.join("config").join("custom.acp.json");
        let mut cmd = resolved_command("custom");
        cmd.arg("--acp-config").arg(config);
        cmd.args(extra_args);
        cmd
    }
}

fn install_acp_config(
    name: &str,
    base_url: &str,
    token: &str,
    embed_token: bool,
    target_mc_home: &Path,
) -> Result<()> {
    let config_dir = target_mc_home.join("config");
    std::fs::create_dir_all(&config_dir)?;
    let out = config_dir.join(format!("{}.acp.json", name));
    let mut config = serde_json::json!({
        "ep_base_url": base_url,
    });
    if embed_token {
        config["ep_token"] = serde_json::json!(token);
    }
    // When not embedding, ep_token is intentionally absent; the ACP client
    // must read EP_AGENT_TOKEN from the process environment at runtime.
    std::fs::write(&out, serde_json::to_string_pretty(&config)?)?;
    ep_ok!("ACP config written → {}", out.display());
    Ok(())
}

// ── Orchestration ─────────────────────────────────────────────────────────────

/// Launch a driver-based agent (gemini, openclaw, custom) with a fully wired
/// Edgeplane harness: instance isolation, profile overlay, MCP config + auth,
/// onboarding-manifest staging, and exec. Backs `edgeplane run <runtime>` for
/// the driver runtimes (claude/codex/goose have their own native modules).
pub async fn run_driver_agent(
    runtime: &str,
    profile: Option<String>,
    passthrough: Vec<String>,
    opts: DriverOpts,
    client: &EdgeplaneClient,
    config: &EdgeplaneConfig,
) -> Result<()> {
    let selected_agent = parse_agent_kind(runtime)?;
    let base_mc_home = ep_home_dir();
    fs::create_dir_all(&base_mc_home)?;

    let profile_name =
        resolve_profile_name(&profile, Some(selected_agent.config_key()), client)
            .await
            .unwrap_or_else(|_| "default".to_string());

    let runtime_session_id = format!("rs_{}", Uuid::new_v4().simple());
    let instance_home = base_mc_home.join("instances").join(&runtime_session_id);
    let profile_home = base_mc_home.join("profiles").join(&profile_name);
    fs::create_dir_all(&instance_home)?;
    fs::create_dir_all(&profile_home)?;

    let agent_home = instance_home.join("home");
    fs::create_dir_all(&agent_home)?;
    let instance_mc_home = instance_home.join("edgeplane");
    fs::create_dir_all(&instance_mc_home)?;
    persist_runtime_context(
        &instance_home,
        &runtime_session_id,
        selected_agent.config_key(),
        &profile_name,
        &profile_home,
    )?;
    if let Err(e) = write_mc_context_json(
        &instance_mc_home,
        config.base_url.as_str(),
        &profile_name,
        &runtime_session_id,
    ) {
        ep_warn!("could not write edgeplane/context.json: {}", e);
    }
    upsert_launch_session(
        &base_mc_home,
        LaunchSessionRecord {
            runtime_session_id: runtime_session_id.clone(),
            agent: selected_agent.config_key().to_string(),
            profile: profile_name.clone(),
            instance_home: instance_home.display().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )?;

    let driver = selected_agent.driver();

    // 1. Verify binary is on PATH before doing anything else.
    check_binary(driver.as_ref())?;

    // Print brand banner after confirming the binary exists.
    ui::print_banner(
        config.base_url.as_str(),
        selected_agent.config_key(),
        env!("CARGO_PKG_VERSION"),
    );

    // 2. (Daemon lifecycle removed — edgeplane serve connects directly to backend.)

    // 3. Auth: verify we have a valid session or static token; run interactive
    //    login if neither is available.  Falls through immediately when EP_AGENT_TOKEN
    //    is already set (static token path).
    let login_client_holder: Option<EdgeplaneClient> = if config.token.is_none() {
        if auth::load_saved_session(config.base_url.as_str()).is_none() {
            ep_warn!("no valid session found for {}", config.base_url.as_str());
            ep_info!("running `edgeplane auth login` to authenticate...");
            auth::login(
                auth::LoginArgs {
                    ttl_hours: 8,
                    print_token: false,
                    non_interactive: false,
                    with_token: false,
                },
                client,
                config.base_url.as_str(),
            )
            .await
            .context("login failed — cannot launch without authentication")?;
        }
        // Rebuild client with the freshly written (or pre-existing) session token.
        let session_token = auth::load_saved_session(config.base_url.as_str())
            .map(|s| s.token)
            .context("session not found after login — run `edgeplane auth login` manually")?;
        Some(
            EdgeplaneClient::new_with_token(config.base_url.as_str(), &session_token)
                .context("failed to build client with session token")?,
        )
    } else {
        None
    };
    let effective_client: &EdgeplaneClient = login_client_holder.as_ref().unwrap_or(client);
    enforce_profile_pin(effective_client, &profile_name, opts.allow_pin_mismatch).await?;

    // 4. Preflight-only mode: verify connectivity then stop.
    if opts.preflight_only {
        effective_client
            .get_json("/mcp/health")
            .await
            .context("auth preflight failed — check EP_AGENT_TOKEN and EP_BASE_URL")?;
        ep_ok!("preflight passed");
        return Ok(());
    }

    let base_url = config.base_url.as_str().trim_end_matches('/').to_string();
    // Effective token: static config token, or session token from disk (after login).
    let token = config.token.clone().unwrap_or_else(|| {
        auth::load_saved_session(config.base_url.as_str())
            .map(|s| s.token)
            .unwrap_or_default()
    });

    // 5. Resolve token-embedding mode.
    //
    //    Precedence (highest first):
    //      a) --no-embed-token flag  → never embed
    //      b) token is empty         → cannot embed; auto-implies no-embed with notice
    //      c) default                → embed
    let embed_token = resolve_embed_token(opts.no_embed_token, &token);

    let staging_dir = instance_mc_home.join("config");
    std::fs::create_dir_all(&staging_dir)?;

    // 6. Fetch agent config from onboarding manifest and write to staging dir.
    if !opts.skip_config_gen {
        fetch_and_stage_agent_config(
            effective_client,
            &selected_agent,
            &staging_dir,
            &base_url,
            &token,
        )
        .await?;
    }

    // 7. Install config in instance-local paths by default.
    let config_target_home = if opts.legacy_global_config {
        dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?
    } else {
        initialize_profile_overlay(
            &selected_agent,
            &agent_home,
            &profile_home,
            &dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?,
        )?;
        agent_home.clone()
    };
    // SAFETY: single-threaded at this point; env is set immediately before exec.
    unsafe {
        std::env::set_var("EP_HOME", &instance_mc_home);
        std::env::set_var("EP_AGENT_PROFILE", &profile_name);
        std::env::set_var("EP_RUNTIME_SESSION_ID", &runtime_session_id);
        std::env::set_var("EP_INSTANCE_HOME", &instance_home);
    }
    let launch_agent_base = config
        .agent_context
        .agent_id
        .clone()
        .or_else(|| crate::config::default_agent_id_from_session(config.base_url.as_str()))
        .unwrap_or_else(|| format!("edgeplane-agent-{}", Uuid::new_v4().simple()));
    let launch_agent_id = format!(
        "{}:{}:{}",
        launch_agent_base,
        selected_agent.config_key(),
        &runtime_session_id[..12.min(runtime_session_id.len())]
    );
    // SAFETY: single-threaded at this point; env is set immediately before exec.
    unsafe { std::env::set_var("EP_AGENT_ID", &launch_agent_id) };
    driver.install_config(
        &staging_dir,
        &base_url,
        &token,
        embed_token,
        &config_target_home,
        &instance_mc_home,
    )?;
    // 7b. MCP connectivity preflight — verify backend is reachable and tools
    //     are available before handing off to the agent. A failure here is
    //     non-fatal (warn only): the MCP server's retry loop will recover if
    //     the backend comes up after the agent starts.
    mcp_connectivity_preflight(effective_client).await;

    // 8. Exec the agent (replaces the current process on Unix).
    //    Always inject EP_AGENT_TOKEN into the agent environment so the MCP shim can
    //    authenticate even when the token was NOT embedded in the config file.
    exec_agent(
        driver.as_ref(),
        &passthrough,
        &token,
        &runtime_session_id,
        &instance_home,
        &agent_home,
        &instance_mc_home,
        &profile_name,
    )
}

/// Verify MCP backend connectivity and tool availability before exec.
///
/// Non-fatal: prints status either way. The MCP server's retry loop handles
/// recovery if the backend is temporarily unavailable at agent start time.
async fn mcp_connectivity_preflight(client: &EdgeplaneClient) {
    let mut tools_count: usize = 0;
    // Health check.
    match client.get_json("/mcp/health").await {
        Err(e) => {
            ep_warn!("MCP preflight: backend unreachable ({})", e);
            ep_warn!(
                "MCP preflight: tools will load once backend is available (retry loop active)"
            );
            return;
        }
        Ok(_) => {}
    }

    // Tools count.
    match client.get_json("/mcp/tools").await {
        Ok(resp) => {
            let count = match &resp {
                serde_json::Value::Array(arr) => arr.len(),
                serde_json::Value::Object(obj) => obj
                    .get("tools")
                    .and_then(|t| t.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0),
                _ => 0,
            };
            if count > 0 {
                ep_ok!(
                    "MCP preflight: backend reachable, {} tools available",
                    count
                );
                tools_count = count;
            } else {
                ep_warn!("MCP preflight: backend reachable but returned 0 tools");
            }
        }
        Err(e) => {
            ep_warn!("MCP preflight: tools fetch failed ({})", e);
        }
    }

    eprintln!();
    eprintln!(
        "{}{}Launch Preflight Summary{}",
        ui::BOLD,
        ui::ORANGE,
        ui::RESET
    );
    eprintln!("{}{}{}", ui::GRAY, "─".repeat(56), ui::RESET);
    eprintln!(
        "  {}Backend:{} {}{}{}",
        ui::DIM,
        ui::RESET,
        ui::GREEN,
        "reachable",
        ui::RESET
    );
    eprintln!(
        "  {}Tools:{} {}{}{}",
        ui::DIM,
        ui::RESET,
        if tools_count > 0 {
            ui::CYAN
        } else {
            ui::YELLOW
        },
        tools_count,
        ui::RESET
    );
}

/// Determine whether to embed `EP_AGENT_TOKEN` into the written agent config.
///
/// Precedence (highest → lowest):
///   1. `--no-embed-token` flag → never embed
///   2. Token is an `mcs_` session token → never embed (managed by edgeplane, injected at exec)
///   3. Token is empty → cannot embed; auto-implies no-embed
///   4. Default → embed
fn resolve_embed_token(no_embed_flag: bool, token: &str) -> bool {
    if no_embed_flag {
        ep_info!("--no-embed-token: EP_AGENT_TOKEN will NOT be written to agent config");
        ep_info!("token will be injected into the agent process at exec time");
        return false;
    }
    if crate::auth::is_session_token(token) {
        ep_info!("session token (mcs_*) detected — will NOT be embedded in agent config");
        ep_info!("session token will be injected into the agent process at exec time");
        return false;
    }
    if token.is_empty() {
        ep_warn!("EP_AGENT_TOKEN is not set — implying --no-embed-token");
        ep_warn!("ensure EP_AGENT_TOKEN is present in the environment when the agent runs");
        return false;
    }
    true
}

fn managed_config_relpaths(agent: &AgentKind) -> &'static [&'static str] {
    match agent {
        AgentKind::Gemini => &[".gemini/settings.json"],
        _ => &[],
    }
}

fn initialize_profile_overlay(
    agent: &AgentKind,
    agent_home: &Path,
    profile_home: &Path,
    global_home: &Path,
) -> Result<()> {
    for rel in managed_config_relpaths(agent) {
        let profile_path = profile_home.join(rel);
        let global_path = global_home.join(rel);
        if global_path.exists() {
            if should_force_profile_refresh(rel) {
                seed_profile_path(&global_path, &profile_path)?;
                ep_info!(
                    "refreshed profile auth from global {}",
                    global_path.display()
                );
            } else if !profile_path.exists() {
                seed_profile_path(&global_path, &profile_path)?;
                ep_info!(
                    "seeded profile config from global {}",
                    global_path.display()
                );
            } else if global_path.is_dir() && profile_path.is_dir() {
                let copied = merge_missing_dir_entries(&global_path, &profile_path)?;
                if copied > 0 {
                    ep_info!(
                        "merged {} missing profile entries from global {}",
                        copied,
                        global_path.display()
                    );
                }
            }
        }

        let instance_path = agent_home.join(rel);
        if let Some(parent) = instance_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if instance_path.exists() {
            let meta = fs::symlink_metadata(&instance_path)?;
            if meta.file_type().is_symlink() || meta.is_file() {
                fs::remove_file(&instance_path)?;
            } else if meta.is_dir() {
                fs::remove_dir_all(&instance_path)?;
            }
        }

        if let Some(parent) = profile_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !profile_path.exists() {
            if rel.ends_with('/') || *rel == ".claude" {
                fs::create_dir_all(&profile_path)?;
            } else {
                fs::write(&profile_path, "")?;
            }
        }

        #[cfg(unix)]
        {
            unix_fs::symlink(&profile_path, &instance_path).with_context(|| {
                format!(
                    "failed to link instance config {} -> {}",
                    instance_path.display(),
                    profile_path.display()
                )
            })?;
        }
        #[cfg(not(unix))]
        {
            fs::copy(&profile_path, &instance_path)?;
        }
    }
    Ok(())
}

fn should_force_profile_refresh(rel: &str) -> bool {
    matches!(
        rel,
        ".claude.json" | ".claude/.credentials.json" | ".claude/settings.json"
    )
}

fn seed_gemini_auth_state(target_home: &Path) -> Result<()> {
    let Some(global_home) = dirs::home_dir() else {
        return Ok(());
    };
    let global_gemini = global_home.join(".gemini");
    if !global_gemini.exists() {
        return Ok(());
    }

    let target_gemini = target_home.join(".gemini");
    std::fs::create_dir_all(&target_gemini)?;

    for name in [
        "oauth_creds.json",
        "google_accounts.json",
        "state.json",
        "trustedFolders.json",
        "projects.json",
        "installation_id",
    ] {
        let src = global_gemini.join(name);
        if !src.exists() {
            continue;
        }
        let dst = target_gemini.join(name);
        std::fs::copy(&src, &dst).with_context(|| {
            format!(
                "failed to seed Gemini auth file from {} to {}",
                src.display(),
                dst.display()
            )
        })?;
    }

    Ok(())
}

fn seed_profile_path(global_path: &Path, profile_path: &Path) -> Result<()> {
    if let Some(parent) = profile_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if global_path.is_dir() {
        copy_dir_recursive(global_path, profile_path).with_context(|| {
            format!(
                "failed to seed profile directory from {} to {}",
                global_path.display(),
                profile_path.display()
            )
        })?;
    } else {
        fs::copy(global_path, profile_path).with_context(|| {
            format!(
                "failed to seed profile config from {} to {}",
                global_path.display(),
                profile_path.display()
            )
        })?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            let link_target = fs::read_link(&path)?;
            #[cfg(unix)]
            unix_fs::symlink(link_target, &target)?;
            #[cfg(not(unix))]
            {
                if path.is_dir() {
                    fs::create_dir_all(&target)?;
                } else {
                    fs::copy(&path, &target)?;
                }
            }
        } else if meta.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn merge_missing_dir_entries(src: &Path, dst: &Path) -> Result<usize> {
    let mut copied: usize = 0;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let meta = fs::symlink_metadata(&src_path)?;
        if meta.is_dir() {
            if !dst_path.exists() {
                copy_dir_recursive(&src_path, &dst_path)?;
                copied += 1;
            } else if dst_path.is_dir() {
                copied += merge_missing_dir_entries(&src_path, &dst_path)?;
            }
            continue;
        }
        if !dst_path.exists() {
            if meta.file_type().is_symlink() {
                let link_target = fs::read_link(&src_path)?;
                #[cfg(unix)]
                unix_fs::symlink(link_target, &dst_path)?;
                #[cfg(not(unix))]
                {
                    fs::copy(&src_path, &dst_path)?;
                }
            } else {
                fs::copy(&src_path, &dst_path)?;
            }
            copied += 1;
        }
    }
    Ok(copied)
}

fn parse_agent_kind(value: &str) -> Result<AgentKind> {
    match value.trim().to_lowercase().as_str() {
        "gemini" => Ok(AgentKind::Gemini),
        "openclaw" => Ok(AgentKind::Openclaw),
        "custom" => Ok(AgentKind::Custom),
        other => bail!(
            "`{}` is not a driver-managed runtime; expected gemini, openclaw, or custom \
             (claude/codex/goose are native runtimes handled by `edgeplane run` directly)",
            other
        ),
    }
}

async fn resolve_profile_name(
    requested: &Option<String>,
    _agent_key: Option<&str>,
    _client: &EdgeplaneClient,
) -> Result<String> {
    if let Some(profile) = requested {
        return Ok(profile.trim().to_string());
    }
    Ok("default".to_string())
}

fn session_index_path(base_mc_home: &Path) -> PathBuf {
    base_mc_home.join("sessions").join("launch-index.jsonl")
}

pub(crate) fn sessions_for_profile(profile: &str) -> Vec<LaunchSessionRecord> {
    read_launch_sessions(&ep_home_dir())
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.profile == profile)
        .collect()
}

fn read_launch_sessions(base_mc_home: &Path) -> Result<Vec<LaunchSessionRecord>> {
    let path = session_index_path(base_mc_home);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<LaunchSessionRecord>(line) {
            out.push(record);
        }
    }
    Ok(out)
}

fn upsert_launch_session(base_mc_home: &Path, record: LaunchSessionRecord) -> Result<()> {
    let mut sessions = read_launch_sessions(base_mc_home)?;
    sessions.retain(|s| s.runtime_session_id != record.runtime_session_id);
    sessions.push(record);
    let sessions_dir = base_mc_home.join("sessions");
    fs::create_dir_all(&sessions_dir)?;
    let body = sessions
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    fs::write(session_index_path(base_mc_home), format!("{}\n", body))?;
    Ok(())
}

/// Write (or refresh) `$EP_INSTANCE_HOME/edgeplane/context.json` with the current
/// agent context. Called at launch and patched live by the MCP server after
/// tool calls that return domain/mission IDs.
///
/// The file is read by the PreCompact and SessionStart(compact) hook scripts
/// to re-inject domain context into Claude's window after compaction.
pub fn write_mc_context_json(
    instance_mc_home: &Path,
    base_url: &str,
    active_profile: &str,
    runtime_session_id: &str,
) -> Result<()> {
    // Load existing file so we can preserve active_domain_id / active_mission_id
    // written by the MCP server between launch invocations.
    let existing: Value = if instance_mc_home.join("context.json").exists() {
        let raw = fs::read_to_string(instance_mc_home.join("context.json"))?;
        serde_json::from_str(&raw).unwrap_or(json!({}))
    } else {
        json!({})
    };

    let payload = json!({
        "runtime_session_id": runtime_session_id,
        "base_url": base_url,
        "active_profile": active_profile,
        "active_domain_id": existing.get("active_domain_id").cloned().unwrap_or(Value::Null),
        "active_mission_id": existing.get("active_mission_id").cloned().unwrap_or(Value::Null),
        "last_sync_at": chrono::Utc::now().to_rfc3339(),
    });

    fs::create_dir_all(instance_mc_home)?;
    fs::write(
        instance_mc_home.join("context.json"),
        serde_json::to_string_pretty(&payload)?,
    )?;
    Ok(())
}

fn persist_runtime_context(
    instance_home: &Path,
    runtime_session_id: &str,
    agent: &str,
    profile: &str,
    profile_home: &Path,
) -> Result<()> {
    let payload = serde_json::json!({
        "runtime_session_id": runtime_session_id,
        "agent": agent,
        "profile": profile,
        "profile_home": profile_home.display().to_string(),
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    fs::write(
        instance_home.join("runtime-context.json"),
        serde_json::to_string_pretty(&payload)?,
    )?;
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn check_binary(driver: &dyn AgentDriver) -> Result<()> {
    let binary = driver.binary();
    which_binary(binary)
        .with_context(|| {
            format!(
                "agent binary '{}' not found on PATH\nInstall with: {}",
                binary,
                driver.install_hint()
            )
        })
        .map(|_| ())
}

fn which_binary(name: &str) -> Result<PathBuf> {
    which::which(name).map_err(|_| anyhow!("not found on PATH"))
}

fn resolved_command(name: &str) -> std::process::Command {
    let binary = which_binary(name).unwrap_or_else(|_| PathBuf::from(name));
    std::process::Command::new(binary)
}

async fn fetch_and_stage_agent_config(
    client: &EdgeplaneClient,
    agent: &AgentKind,
    staging_dir: &Path,
    base_url: &str,
    _token: &str,
) -> Result<()> {
    let encoded_url: String = url::form_urlencoded::byte_serialize(base_url.as_bytes()).collect();
    let manifest = client
        .get_json(&format!("/agent-onboarding.json?endpoint={}", encoded_url))
        .await
        .context("failed to fetch agent onboarding manifest")?;

    let config_key = agent.config_key();
    if let Some(agent_cfg) = manifest
        .get("agent_configs")
        .and_then(|c| c.get(config_key))
    {
        let out_path = staging_dir.join(format!("{}.manifest.json", config_key));
        std::fs::write(&out_path, serde_json::to_string_pretty(agent_cfg)?)?;
        ep_info!("manifest staged → {}", out_path.display());
    } else {
        ep_warn!(
            "no agent_configs.{} in manifest — using embedded template",
            config_key
        );
    }
    Ok(())
}

fn exec_agent(
    driver: &dyn AgentDriver,
    extra_args: &[String],
    token: &str,
    runtime_session_id: &str,
    instance_home: &Path,
    agent_home: &Path,
    instance_mc_home: &Path,
    profile_name: &str,
) -> Result<()> {
    let binary_name = driver.binary().to_string();
    let mut cmd = driver.command(extra_args, instance_mc_home);

    // Always inject EP_AGENT_TOKEN into the agent's process environment. This ensures
    // the MCP shim can authenticate regardless of whether the token was embedded
    // in the config file — covering session tokens, --no-embed-token, and the
    // standard embedded-token path uniformly. EP_AGENT_TOKEN is also the env var
    // Claude Code native hooks (SessionStart, SessionEnd, PostToolUse) read; it is
    // listed in `allowedEnvVars` so Claude Code forwards it in HTTP hook
    // Authorization headers.
    if !token.is_empty() {
        cmd.env("EP_AGENT_TOKEN", token);
    }
    cmd.env("HOME", agent_home);
    #[cfg(windows)]
    {
        cmd.env("USERPROFILE", agent_home);
    }

    // Claude Code checks that $HOME/.local/bin is in PATH to confirm its install
    // method. Since we override HOME to the isolated instance home, prepend the
    // instance's .local/bin so that check passes.
    let instance_local_bin = agent_home.join(".local").join("bin");
    if let Some(current_path) = std::env::var_os("PATH") {
        let new_path = std::env::join_paths(
            std::iter::once(instance_local_bin.clone()).chain(std::env::split_paths(&current_path)),
        )
        .unwrap_or(current_path);
        cmd.env("PATH", new_path);
    }

    cmd.env("EP_HOME", instance_mc_home);
    cmd.env("EP_RUNTIME_SESSION_ID", runtime_session_id);
    cmd.env("EP_INSTANCE_HOME", instance_home);
    cmd.env("EP_AGENT_PROFILE", profile_name);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        // exec() only returns if it fails.
        Err(anyhow::Error::new(err).context(format!("failed to exec '{}'", binary_name)))
    }

    #[cfg(not(unix))]
    {
        let status = cmd.status()?;
        if !status.success() {
            anyhow::bail!("'{}' exited with status: {}", binary_name, status);
        }
        Ok(())
    }
}

async fn enforce_profile_pin(
    client: &EdgeplaneClient,
    profile_name: &str,
    allow_pin_mismatch: bool,
) -> Result<()> {
    let profile_root = ep_home_dir().join("profiles").join(profile_name);
    let pin_path = profile_root.join("pin.json");
    if !pin_path.exists() {
        return Ok(());
    }
    let pin_json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&pin_path)?)
        .context("invalid pin.json (expected JSON)")?;
    let pinned_sha = pin_json
        .get("pinned_sha256")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if pinned_sha.is_empty() {
        return Ok(());
    }
    let encoded: String = url::form_urlencoded::byte_serialize(profile_name.as_bytes()).collect();
    let remote = client
        .get_json(&format!("/me/profiles/{}", encoded))
        .await
        .with_context(|| format!("failed to fetch remote profile '{}'", profile_name))?;
    let remote_sha = remote
        .get("sha256")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if remote_sha.is_empty() {
        anyhow::bail!("remote profile '{}' has no sha256", profile_name);
    }
    if remote_sha != pinned_sha && !allow_pin_mismatch {
        anyhow::bail!(
            "profile '{}' is pinned to sha256 '{}' but remote is '{}'; rerun with --allow-pin-mismatch to override",
            profile_name,
            pinned_sha,
            remote_sha
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolve_embed_token_never_embeds_session_tokens() {
        assert!(!resolve_embed_token(false, "mcs_example_session_token"));
    }

    #[test]
    fn parse_agent_kind_accepts_only_driver_agents() {
        assert!(matches!(parse_agent_kind("gemini"), Ok(AgentKind::Gemini)));
        assert!(matches!(parse_agent_kind("openclaw"), Ok(AgentKind::Openclaw)));
        assert!(matches!(parse_agent_kind("custom"), Ok(AgentKind::Custom)));
        // Native runtimes are handled by `edgeplane run` directly, never here.
        assert!(parse_agent_kind("claude").is_err());
        assert!(parse_agent_kind("codex").is_err());
        // Removed legacy alias.
        assert!(parse_agent_kind("nanoclaw").is_err());
        assert!(parse_agent_kind("bogus").is_err());
    }

    #[test]
    fn gemini_config_writes_to_target_home() {
        let tmp = tempdir().expect("tempdir");
        let target_home = tmp.path().join("agent-home");
        let target_mc_home = tmp.path().join("edgeplane-home");
        fs::create_dir_all(&target_home).expect("target_home");
        fs::create_dir_all(&target_mc_home).expect("target_mc_home");

        let driver = GeminiDriver;
        driver
            .install_config(
                tmp.path(),
                "http://localhost:8008",
                "tok",
                true,
                &target_home,
                &target_mc_home,
            )
            .expect("install gemini config");

        assert!(target_home.join(".gemini/settings.json").exists());
    }

    #[test]
    #[cfg(unix)]
    fn overlay_seeds_profile_from_global_and_links_instance() {
        let tmp = tempdir().expect("tempdir");
        let global_home = tmp.path().join("global-home");
        let profile_home = tmp.path().join("profile-home");
        let agent_home = tmp.path().join("agent-home");
        fs::create_dir_all(global_home.join(".gemini")).expect("global home");
        fs::create_dir_all(&profile_home).expect("profile home");
        fs::create_dir_all(&agent_home).expect("agent home");

        let global_cfg = global_home.join(".gemini/settings.json");
        fs::write(&global_cfg, r#"{"theme":"dark"}"#).expect("write global config");

        initialize_profile_overlay(&AgentKind::Gemini, &agent_home, &profile_home, &global_home)
            .expect("initialize profile overlay");

        let profile_cfg = profile_home.join(".gemini/settings.json");
        assert!(profile_cfg.exists(), "profile config should be seeded");
        assert_eq!(
            fs::read_to_string(&profile_cfg).expect("read profile"),
            r#"{"theme":"dark"}"#
        );

        let instance_cfg = agent_home.join(".gemini/settings.json");
        let meta = fs::symlink_metadata(&instance_cfg).expect("instance metadata");
        assert!(
            meta.file_type().is_symlink(),
            "instance path should be symlink"
        );
        let target = fs::read_link(&instance_cfg).expect("read symlink");
        assert_eq!(target, profile_cfg);
    }
}
