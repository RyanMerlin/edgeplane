/// `mc mesh` — mc-mesh daemon control and work-model commands.
use crate::client::MissionControlClient;
use crate::config::McConfig;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

// ---------------------------------------------------------------------------
// Top-level group
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug)]
pub enum MeshCommand {
    /// Bring mc-mesh up: install if missing, then start the daemon.
    Up(MeshUpArgs),
    /// Stop the running mc-mesh daemon (install stays).
    Down,
    /// Remove the mc-mesh binary and systemd unit.
    Uninstall,
    /// Show daemon health: backend reachable, runtimes, watchdog state.
    Status,
    /// Deep health check with individual component results.
    Health,
    /// Upgrade the mc-mesh binary in place.
    Upgrade(MeshUpgradeArgs),
    /// Print mc-mesh daemon version.
    Version,
    /// Manage locally installed agent runtimes.
    #[command(subcommand)]
    Runtime(MeshRuntimeCommand),
    /// Manage agents in a mission's durable pool.
    #[command(subcommand)]
    Agent(MeshAgentCommand),
    /// Inspect klusters and their task DAGs.
    #[command(subcommand)]
    Kluster(MeshKlusterCommand),
    /// Manage and observe tasks.
    #[command(subcommand)]
    Task(MeshTaskCommand),
    /// Send and tail inter-agent messages.
    #[command(subcommand)]
    Msg(MeshMsgCommand),
    /// Attach to a running agent, task, or exec (auto-detected).
    Attach(MeshAttachArgs),
    /// Unified live feed of progress events and messages.
    Watch(MeshWatchArgs),
    /// Manage controlplane profiles (add, list, remove, rename).
    #[command(subcommand)]
    Profile(MeshProfileCommand),
    /// Select the active controlplane profile (or show the current one).
    Use(MeshUseArgs),
}

// ---------------------------------------------------------------------------
// Args
// ---------------------------------------------------------------------------

#[derive(Args, Debug)]
pub struct MeshUpArgs {
    #[arg(long, env = "MC_BACKEND_URL")]
    pub backend_url: Option<String>,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct MeshUpgradeArgs {
    #[arg(long)]
    pub version: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum MeshRuntimeCommand {
    Ls,
    Install(RuntimeInstallArgs),
    Test(RuntimeTestArgs),
}

#[derive(Args, Debug)]
pub struct RuntimeInstallArgs {
    pub kind: String,
}

#[derive(Args, Debug)]
pub struct RuntimeTestArgs {
    pub kind: String,
}

#[derive(Subcommand, Debug)]
pub enum MeshAgentCommand {
    /// List agents. In standalone mode reads the local registry; in federated
    /// mode queries the controlplane.
    Ls(AgentLsArgs),
    /// Enroll a new agent. In standalone mode writes to the local registry
    /// (~/.mc/mc-mesh.db); in federated mode calls the controlplane API.
    Enroll(AgentEnrollArgs),
    /// Provision the per-node home mission and enroll a default Goose agent
    /// in it. Standalone mirror of the controlplane's auto-provisioning at
    /// node-register time. Idempotent.
    EnrollHome(AgentEnrollHomeArgs),
    /// Reassign an agent to a different mission.
    Reassign(AgentReassignArgs),
    /// Remove an agent from the registry / controlplane.
    Unenroll(AgentUnenrollArgs),
    Attach(AgentAttachArgs),
    /// Set or update an agent's profile (role, instructions, scope, constraints).
    Profile(AgentProfileArgs),
}

#[derive(Args, Debug)]
pub struct AgentLsArgs {
    #[arg(long)]
    pub mission: Option<String>,
    #[arg(long)]
    pub status: Option<String>,
}

#[derive(Args, Debug)]
pub struct AgentEnrollArgs {
    #[arg(long)]
    pub mission: String,
    #[arg(long)]
    pub runtime: String,
    /// Task (default) or persistent supervision mode.
    #[arg(long, default_value = "task")]
    pub supervision: String,
    #[arg(long)]
    pub node: Option<String>,
    /// Path to a YAML or JSON profile file for this agent.
    #[arg(long)]
    pub profile: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct AgentEnrollHomeArgs {
    /// Hostname used to form the home mission slug `home-{slug(hostname)}`.
    /// Defaults to the Tailscale FQDN leaf (when Tailscale is running) or
    /// the system hostname.
    #[arg(long)]
    pub hostname: Option<String>,
    /// Runtime kind for the default home-mission agent. Goose is the
    /// recommended default — cheap local inference for routing/triage.
    #[arg(long, default_value = "goose")]
    pub runtime: String,
}

#[derive(Args, Debug)]
pub struct AgentReassignArgs {
    pub agent_id: String,
    /// New mission ID.
    #[arg(long)]
    pub mission: String,
}

#[derive(Args, Debug)]
pub struct AgentUnenrollArgs {
    pub agent_id: String,
}

#[derive(Args, Debug)]
pub struct AgentProfileArgs {
    /// Agent ID to update.
    pub agent_id: String,
    /// Path to a YAML or JSON file containing the profile.
    #[arg(long)]
    pub file: Option<std::path::PathBuf>,
    /// Quick single-field overrides: --name, --role, --instructions
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub role: Option<String>,
    #[arg(long)]
    pub instructions: Option<String>,
}

#[derive(Args, Debug)]
pub struct AgentAttachArgs {
    pub agent_id: String,
}

#[derive(Subcommand, Debug)]
pub enum MeshKlusterCommand {
    Ls(KlusterLsArgs),
    Show(KlusterShowArgs),
    Watch(KlusterWatchArgs),
}

#[derive(Args, Debug)]
pub struct KlusterLsArgs {
    #[arg(long)]
    pub mission: Option<String>,
}

#[derive(Args, Debug)]
pub struct KlusterShowArgs {
    pub kluster_id: String,
}

#[derive(Args, Debug)]
pub struct KlusterWatchArgs {
    pub kluster_id: String,
}

#[derive(Subcommand, Debug)]
pub enum MeshTaskCommand {
    Run(TaskRunArgs),
    Ls(TaskLsArgs),
    Show(TaskShowArgs),
    Watch(TaskWatchArgs),
    Attach(TaskAttachArgs),
    Cancel(TaskCancelArgs),
    Retry(TaskRetryArgs),
}

#[derive(Args, Debug)]
pub struct TaskRunArgs {
    pub kluster_id: String,
    #[arg(long)]
    pub title: String,
    #[arg(long, default_value = "")]
    pub description: String,
    #[arg(long, default_value = "first_claim")]
    pub claim_policy: String,
    #[arg(long)]
    pub runtime: Option<String>,
    #[arg(long)]
    pub depends_on: Option<String>,
    #[arg(long, default_value = "0")]
    pub priority: i32,
    #[arg(long)]
    pub input_file: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct TaskLsArgs {
    #[arg(long)]
    pub kluster: Option<String>,
    #[arg(long)]
    pub mission: Option<String>,
    #[arg(long)]
    pub status: Option<String>,
}

#[derive(Args, Debug)]
pub struct TaskShowArgs {
    pub task_id: String,
}

#[derive(Args, Debug)]
pub struct TaskWatchArgs {
    pub task_id: String,
    #[arg(long, default_value = "2")]
    pub interval_secs: u64,
}

#[derive(Args, Debug)]
pub struct TaskAttachArgs {
    pub task_id: String,
}

#[derive(Args, Debug)]
pub struct TaskCancelArgs {
    pub task_id: String,
}

#[derive(Args, Debug)]
pub struct TaskRetryArgs {
    pub task_id: String,
}

#[derive(Subcommand, Debug)]
pub enum MeshMsgCommand {
    Send(MsgSendArgs),
    Tail(MsgTailArgs),
}

#[derive(Args, Debug)]
pub struct MsgSendArgs {
    #[arg(long)]
    pub kluster: Option<String>,
    #[arg(long)]
    pub mission: Option<String>,
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long, default_value = "coordination")]
    pub channel: String,
    pub body: String,
}

#[derive(Args, Debug)]
pub struct MsgTailArgs {
    #[arg(long)]
    pub kluster: Option<String>,
    #[arg(long)]
    pub mission: Option<String>,
}

#[derive(Args, Debug)]
pub struct MeshAttachArgs {
    pub target: String,
}

#[derive(Args, Debug)]
pub struct MeshWatchArgs {
    #[arg(long)]
    pub mission: Option<String>,
    #[arg(long)]
    pub kluster: Option<String>,
}

// ---------------------------------------------------------------------------
// Profile management args
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug)]
pub enum MeshProfileCommand {
    /// Add a controlplane profile. If --bootstrap-token is given, registers
    /// this node with the controlplane and saves its identity in the profile.
    Add(ProfileAddArgs),
    /// List saved profiles.
    #[command(alias = "ls")]
    List,
    /// Remove a profile (clears active_profile if it was the active one).
    #[command(alias = "rm")]
    Remove(ProfileRemoveArgs),
    /// Rename a profile (preserves active_profile pointer if needed).
    Rename(ProfileRenameArgs),
    /// Show profile details (auth token is redacted).
    Show(ProfileShowArgs),
}

#[derive(Args, Debug)]
pub struct ProfileAddArgs {
    /// Unique profile name (e.g. "homelab", "work").
    pub name: String,
    /// Controlplane base URL (e.g. http://missioncontrol:8008).
    #[arg(long)]
    pub url: String,
    /// Bearer token for the controlplane.
    #[arg(long)]
    pub token: String,
    /// One-time bootstrap token from `mc node join-tokens`. When supplied,
    /// this node is registered with the controlplane and its identity
    /// (node_id + attach_secret) is saved into the profile.
    #[arg(long)]
    pub bootstrap_token: Option<String>,
    /// Display name for this node (defaults to system hostname).
    #[arg(long)]
    pub node_name: Option<String>,
    /// Trust tier label sent at registration (default: "untrusted").
    #[arg(long, default_value = "untrusted")]
    pub trust_tier: String,
    /// Tailscale FQDN to register (e.g. epyc.tailnet.ts.net).
    #[arg(long)]
    pub tailscale_fqdn: Option<String>,
    /// Set this profile as active immediately after adding.
    #[arg(long)]
    pub activate: bool,
}

#[derive(Args, Debug)]
pub struct ProfileRemoveArgs {
    pub name: String,
}

#[derive(Args, Debug)]
pub struct ProfileRenameArgs {
    pub old_name: String,
    pub new_name: String,
}

#[derive(Args, Debug)]
pub struct ProfileShowArgs {
    pub name: String,
}

#[derive(Args, Debug)]
pub struct MeshUseArgs {
    /// Profile to activate. Omit to show the currently active profile.
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// Main dispatcher
// ---------------------------------------------------------------------------

pub async fn handle(
    cmd: MeshCommand,
    client: &MissionControlClient,
    config: &McConfig,
) -> Result<()> {
    match cmd {
        MeshCommand::Up(a) => handle_up(a, config).await,
        MeshCommand::Down => handle_down(),
        MeshCommand::Uninstall => handle_uninstall(),
        MeshCommand::Status => handle_status(client).await,
        MeshCommand::Health => handle_health(client).await,
        MeshCommand::Upgrade(a) => handle_upgrade(a).await,
        MeshCommand::Version => handle_version(),
        MeshCommand::Runtime(cmd) => handle_runtime(cmd),
        MeshCommand::Agent(cmd) => handle_agent(cmd, client).await,
        MeshCommand::Kluster(cmd) => handle_kluster(cmd, client).await,
        MeshCommand::Task(cmd) => handle_task(cmd, client).await,
        MeshCommand::Msg(cmd) => handle_msg(cmd, client).await,
        MeshCommand::Attach(a) => handle_attach(a, client).await,
        MeshCommand::Watch(a) => handle_watch(a, client).await,
        MeshCommand::Profile(cmd) => handle_profile(cmd, client).await,
        MeshCommand::Use(a) => handle_use(a),
    }
}

fn _not_yet(cmd: &str) -> Result<()> {
    println!("{cmd}: not yet implemented");
    Ok(())
}

// ---------------------------------------------------------------------------
// Daemon lifecycle
// ---------------------------------------------------------------------------

async fn handle_up(args: MeshUpArgs, config: &McConfig) -> Result<()> {
    // 1. Check if mc-mesh binary exists.
    let binary = which_mc_mesh();

    if binary.is_none() {
        println!("mc-mesh binary not found in PATH.");
        let install = if args.yes {
            true
        } else {
            prompt_yes_no("Install mc-mesh now? (build from source) [y/N]")
        };
        if install {
            build_and_install_mc_mesh()?;
        } else {
            println!(
                "Skipped. Run `cargo install` from integrations/mc-mesh/ to install manually."
            );
            return Ok(());
        }
    }

    // 2. Check if already running.
    if is_daemon_running() {
        println!("mc-mesh daemon is already running.");
        return Ok(());
    }

    // 3. Start the daemon.
    let backend_url = args
        .backend_url
        .unwrap_or_else(|| config.base_url.to_string());
    let token = config.token.clone().unwrap_or_default();

    println!("Starting mc-mesh daemon…");
    start_daemon_background(&backend_url, &token)?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    if is_daemon_running() {
        println!("mc-mesh daemon started.");
    } else {
        println!(
            "mc-mesh daemon may not have started. Check logs at: journalctl --user -u mc-mesh"
        );
    }

    // Offer to install / enable the systemd user unit for persistence.
    if which_binary("systemctl") {
        let unit_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".config")
            .join("systemd")
            .join("user")
            .join("mc-mesh.service");
        if !unit_path.exists() {
            let install = if args.yes {
                true
            } else {
                prompt_yes_no("Install systemd user unit so mc-mesh starts on login? [y/N]")
            };
            if install {
                install_systemd_unit(&unit_path)?;
            }
        }
    }

    // Auto-register this host as a RuntimeNode if no node state exists.
    auto_register_node(config).await;

    println!("Run `mc mesh status` to check.");
    Ok(())
}

/// Register this host as a RuntimeNode if no node-state file exists.
/// Best-effort — logs a warning but does not abort `mc mesh up` on failure.
async fn auto_register_node(config: &McConfig) {
    use crate::runtime::{NodeState, load_node_state, persist_node_state};

    if matches!(load_node_state(), Ok(Some(_))) {
        // Already registered.
        return;
    }

    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| {
            std::process::Command::new("hostname")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .map_err(|_| std::env::VarError::NotPresent.into())
        })
        .unwrap_or_else(|_: Box<dyn std::error::Error>| "unknown".to_string());

    let client = match crate::client::MissionControlClient::new(config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mc mesh up: warning: could not build client for node registration: {e}");
            return;
        }
    };

    let body = serde_json::json!({
        "node_name": hostname,
        "hostname": hostname,
        "trust_tier": "standard",
        "labels": {},
        "capabilities": ["claude_code", "codex"],
    });

    match client.post_json("/runtime/nodes/register", &body).await {
        Ok(resp) => {
            let node_id = resp
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !node_id.is_empty() {
                let state = NodeState {
                    node_id: node_id.clone(),
                    node_name: hostname.clone(),
                };
                if let Err(e) = persist_node_state(&state) {
                    eprintln!("mc mesh up: warning: could not save node state: {e}");
                } else {
                    println!("Registered as runtime node {node_id} ({hostname})");
                }
            }
        }
        Err(e) => {
            eprintln!("mc mesh up: warning: could not auto-register runtime node: {e}");
        }
    }
}

fn install_systemd_unit(unit_path: &std::path::Path) -> Result<()> {
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Write a minimal user unit (binary path resolved at install time).
    let binary = which_mc_mesh().context("mc-mesh binary not found")?;
    let unit = format!(
        "[Unit]\n\
         Description=mc-mesh agent coordination daemon\n\
         After=network.target\n\n\
         [Service]\n\
         ExecStart={bin} run\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         StandardOutput=journal\n\
         StandardError=journal\n\
         SyslogIdentifier=mc-mesh\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        bin = binary.display()
    );
    std::fs::write(unit_path, unit)?;
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "enable", "mc-mesh.service"])
        .status();
    println!(
        "Systemd user unit installed and enabled at {}",
        unit_path.display()
    );
    println!("mc-mesh will start automatically on next login.");
    Ok(())
}

fn handle_down() -> Result<()> {
    let pid_path = pid_file_path();
    if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
            let _ = std::fs::remove_file(&pid_path);
            println!("Sent SIGTERM to mc-mesh daemon (pid {pid}).");
            return Ok(());
        }
    }
    println!("mc-mesh daemon does not appear to be running.");
    Ok(())
}

fn handle_uninstall() -> Result<()> {
    // 1. Stop the daemon if running.
    handle_down()?;

    // 2. Remove the binary from PATH locations.
    let removed_binary = if let Some(bin) = which_mc_mesh() {
        std::fs::remove_file(&bin).with_context(|| format!("remove {}", bin.display()))?;
        println!("Removed {}", bin.display());
        true
    } else {
        false
    };

    // 3. Disable and remove the systemd user unit if present.
    let unit_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".config")
        .join("systemd")
        .join("user")
        .join("mc-mesh.service");
    if unit_path.exists() {
        // Best-effort: disable then remove.
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "--now", "mc-mesh.service"])
            .status();
        let _ = std::fs::remove_file(&unit_path);
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        println!("Removed systemd user unit.");
    }

    // 4. Remove the local control socket if present.
    let sock = attach_socket_path();
    if sock.exists() {
        let _ = std::fs::remove_file(&sock);
    }

    if removed_binary {
        println!(
            "mc-mesh uninstalled. Config and work dirs are preserved at ~/.mc/mc-mesh*"
        );
    } else {
        println!("mc-mesh binary not found; nothing to remove.");
    }
    Ok(())
}

fn handle_version() -> Result<()> {
    match which_mc_mesh() {
        Some(bin) => {
            let out = std::process::Command::new(&bin)
                .arg("version")
                .output()
                .ok();
            if let Some(o) = out {
                print!("{}", String::from_utf8_lossy(&o.stdout));
            } else {
                println!("mc-mesh (version unknown)");
            }
        }
        None => println!("mc-mesh not installed"),
    }
    Ok(())
}

async fn handle_status(client: &MissionControlClient) -> Result<()> {
    let daemon_ok = is_daemon_running();
    let backend_ok = client.get_json("/health").await.is_ok();

    println!(
        "mc-mesh daemon:  {}",
        if daemon_ok { "running" } else { "stopped" }
    );
    println!(
        "backend:         {}",
        if backend_ok {
            "reachable"
        } else {
            "unreachable"
        }
    );

    if daemon_ok {
        if let Ok(pid) = std::fs::read_to_string(pid_file_path()) {
            println!("pid:             {}", pid.trim());
        }
    }

    // Phase 6: show what mode the daemon is in and what agents it manages.
    let state = read_state_v2();
    let mut node_id_for_query: Option<String> = None;
    if let Some(profile_name) = state["active_profile"].as_str() {
        println!("mode:            federated");
        println!("active profile:  {profile_name}");
        if let Some(entry) = state["profiles"].get(profile_name) {
            if let Some(node) = entry["node_id"].as_str() {
                if !node.is_empty() {
                    let short = if node.len() > 12 { &node[..12] } else { node };
                    println!("node_id:         {short}");
                    node_id_for_query = Some(node.to_string());
                }
            }
        }
    } else {
        println!("mode:            standalone");
    }

    // Federated: pull authoritative agent list from the controlplane.
    // Standalone: read the local SQLite registry.
    if let Some(node_id) = node_id_for_query {
        let path = format!("/runtime/nodes/{node_id}/agents");
        match client.get_json(&path).await {
            Ok(v) => print_federated_agents(&v),
            Err(e) => println!("agents:          (controlplane query failed: {e})"),
        }
    } else {
        match crate::local_db::list(None) {
            Ok(rows) if !rows.is_empty() => print_local_agents(&rows),
            Ok(_) => println!("agents:          none enrolled"),
            Err(e) => println!("agents:          (could not read local registry: {e})"),
        }
    }
    Ok(())
}

fn print_local_agents(rows: &[crate::local_db::LocalAgent]) {
    let mut by_mission: std::collections::BTreeMap<&str, Vec<&crate::local_db::LocalAgent>> =
        std::collections::BTreeMap::new();
    for a in rows {
        by_mission.entry(a.mission_id.as_str()).or_default().push(a);
    }
    println!(
        "agents:          {} across {} mission(s)",
        rows.len(),
        by_mission.len()
    );
    for (mid, agents) in &by_mission {
        let label = if mid.starts_with("home-") {
            format!("{mid} (home)")
        } else {
            (*mid).to_string()
        };
        let runtimes: Vec<&str> = agents.iter().map(|a| a.runtime_kind.as_str()).collect();
        println!("  - {label}: {}", runtimes.join(", "));
    }
}

fn print_federated_agents(v: &Value) {
    // The Phase 6 GET returns a JSON array of agents.
    let Some(arr) = v.as_array() else {
        println!("agents:          (unexpected response shape)");
        return;
    };
    if arr.is_empty() {
        println!("agents:          none enrolled");
        return;
    }
    let mut by_mission: std::collections::BTreeMap<&str, Vec<&Value>> =
        std::collections::BTreeMap::new();
    for a in arr {
        let mid = a.get("mission_id").and_then(|v| v.as_str()).unwrap_or("?");
        by_mission.entry(mid).or_default().push(a);
    }
    println!(
        "agents:          {} across {} mission(s)",
        arr.len(),
        by_mission.len()
    );
    for (mid, agents) in &by_mission {
        let kind = agents
            .iter()
            .find_map(|a| a.get("mission_kind").and_then(|v| v.as_str()))
            .unwrap_or("");
        let label = if kind == "home" {
            format!("{mid} (home)")
        } else {
            (*mid).to_string()
        };
        let runtimes: Vec<&str> = agents
            .iter()
            .filter_map(|a| a.get("runtime_kind").and_then(|v| v.as_str()))
            .collect();
        println!("  - {label}: {}", runtimes.join(", "));
    }
}

async fn handle_health(client: &MissionControlClient) -> Result<()> {
    handle_status(client).await?;

    // Check runtime binaries.
    for rt in &["claude", "codex", "gemini"] {
        let found = which_binary(rt);
        println!("{rt:15} {}", if found { "found" } else { "not found" });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Runtime management
// ---------------------------------------------------------------------------

fn handle_runtime(cmd: MeshRuntimeCommand) -> Result<()> {
    match cmd {
        MeshRuntimeCommand::Ls => {
            for rt in &["claude (claude-code)", "codex", "gemini"] {
                let binary = rt.split_whitespace().next().unwrap_or(rt);
                let found = which_binary(binary);
                println!(
                    "{rt:30} {}",
                    if found { "installed" } else { "not installed" }
                );
            }
            Ok(())
        }
        MeshRuntimeCommand::Install(a) => {
            let binary = match a.kind.as_str() {
                "claude-code" | "claude_code" => "claude",
                other => other,
            };
            println!("Install instructions for {binary}:");
            match binary {
                "claude" => println!("  npm install -g @anthropic-ai/claude-code"),
                "codex" => println!("  npm install -g @openai/codex"),
                "gemini" => println!("  npm install -g @google/gemini-cli"),
                _ => println!("  Unknown runtime. Check the project's README."),
            }
            Ok(())
        }
        MeshRuntimeCommand::Test(a) => {
            let binary = match a.kind.as_str() {
                "claude-code" | "claude_code" => "claude",
                other => other,
            };
            if which_binary(binary) {
                let out = std::process::Command::new(binary)
                    .arg("--version")
                    .output()
                    .context("failed to run --version")?;
                println!(
                    "{}: {}",
                    a.kind,
                    String::from_utf8_lossy(&out.stdout).trim()
                );
            } else {
                println!("{}: not found", a.kind);
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Agent pool
// ---------------------------------------------------------------------------

async fn handle_agent(cmd: MeshAgentCommand, client: &MissionControlClient) -> Result<()> {
    match cmd {
        MeshAgentCommand::Ls(a) => {
            if crate::local_db::is_federated() {
                // Federated: query controlplane.
                let mission_id = a.mission.as_deref().unwrap_or_default();
                if mission_id.is_empty() {
                    anyhow::bail!("--mission is required in federated mode");
                }
                let path = format!("/work/missions/{mission_id}/agents");
                let agents = client.get_json(&path).await?;
                print_agents(&agents);
            } else {
                // Standalone: read local registry.
                let agents = crate::local_db::list(a.mission.as_deref())
                    .context("reading local registry")?;
                if agents.is_empty() {
                    println!("No agents enrolled. Use `mc mesh agent enroll` to add one.");
                } else {
                    println!(
                        "{:<38} {:<14} {:<12} {:<14} {}",
                        "ID", "RUNTIME", "SUPERVISION", "MISSION", "ENROLLED"
                    );
                    println!("{}", "-".repeat(95));
                    for ag in &agents {
                        println!(
                            "{:<38} {:<14} {:<12} {:<14} {}",
                            ag.id,
                            ag.runtime_kind,
                            ag.supervision_mode,
                            ag.mission_id,
                            &ag.enrolled_at[..10], // date only
                        );
                    }
                }
            }
            Ok(())
        }

        MeshAgentCommand::Enroll(a) => {
            if crate::local_db::is_federated() {
                // Federated: call controlplane API (existing path).
                let machine = detect_machine_info();
                let profile: Option<Value> = match &a.profile {
                    Some(path) => {
                        let raw = std::fs::read_to_string(path)
                            .with_context(|| format!("reading profile file {}", path.display()))?;
                        let v: Value =
                            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                                serde_json::from_str(&raw)?
                            } else {
                                serde_yaml::from_str(&raw).context("parsing profile as YAML")?
                            };
                        Some(v)
                    }
                    None => None,
                };
                let mut body = json!({
                    "runtime_kind": a.runtime.replace('-', "_"),
                    "capabilities": default_capabilities_for(&a.runtime),
                    "labels": {},
                    "node_id": a.node,
                    "machine": machine,
                });
                if let Some(p) = profile {
                    body["profile"] = p;
                }
                let path = format!("/work/missions/{}/agents/enroll", a.mission);
                let result = client.post_json(&path, &body).await?;
                // Display the public_id (e.g. `aria-work-e88c006e`) — it's
                // what `mc agent remote message --to-agent-id` accepts, and
                // what mc-mesh uses to poll `/agents/{public_id}/messages`.
                // Falls back to the meshagent UUID for legacy responses.
                let display_id = result
                    .get("public_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| result.get("id").and_then(|v| v.as_str()))
                    .unwrap_or("?");
                println!(
                    "Enrolled agent {display_id} ({} in mission {})",
                    a.runtime, a.mission
                );
                println!(
                    "\nSet a profile: mc mesh agent profile {display_id} --role \"...\" --name \"...\""
                );
            } else {
                // Standalone: write directly to local SQLite registry.
                let supervision = match a.supervision.as_str() {
                    "persistent" => "persistent",
                    _ => "task",
                };
                let caps: Vec<String> = default_capabilities_for(&a.runtime)
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect();
                let profile_str = a.profile.as_ref().map(|p| p.to_string_lossy().into_owned());
                let agent_id = crate::local_db::enroll(
                    &a.mission,
                    &a.runtime.replace('-', "_"),
                    supervision,
                    &caps,
                    profile_str.as_deref(),
                )
                .context("enrolling agent in local registry")?;
                println!("Enrolled agent {agent_id} ({} in mission {}) [standalone]", a.runtime, a.mission);
                println!("The mc-mesh daemon will pick this up on its next reconcile tick.");
            }
            Ok(())
        }

        MeshAgentCommand::EnrollHome(a) => {
            if crate::local_db::is_federated() {
                println!(
                    "Federated mode — the home mission is auto-provisioned at \
                     `mc mesh profile add` time. Use `mc mesh agent ls` to inspect."
                );
                return Ok(());
            }
            // Pick hostname: explicit > Tailscale FQDN leaf > system hostname.
            let hostname_raw = a.hostname.clone()
                .or_else(|| detect_tailscale_fqdn().and_then(|f| {
                    f.split('.').next().filter(|s| !s.is_empty()).map(str::to_string)
                }))
                .or_else(|| {
                    std::process::Command::new("hostname")
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8(o.stdout).ok())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                });
            let hostname = hostname_raw.ok_or_else(|| {
                anyhow::anyhow!("could not determine hostname; pass --hostname")
            })?;
            let slug = slug_hostname(&hostname);
            if slug.is_empty() {
                anyhow::bail!("hostname {hostname:?} produced an empty slug");
            }
            let mission_id = format!("home-{slug}");
            let runtime = a.runtime.replace('-', "_");

            // Idempotency: bail if a matching home agent already exists.
            let existing = crate::local_db::list(Some(&mission_id))
                .context("checking local registry for existing home agent")?;
            if let Some(found) = existing.iter().find(|x| x.runtime_kind == runtime) {
                println!(
                    "Home mission {} already has a {} agent ({}). Nothing to do.",
                    mission_id, runtime, found.id
                );
                return Ok(());
            }

            // Persistent supervision so the home agent stays attached for live
            // messages and routing decisions.
            let caps: Vec<String> = [
                "routing",
                "triage",
                "dispatch",
                "overlap_check",
            ]
            .into_iter()
            .map(String::from)
            .collect();

            let agent_id = crate::local_db::enroll(
                &mission_id,
                &runtime,
                "persistent",
                &caps,
                None,
            )
            .context("enrolling home agent in local registry")?;

            println!("Provisioned home mission {} [standalone]", mission_id);
            println!("  agent: {agent_id} ({runtime}, persistent)");
            println!("The mc-mesh daemon will pick this up on its next reconcile tick.");
            Ok(())
        }

        MeshAgentCommand::Reassign(a) => {
            if crate::local_db::is_federated() {
                // Federated: PATCH on controlplane.
                let path = format!("/work/agents/{}/reassign", a.agent_id);
                let body = json!({ "mission_id": a.mission });
                client.post_json(&path, &body).await
                    .with_context(|| format!("reassigning {} to {}", a.agent_id, a.mission))?;
                println!("Reassigned {} → {}", a.agent_id, a.mission);
            } else {
                // Standalone: update local registry.
                let found = crate::local_db::reassign(&a.agent_id, &a.mission)
                    .context("updating local registry")?;
                if found {
                    println!("Reassigned {} → {} [standalone]", a.agent_id, a.mission);
                    println!("The mc-mesh daemon will pick this up on its next reconcile tick.");
                } else {
                    anyhow::bail!("agent {} not found in local registry", a.agent_id);
                }
            }
            Ok(())
        }

        MeshAgentCommand::Unenroll(a) => {
            if crate::local_db::is_federated() {
                // Federated: DELETE on controlplane.
                let path = format!("/work/agents/{}", a.agent_id);
                client.delete(&path).await
                    .with_context(|| format!("unenrolling {}", a.agent_id))?;
                println!("Unenrolled {}", a.agent_id);
            } else {
                // Standalone: delete from local registry.
                let found = crate::local_db::unenroll(&a.agent_id)
                    .context("deleting from local registry")?;
                if found {
                    println!("Unenrolled {} [standalone]", a.agent_id);
                    println!("The mc-mesh daemon will pick this up on its next reconcile tick.");
                } else {
                    anyhow::bail!("agent {} not found in local registry", a.agent_id);
                }
            }
            Ok(())
        }

        MeshAgentCommand::Attach(a) => {
            handle_attach(MeshAttachArgs { target: a.agent_id }, client).await
        }
        MeshAgentCommand::Profile(a) => handle_agent_profile(a, client).await,
    }
}

fn print_agents(agents: &Value) {
    if let Some(arr) = agents.as_array() {
        if arr.is_empty() {
            println!("No agents enrolled.");
            return;
        }
        // `public_id` is the wire identifier (e.g. `aria-work-e88c006e`).
        // Falls back to the meshagent UUID when the row predates the
        // public_id link migration. The numeric meshagent.id column is no
        // longer shown — too noisy at fleet scale and unused by mc CLI verbs.
        println!(
            "{:<28} {:<14} {:<10} {:<20} {}",
            "PUBLIC_ID", "RUNTIME", "STATUS", "NAME / ROLE", "TASK"
        );
        println!("{}", "-".repeat(85));
        for a in arr {
            let pid = a["public_id"]
                .as_str()
                .or_else(|| a["id"].as_str())
                .unwrap_or("?");
            let rt = a["runtime_kind"].as_str().unwrap_or("?");
            let st = a["status"].as_str().unwrap_or("?");
            let task = a["current_task_id"].as_str().unwrap_or("-");
            let name_role = {
                let name = a["profile"]["name"].as_str().unwrap_or("");
                let role = a["profile"]["role"].as_str().unwrap_or("");
                match (name, role) {
                    ("", "") => "-".to_string(),
                    (n, "") => n.to_string(),
                    ("", r) => r.to_string(),
                    (n, r) => format!("{n} / {r}"),
                }
            };
            println!("{pid:<28} {rt:<14} {st:<10} {name_role:<20} {task}");
        }
    }
}

/// Detect host machine info to send at enrollment.
fn detect_machine_info() -> Value {
    let hostname = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into()));

    let os = {
        let kernel = std::process::Command::new("uname")
            .arg("-sr")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        // Try /etc/os-release pretty name on Linux.
        let pretty = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("PRETTY_NAME="))
                    .and_then(|l| l.strip_prefix("PRETTY_NAME="))
                    .map(|v| v.trim_matches('"').to_string())
            });
        match pretty {
            Some(p) if !kernel.is_empty() => format!("{p} ({kernel})"),
            Some(p) => p,
            None => kernel,
        }
    };

    let cpu_cores: u32 = std::process::Command::new("nproc")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let work_dir = crate::config::mc_home_dir().join("mc-mesh").join("work");

    // Detect key tools.
    let tools: Vec<Value> = [
        ("claude", &["--version"][..]),
        ("codex", &["--version"]),
        ("gemini", &["version"]),
        ("git", &["--version"]),
        ("cargo", &["--version"]),
        ("docker", &["--version"]),
    ]
    .iter()
    .filter_map(|(name, args)| {
        let out = std::process::Command::new(name).args(*args).output().ok()?;
        let raw = if out.stdout.is_empty() {
            out.stderr
        } else {
            out.stdout
        };
        let version = String::from_utf8_lossy(&raw)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if version.is_empty() {
            return None;
        }
        Some(json!({ "name": name, "version": version }))
    })
    .collect();

    json!({
        "hostname": hostname,
        "os": os,
        "cpu_cores": cpu_cores,
        "working_dir": work_dir.display().to_string(),
        "installed_tools": tools,
    })
}

/// Default capabilities for a runtime kind.
fn default_capabilities_for(runtime: &str) -> Vec<&'static str> {
    match runtime.replace('-', "_").as_str() {
        "claude_code" => vec![
            "claude_code",
            "code.read",
            "code.edit",
            "code.plan",
            "test.run",
        ],
        "codex" => vec!["codex", "code.read", "code.edit", "test.run"],
        "gemini" => vec!["gemini", "code.read", "code.plan"],
        _ => vec![],
    }
}

async fn handle_agent_profile(a: AgentProfileArgs, client: &MissionControlClient) -> Result<()> {
    // Start from file if provided, else empty object.
    let mut profile: serde_json::Map<String, Value> = match &a.file {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            let v: Value = if path.extension().and_then(|e| e.to_str()) == Some("json") {
                serde_json::from_str(&raw)?
            } else {
                serde_yaml::from_str(&raw).context("parsing profile as YAML")?
            };
            v.as_object().cloned().unwrap_or_default()
        }
        None => serde_json::Map::new(),
    };

    // CLI overrides take precedence.
    if let Some(name) = a.name {
        profile.insert("name".into(), Value::String(name));
    }
    if let Some(role) = a.role {
        profile.insert("role".into(), Value::String(role));
    }
    if let Some(inst) = a.instructions {
        profile.insert("instructions".into(), Value::String(inst));
    }

    if profile.is_empty() {
        anyhow::bail!("Provide --file or at least one of --name, --role, --instructions");
    }

    let path = format!("/work/agents/{}/profile", a.agent_id);
    let result = client.patch_json(&path, &Value::Object(profile)).await?;
    let name = result["profile"]["name"].as_str().unwrap_or("-");
    let role = result["profile"]["role"].as_str().unwrap_or("-");
    println!(
        "Updated profile for {} — name: {name}, role: {role}",
        a.agent_id
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Kluster commands
// ---------------------------------------------------------------------------

async fn handle_kluster(cmd: MeshKlusterCommand, client: &MissionControlClient) -> Result<()> {
    match cmd {
        MeshKlusterCommand::Ls(a) => {
            let mission_id = a.mission.as_deref().unwrap_or_default();
            if mission_id.is_empty() {
                anyhow::bail!("--mission is required");
            }
            let klusters = client
                .get_json(&format!("/missions/{mission_id}/k"))
                .await?;
            if let Some(arr) = klusters.as_array() {
                println!("{:<38} {}", "ID", "NAME");
                println!("{}", "-".repeat(60));
                for k in arr {
                    println!(
                        "{:<38} {}",
                        k["id"].as_str().unwrap_or("?"),
                        k["name"].as_str().unwrap_or("?")
                    );
                }
            }
            Ok(())
        }
        MeshKlusterCommand::Show(a) => {
            let graph = client
                .get_json(&format!("/work/klusters/{}/graph", a.kluster_id))
                .await?;
            println!("Kluster {}", a.kluster_id);
            if let Some(nodes) = graph["nodes"].as_array() {
                println!("\nTasks ({}):", nodes.len());
                println!("{:<38} {:<12} {}", "ID", "STATUS", "TITLE");
                println!("{}", "-".repeat(70));
                for n in nodes {
                    println!(
                        "{:<38} {:<12} {}",
                        n["id"].as_str().unwrap_or("?"),
                        n["status"].as_str().unwrap_or("?"),
                        n["title"].as_str().unwrap_or("?")
                    );
                }
            }
            if let Some(edges) = graph["edges"].as_array() {
                if !edges.is_empty() {
                    println!("\nDependencies:");
                    for e in edges {
                        println!(
                            "  {} → {}",
                            e["from"].as_str().unwrap_or("?"),
                            e["to"].as_str().unwrap_or("?")
                        );
                    }
                }
            }
            Ok(())
        }
        MeshKlusterCommand::Watch(a) => watch_kluster(&a.kluster_id, client).await,
    }
}

// ---------------------------------------------------------------------------
// Task commands
// ---------------------------------------------------------------------------

async fn handle_task(cmd: MeshTaskCommand, client: &MissionControlClient) -> Result<()> {
    match cmd {
        MeshTaskCommand::Run(a) => {
            let depends_on: Vec<String> = a
                .depends_on
                .as_deref()
                .unwrap_or("")
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(String::from)
                .collect();

            let required = a
                .runtime
                .as_deref()
                .map(|r| vec![r.replace('-', "_")])
                .unwrap_or_default();

            let input_json = if let Some(path) = &a.input_file {
                std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?
            } else {
                "{}".into()
            };

            let body = json!({
                "title": a.title,
                "description": a.description,
                "claim_policy": a.claim_policy,
                "depends_on": depends_on,
                "required_capabilities": required,
                "priority": a.priority,
                "input_json": input_json,
            });

            let result = client
                .post_json(&format!("/work/klusters/{}/tasks", a.kluster_id), &body)
                .await?;

            let task_id = result["id"].as_str().unwrap_or("?");
            let status = result["status"].as_str().unwrap_or("?");
            println!("Task created: {task_id}");
            println!("Status:       {status}");
            println!("\nWatch progress: mc mesh task watch {task_id}");
            Ok(())
        }
        MeshTaskCommand::Ls(a) => {
            if let Some(kluster_id) = &a.kluster {
                let path = match &a.status {
                    Some(s) => format!("/work/klusters/{kluster_id}/tasks?status={s}"),
                    None => format!("/work/klusters/{kluster_id}/tasks"),
                };
                let tasks = client.get_json(&path).await?;
                print_tasks(&tasks);
            } else {
                anyhow::bail!("--kluster is required");
            }
            Ok(())
        }
        MeshTaskCommand::Show(a) => {
            let task = client
                .get_json(&format!("/work/tasks/{}", a.task_id))
                .await?;
            println!("{}", serde_json::to_string_pretty(&task)?);

            // Also print progress history.
            let progress = client
                .get_json(&format!("/work/tasks/{}/progress", a.task_id))
                .await?;
            if let Some(arr) = progress.as_array() {
                if !arr.is_empty() {
                    println!("\n-- Progress events ({}) --", arr.len());
                    print_progress_events(arr);
                }
            }
            Ok(())
        }
        MeshTaskCommand::Watch(a) => watch_task(&a.task_id, a.interval_secs, client).await,
        MeshTaskCommand::Attach(a) => {
            // Resolve the task → its claiming agent, then attach to that agent.
            let task = client
                .get_json(&format!("/work/tasks/{}", a.task_id))
                .await?;
            let agent_id = task["claimed_by_agent_id"]
                .as_str()
                .ok_or_else(|| {
                    anyhow::anyhow!("task {} is not currently claimed by any agent", a.task_id)
                })?
                .to_string();
            println!("Task {} is running on agent {agent_id}", a.task_id);
            handle_attach(MeshAttachArgs { target: agent_id }, client).await
        }
        MeshTaskCommand::Cancel(a) => {
            client
                .post_json(&format!("/work/tasks/{}/cancel", a.task_id), &json!({}))
                .await?;
            println!("Task {} cancelled.", a.task_id);
            Ok(())
        }
        MeshTaskCommand::Retry(a) => {
            let result = client
                .post_json(&format!("/work/tasks/{}/retry", a.task_id), &json!({}))
                .await?;
            println!(
                "Task {} status: {}",
                a.task_id,
                result["status"].as_str().unwrap_or("?")
            );
            Ok(())
        }
    }
}

fn print_tasks(tasks: &Value) {
    if let Some(arr) = tasks.as_array() {
        if arr.is_empty() {
            println!("No tasks.");
            return;
        }
        println!("{:<38} {:<12} {}", "ID", "STATUS", "TITLE");
        println!("{}", "-".repeat(70));
        for t in arr {
            println!(
                "{:<38} {:<12} {}",
                t["id"].as_str().unwrap_or("?"),
                t["status"].as_str().unwrap_or("?"),
                t["title"].as_str().unwrap_or("?")
            );
        }
    }
}

fn print_progress_events(events: &[Value]) {
    for e in events {
        let seq = e["seq"].as_i64().unwrap_or(0);
        let ev_type = e["event_type"].as_str().unwrap_or("info");
        let phase = e["phase"].as_str().unwrap_or("");
        let summary = e["summary"].as_str().unwrap_or("");
        let phase_str = if !phase.is_empty() {
            format!("[{phase}] ")
        } else {
            String::new()
        };
        println!("  #{seq:>4}  {ev_type:<20} {phase_str}{summary}");
    }
}

/// Poll /work/tasks/{id}/progress until task is finished/failed/cancelled.
async fn watch_task(
    task_id: &str,
    interval_secs: u64,
    client: &MissionControlClient,
) -> Result<()> {
    println!("Watching task {task_id} (Ctrl-C to stop)…\n");
    let mut last_seq: i64 = -1;
    let interval = Duration::from_secs(interval_secs);

    loop {
        // Fetch task status.
        let task = client.get_json(&format!("/work/tasks/{task_id}")).await?;
        let status = task["status"].as_str().unwrap_or("?");

        // Fetch new progress events.
        let progress = client
            .get_json(&format!(
                "/work/tasks/{task_id}/progress?since_seq={last_seq}"
            ))
            .await?;

        if let Some(arr) = progress.as_array() {
            for e in arr {
                let seq = e["seq"].as_i64().unwrap_or(0);
                let ev_type = e["event_type"].as_str().unwrap_or("info");
                let phase = e["phase"].as_str().unwrap_or("");
                let summary = e["summary"].as_str().unwrap_or("");
                let phase_str = if !phase.is_empty() {
                    format!("[{phase}] ")
                } else {
                    String::new()
                };
                println!("  #{seq:>4}  {ev_type:<20} {phase_str}{summary}");
                last_seq = last_seq.max(seq);
            }
        }

        if matches!(status, "finished" | "failed" | "cancelled") {
            println!("\nTask {task_id}: {status}");
            break;
        }

        tokio::time::sleep(interval).await;
    }
    Ok(())
}

/// Stream /work/klusters/{id}/stream via WebSocket with exponential-backoff reconnect.
async fn watch_kluster(kluster_id: &str, client: &MissionControlClient) -> Result<()> {
    println!("Watching kluster {kluster_id} (Ctrl-C to stop)…\n");
    watch_ws_stream(
        &format!("/work/klusters/{kluster_id}/stream"),
        client,
    )
    .await
}

/// Connect to a WebSocket event stream path and print events until Ctrl-C.
/// Reconnects with exponential backoff (1s → 30s) on disconnect.
async fn watch_ws_stream(path: &str, client: &MissionControlClient) -> Result<()> {
    let mut backoff = Duration::from_secs(1);

    loop {
        let mut url = client.ws_url(path)?;
        if let Some(token) = client.token() {
            url.query_pairs_mut().append_pair("token", token);
        }

        match connect_async(url.as_str()).await {
            Ok((mut ws, _)) => {
                backoff = Duration::from_secs(1); // reset on successful connect
                while let Some(msg) = ws.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if let Ok(event) = serde_json::from_str::<Value>(&text) {
                                let event_kind = event["event"].as_str().unwrap_or("");
                                let event_type = event["type"].as_str().unwrap_or("");
                                if event_type == "ping" || event_kind.is_empty() {
                                    continue;
                                }
                                let task_id = event["task_id"].as_str().unwrap_or("");
                                let status = event["status"].as_str().unwrap_or("");
                                println!("{event_kind:<24} task={task_id}  status={status}");
                            }
                        }
                        Ok(Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }
                eprintln!("[watch] disconnected — reconnecting in {}s…", backoff.as_secs());
            }
            Err(e) => {
                eprintln!("[watch] connect failed: {e} — retrying in {}s…", backoff.as_secs());
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

// Legacy REST poll kept for reference; replaced by watch_kluster above.
#[allow(dead_code)]
async fn _watch_kluster_rest(kluster_id: &str, client: &MissionControlClient) -> Result<()> {
    println!("Watching kluster {kluster_id} (REST poll, Ctrl-C to stop)…\n");
    let mut _last_progress_id: i64 = 0;
    let mut last_msg_id: i64 = 0;

    loop {
        let tasks = client
            .get_json(&format!("/work/klusters/{kluster_id}/tasks"))
            .await?;
        let msgs = client
            .get_json(&format!(
                "/work/klusters/{kluster_id}/messages?since_id={last_msg_id}"
            ))
            .await?;

        if let Some(arr) = msgs.as_array() {
            for m in arr {
                let id = m["id"].as_i64().unwrap_or(0);
                let from = m["from_agent_id"].as_str().unwrap_or("?");
                let channel = m["channel"].as_str().unwrap_or("?");
                let body = &m["body_json"];
                println!("[msg/{channel}] {from}: {body}");
                last_msg_id = last_msg_id.max(id);
            }
        }

        if let Some(arr) = tasks.as_array() {
            let in_progress: Vec<_> = arr
                .iter()
                .filter(|t| matches!(t["status"].as_str().unwrap_or(""), "running" | "claimed"))
                .collect();
            if !in_progress.is_empty() {
                for t in &in_progress {
                    println!(
                        "[task/{}] {} — {}",
                        t["status"].as_str().unwrap_or("?"),
                        t["id"].as_str().unwrap_or("?"),
                        t["title"].as_str().unwrap_or("?")
                    );
                }
            }

            let all_done = arr.iter().all(|t| {
                matches!(
                    t["status"].as_str().unwrap_or(""),
                    "finished" | "failed" | "cancelled"
                )
            });
            if !arr.is_empty() && all_done {
                println!("\nAll tasks in kluster {kluster_id} are done.");
                break;
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

async fn handle_msg(cmd: MeshMsgCommand, client: &MissionControlClient) -> Result<()> {
    match cmd {
        MeshMsgCommand::Send(a) => {
            let body = json!({
                "to_agent_id": a.to,
                "channel": a.channel,
                "body_json": json!({ "text": a.body }).to_string(),
            });
            if let Some(kluster_id) = &a.kluster {
                client
                    .post_json(&format!("/work/klusters/{kluster_id}/messages"), &body)
                    .await?;
                println!("Message sent to kluster {kluster_id}.");
            } else if let Some(mission_id) = &a.mission {
                client
                    .post_json(&format!("/work/missions/{mission_id}/messages"), &body)
                    .await?;
                println!("Message sent to mission {mission_id}.");
            } else {
                anyhow::bail!("--kluster or --mission is required");
            }
            Ok(())
        }
        MeshMsgCommand::Tail(a) => {
            let is_kluster = a.kluster.is_some();
            let scope_id = a
                .kluster
                .or(a.mission)
                .context("--kluster or --mission is required")?;

            println!("Tailing messages for {scope_id} (Ctrl-C to stop)…\n");
            let mut last_id: i64 = 0;

            loop {
                let path = if is_kluster {
                    format!("/work/klusters/{scope_id}/messages?since_id={last_id}")
                } else {
                    format!("/work/missions/{scope_id}/messages?since_id={last_id}")
                };
                let msgs = client.get_json(&path).await?;
                if let Some(arr) = msgs.as_array() {
                    for m in arr {
                        let id = m["id"].as_i64().unwrap_or(0);
                        let from = m["from_agent_id"].as_str().unwrap_or("?");
                        let channel = m["channel"].as_str().unwrap_or("?");
                        let body = &m["body_json"];
                        println!("[{channel}] {from}: {body}");
                        last_id = last_id.max(id);
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Watch (unified feed)
// ---------------------------------------------------------------------------

async fn handle_watch(args: MeshWatchArgs, client: &MissionControlClient) -> Result<()> {
    if let Some(kluster_id) = &args.kluster {
        println!("Watching kluster {kluster_id} (Ctrl-C to stop)…\n");
        watch_ws_stream(&format!("/work/klusters/{kluster_id}/stream"), client).await
    } else if let Some(mission_id) = &args.mission {
        println!("Watching mission {mission_id} (Ctrl-C to stop)…\n");
        watch_ws_stream(&format!("/work/missions/{mission_id}/stream"), client).await
    } else {
        anyhow::bail!("--mission or --kluster is required for `mc mesh watch`")
    }
}

// ---------------------------------------------------------------------------
// Attach (PTY proxy via local daemon unix socket)
// ---------------------------------------------------------------------------

#[cfg(unix)]
async fn handle_attach(args: MeshAttachArgs, client: &MissionControlClient) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let socket_path = attach_socket_path();
    if !socket_path.exists() {
        anyhow::bail!(
            "mc-mesh daemon socket not found at {}.\nIs the daemon running? Try `mc mesh up`.",
            socket_path.display()
        );
    }

    let target = &args.target;

    // Auto-detect: if the target looks like a task ID (not an agent), resolve it.
    // Agent IDs and task IDs are both UUIDs; we try the task endpoint first.
    let agent_id = resolve_attach_target(target, client).await?;

    println!("Attaching to agent {agent_id}… (Ctrl-C or Ctrl-D to detach)\n");

    let stream = UnixStream::connect(&socket_path)
        .await
        .with_context(|| format!("connect to {}", socket_path.display()))?;

    let (mut sock_read, mut sock_write) = stream.into_split();

    // Send agent ID.
    sock_write
        .write_all(format!("{agent_id}\n").as_bytes())
        .await?;

    // Read response line.
    let mut resp = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        sock_read.read_exact(&mut byte).await?;
        if byte[0] == b'\n' {
            break;
        }
        resp.push(byte[0]);
    }
    let resp = String::from_utf8_lossy(&resp).into_owned();
    if !resp.starts_with("OK") {
        anyhow::bail!("daemon refused attach: {resp}");
    }

    // Enter raw terminal mode so every keystroke goes straight to the PTY.
    let _raw_guard = RawTerminal::enter()?;

    // Spawn task: socket output → stdout
    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        let mut buf = vec![0u8; 4096];
        loop {
            match sock_read.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if stdout.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    let _ = stdout.flush().await;
                }
            }
        }
        println!("\r\n[detached]");
    });

    // This task: stdin → socket
    let mut stdin = tokio::io::stdin();
    let mut buf = vec![0u8; 256];
    loop {
        match stdin.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if sock_write.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}

#[cfg(not(unix))]
async fn handle_attach(_args: MeshAttachArgs, _client: &MissionControlClient) -> Result<()> {
    anyhow::bail!("`mc mesh attach` is currently only supported on Unix-like hosts");
}

/// Auto-detect whether `target` is a task ID or an agent ID.
///
/// Tries `/work/tasks/{target}` first.  If it 404s, assumes it's an agent ID.
async fn resolve_attach_target(target: &str, client: &MissionControlClient) -> Result<String> {
    if let Ok(task) = client.get_json(&format!("/work/tasks/{target}")).await {
        if let Some(agent_id) = task["claimed_by_agent_id"].as_str() {
            return Ok(agent_id.to_string());
        }
        // Task exists but isn't running — fall through to treat target as agent ID.
    }
    Ok(target.to_string())
}

/// RAII guard that sets the terminal to raw mode on entry and restores on drop.
struct RawTerminal {
    #[cfg(unix)]
    saved: libc::termios,
}

impl RawTerminal {
    fn enter() -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = std::io::stdin().as_raw_fd();
            let mut saved = unsafe { std::mem::zeroed::<libc::termios>() };
            if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
                anyhow::bail!("tcgetattr failed");
            }
            let mut raw = saved;
            unsafe { libc::cfmakeraw(&mut raw) };
            if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
                anyhow::bail!("tcsetattr failed");
            }
            Ok(RawTerminal { saved })
        }
        #[cfg(not(unix))]
        {
            Ok(RawTerminal {})
        }
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = std::io::stdin().as_raw_fd();
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &self.saved) };
        }
    }
}

fn attach_socket_path() -> std::path::PathBuf {
    crate::config::mc_home_dir().join("mc-mesh.sock")
}

fn mgmt_socket_path() -> std::path::PathBuf {
    crate::config::mc_home_dir().join("mc-mesh-mgmt.sock")
}

/// Connect-probe the daemon's mgmt socket. Returns true only if the socket
/// exists AND accepts a connection — covers the systemd-managed daemon that
/// doesn't write /tmp/mc-mesh.pid.
fn mgmt_socket_responsive() -> bool {
    #[cfg(unix)]
    {
        let path = mgmt_socket_path();
        if !path.exists() {
            return false;
        }
        std::os::unix::net::UnixStream::connect(&path).is_ok()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

// ---------------------------------------------------------------------------
// Upgrade
// ---------------------------------------------------------------------------

async fn handle_upgrade(args: MeshUpgradeArgs) -> Result<()> {
    println!("Upgrading mc-mesh…");

    // Stop the running daemon first.
    handle_down()?;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // If a version was requested, we'd normally fetch it from a release URL.
    // For now, rebuild from source (same as the initial install path).
    if let Some(ref v) = args.version {
        println!("Requested version: {v}");
        println!("Pinned-version installs from release URLs are not yet supported.");
        println!("Building from source instead…");
    }

    build_and_install_mc_mesh()?;

    println!("Upgrade complete. Run `mc mesh up` to restart the daemon.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn which_mc_mesh() -> Option<std::path::PathBuf> {
    which::which("mc-mesh").ok()
}

fn which_binary(name: &str) -> bool {
    which::which(name).is_ok()
}

fn pid_file_path() -> std::path::PathBuf {
    std::env::temp_dir().join("mc-mesh.pid")
}


fn is_daemon_running() -> bool {
    // 1. Try the PID file written by `mc mesh start` (foreground / detached
    //    launch from the CLI).
    let pid_path = pid_file_path();
    if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            #[cfg(unix)]
            {
                if unsafe { libc::kill(pid as libc::pid_t, 0) == 0 } {
                    return true;
                }
            }
            #[cfg(not(unix))]
            {
                return true;
            }
        }
    }
    // 2. Fall back to probing the mgmt socket. The systemd-managed daemon
    //    (mc-mesh.service) doesn't touch /tmp/mc-mesh.pid, so a responsive
    //    socket is the authoritative liveness signal.
    mgmt_socket_responsive()
}

fn start_daemon_background(backend_url: &str, token: &str) -> Result<()> {
    let binary = which_mc_mesh().context("mc-mesh binary not found after install attempt")?;

    let pid_path = pid_file_path();
    let child = std::process::Command::new(&binary)
        .arg("run")
        .arg("--backend-url")
        .arg(backend_url)
        .arg("--token")
        .arg(token)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn mc-mesh run")?;

    std::fs::write(&pid_path, child.id().to_string())?;
    Ok(())
}

fn build_and_install_mc_mesh() -> Result<()> {
    // Find the mc-mesh workspace relative to the mc crate's location.
    // In development, both live in the missioncontrol repo.
    let mc_mesh_dir = locate_mc_mesh_workspace();

    if let Some(dir) = mc_mesh_dir {
        println!("Building mc-mesh from {}…", dir.display());
        let status = std::process::Command::new("cargo")
            .args(["install", "--path", "crates/mc-mesh", "--force"])
            .current_dir(&dir)
            .status()
            .context("cargo install failed")?;
        if !status.success() {
            anyhow::bail!("cargo install exited with {status}");
        }
        println!("mc-mesh installed.");
    } else {
        println!("Could not locate mc-mesh workspace. Install manually:");
        println!("  cd integrations/mc-mesh && cargo install --path crates/mc-mesh");
    }
    Ok(())
}

fn locate_mc_mesh_workspace() -> Option<std::path::PathBuf> {
    // Walk up from current exe to find the mc-mesh workspace in development.
    let mut dir = std::env::current_exe().ok()?;
    for _ in 0..8 {
        dir = dir.parent()?.to_path_buf();
        let relative = "integrations/mc-mesh";
        let candidate = dir.join(relative).join("Cargo.toml");
        if candidate.exists() {
            return Some(dir.join(relative));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Profile management
// ---------------------------------------------------------------------------

fn state_file_path() -> PathBuf {
    crate::config::mc_home_dir().join("mc-mesh.state.json")
}

/// Read the state file as a v2 JSON object. Returns an empty v2 structure on
/// any error. If the file is v1 format, the v1 identity is preserved in memory
/// as a "default" profile entry (the daemon will write back v2 on next start).
fn read_state_v2() -> serde_json::Value {
    let path = state_file_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return json!({ "schema_version": 2, "profiles": {} }),
    };
    let v: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return json!({ "schema_version": 2, "profiles": {} }),
    };
    let version = v.get("schema_version").and_then(|s| s.as_u64()).unwrap_or(0) as u32;
    if version >= 2 {
        return v;
    }
    // v1: synthesize a v2 structure in memory only.
    let mut profiles = serde_json::Map::new();
    if let (Some(nid), Some(sec), Some(url)) = (
        v.get("node_id").and_then(|s| s.as_str()),
        v.get("attach_secret").and_then(|s| s.as_str()),
        v.get("controlplane_url").and_then(|s| s.as_str()),
    ) {
        let reg = v.get("registered_at").and_then(|s| s.as_str()).unwrap_or("");
        profiles.insert(
            "default".into(),
            json!({
                "url": url,
                "auth": { "kind": "token", "token": "" },
                "node_id": nid,
                "attach_secret": sec,
                "registered_at": reg,
            }),
        );
    }
    let active = if profiles.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String("default".into())
    };
    json!({ "schema_version": 2, "active_profile": active, "profiles": serde_json::Value::Object(profiles) })
}

/// Atomically write the state file (mode 0600).
fn write_state(state: &serde_json::Value) -> Result<()> {
    let path = state_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(mut p) = std::fs::metadata(&tmp).map(|m| m.permissions()) {
            p.set_mode(0o600);
            let _ = std::fs::set_permissions(&tmp, p);
        }
    }
    std::fs::rename(&tmp, &path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(mut p) = std::fs::metadata(&path).map(|m| m.permissions()) {
            p.set_mode(0o600);
            let _ = std::fs::set_permissions(&path, p);
        }
    }
    Ok(())
}

async fn handle_profile(cmd: MeshProfileCommand, client: &MissionControlClient) -> Result<()> {
    match cmd {
        MeshProfileCommand::Add(a) => handle_profile_add(a, client).await,
        MeshProfileCommand::List => handle_profile_list(),
        MeshProfileCommand::Remove(a) => handle_profile_remove(a),
        MeshProfileCommand::Rename(a) => handle_profile_rename(a),
        MeshProfileCommand::Show(a) => handle_profile_show(a),
    }
}

async fn handle_profile_add(a: ProfileAddArgs, _client: &MissionControlClient) -> Result<()> {
    // If a bootstrap token is provided, register this node with the controlplane.
    let (node_id, attach_secret, resolved_fqdn, home_info) = if let Some(bt) = &a.bootstrap_token {
        let reg_client = MissionControlClient::new_with_token(&a.url, &a.token)?;
        let node_name = a.node_name.clone().unwrap_or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".into())
        });

        // Auto-detect Tailscale FQDN if the user didn't supply one. The home
        // mission slug is derived from this (FQDN leaf > hostname > node_name)
        // on the server side, so a real FQDN gives the nicest naming.
        let tailscale_fqdn = a
            .tailscale_fqdn
            .clone()
            .or_else(detect_tailscale_fqdn);

        let body = json!({
            "node_name": node_name,
            "hostname": node_name,
            "trust_tier": a.trust_tier,
            "labels": {},
            "capacity": {},
            "capabilities": [],
            "runtime_version": env!("CARGO_PKG_VERSION"),
            "bootstrap_token": bt,
            "tailscale_fqdn": tailscale_fqdn,
        });
        let resp = reg_client
            .post_json("/runtime/nodes/register", &body)
            .await
            .context("calling /runtime/nodes/register")?;
        let nid = resp
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("register response missing `id`"))?
            .to_string();
        let sec = resp
            .get("attach_secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("register response missing `attach_secret`"))?
            .to_string();
        // Phase 6: register_node returns `home: {mission_id, agent_id}` when
        // home-mission auto-provisioning succeeded.
        let home = resp.get("home").cloned();
        (nid, sec, tailscale_fqdn, home)
    } else {
        (String::new(), String::new(), a.tailscale_fqdn.clone(), None)
    };

    let mut state = read_state_v2();
    let profiles = state["profiles"]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("unexpected state file structure"))?;

    if profiles.contains_key(&a.name) {
        anyhow::bail!(
            "profile '{}' already exists. Remove it first: mc mesh profile rm {}",
            a.name,
            a.name
        );
    }

    let is_first = profiles.is_empty();
    let registered_at = chrono::Utc::now().to_rfc3339();
    let mut entry = json!({
        "url": a.url,
        "auth": { "kind": "token", "token": a.token },
        "node_id": node_id,
        "attach_secret": attach_secret,
        "registered_at": registered_at,
    });
    if let Some(fqdn) = &resolved_fqdn {
        entry["tailscale_fqdn"] = serde_json::Value::String(fqdn.clone());
    }
    profiles.insert(a.name.clone(), entry);
    // profiles borrow ends here.

    if a.activate || is_first {
        state["active_profile"] = serde_json::Value::String(a.name.clone());
    }

    write_state(&state)?;

    if !node_id.is_empty() {
        println!("Registered node {} and saved as profile '{}'.", node_id, a.name);
        if let Some(fqdn) = &resolved_fqdn {
            if a.tailscale_fqdn.is_none() {
                println!("  detected Tailscale FQDN: {fqdn}");
            }
        }
        if let Some(home) = &home_info {
            let mid = home.get("mission_id").and_then(|v| v.as_str()).unwrap_or("?");
            println!("  home mission: {mid}");
        }
    } else {
        println!("Saved profile '{}' (url: {}).", a.name, a.url);
    }
    if state["active_profile"].as_str() == Some(&a.name) {
        println!("Active profile: '{}'.", a.name);
    } else {
        println!("Run `mc mesh use {}` to activate this profile.", a.name);
    }
    Ok(())
}

/// Slug-safe form of a hostname for use in stable mission IDs. Mirrors the
/// controlplane's `slug_hostname` so federated and standalone provisioning
/// produce identical names for the same node.
fn slug_hostname(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for c in input.chars() {
        let mapped = if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase()
        } else {
            '-'
        };
        if mapped == '-' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
            }
            prev_dash = true;
        } else {
            out.push(mapped);
            prev_dash = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Best-effort Tailscale FQDN detection via `tailscale status --json`.
/// Returns the FQDN with the trailing dot stripped, or None if Tailscale is
/// not installed / not running / produces unexpected output.
fn detect_tailscale_fqdn() -> Option<String> {
    let out = std::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let dns_name = v.get("Self")?.get("DNSName")?.as_str()?;
    let trimmed = dns_name.trim_end_matches('.');
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}

fn handle_profile_list() -> Result<()> {
    let state = read_state_v2();
    let active = state["active_profile"].as_str().unwrap_or("");
    let profiles = match state["profiles"].as_object() {
        Some(p) => p,
        None => {
            println!("No profiles saved.");
            return Ok(());
        }
    };
    if profiles.is_empty() {
        println!("No profiles saved. Add one: mc mesh profile add <name> --url <url> --token <tok>");
        return Ok(());
    }
    println!("{:<4} {:<20} {:<40} {}", "  ", "NAME", "URL", "NODE_ID");
    println!("{}", "-".repeat(80));
    for (name, entry) in profiles {
        let marker = if name == active { "* " } else { "  " };
        let url = entry["url"].as_str().unwrap_or("-");
        let nid = entry["node_id"].as_str().unwrap_or("-");
        let nid_short = if nid.len() > 12 { &nid[..12] } else { nid };
        println!("{}{:<20} {:<40} {}", marker, name, url, nid_short);
    }
    Ok(())
}

fn handle_profile_remove(a: ProfileRemoveArgs) -> Result<()> {
    let mut state = read_state_v2();
    let profiles = state["profiles"]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("unexpected state file structure"))?;
    if !profiles.contains_key(&a.name) {
        anyhow::bail!("profile '{}' not found", a.name);
    }
    profiles.remove(&a.name);
    // profiles borrow ends here.

    // Clear active_profile if it pointed at the removed profile.
    if state["active_profile"].as_str() == Some(&a.name) {
        state["active_profile"] = serde_json::Value::Null;
    }
    write_state(&state)?;
    println!("Removed profile '{}'.", a.name);
    Ok(())
}

fn handle_profile_rename(a: ProfileRenameArgs) -> Result<()> {
    let mut state = read_state_v2();
    let profiles = state["profiles"]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("unexpected state file structure"))?;
    if !profiles.contains_key(&a.old_name) {
        anyhow::bail!("profile '{}' not found", a.old_name);
    }
    if profiles.contains_key(&a.new_name) {
        anyhow::bail!("profile '{}' already exists", a.new_name);
    }
    let entry = profiles.remove(&a.old_name).expect("checked above");
    profiles.insert(a.new_name.clone(), entry);
    // profiles borrow ends here.

    if state["active_profile"].as_str() == Some(a.old_name.as_str()) {
        state["active_profile"] = serde_json::Value::String(a.new_name.clone());
    }
    write_state(&state)?;
    println!("Renamed '{}' → '{}'.", a.old_name, a.new_name);
    Ok(())
}

fn handle_profile_show(a: ProfileShowArgs) -> Result<()> {
    let state = read_state_v2();
    let profiles = state["profiles"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("unexpected state file structure"))?;
    let entry = profiles
        .get(&a.name)
        .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", a.name))?;
    let active = state["active_profile"].as_str().unwrap_or("");
    println!("Profile: {} {}", a.name, if a.name == active { "(active)" } else { "" });
    println!("  url:           {}", entry["url"].as_str().unwrap_or("-"));
    println!("  node_id:       {}", entry["node_id"].as_str().unwrap_or("-"));
    println!("  registered_at: {}", entry["registered_at"].as_str().unwrap_or("-"));
    println!("  auth.kind:     {}", entry["auth"]["kind"].as_str().unwrap_or("-"));
    println!("  auth.token:    <redacted>");
    if let Some(fqdn) = entry["tailscale_fqdn"].as_str() {
        println!("  tailscale_fqdn: {fqdn}");
    }
    Ok(())
}

fn handle_use(a: MeshUseArgs) -> Result<()> {
    let mut state = read_state_v2();

    match &a.name {
        None => {
            // Show current active profile.
            let active = state["active_profile"].as_str().unwrap_or("<none>");
            println!("Active profile: {active}");
        }
        Some(name) => {
            let exists = state["profiles"]
                .as_object()
                .is_some_and(|p| p.contains_key(name));
            if !exists {
                anyhow::bail!(
                    "profile '{}' not found. List profiles with `mc mesh profile list`.",
                    name
                );
            }
            state["active_profile"] = serde_json::Value::String(name.clone());
            write_state(&state)?;
            println!("Switched to profile '{name}'.");
            println!("Restart mc-mesh for the change to take effect: mc mesh down && mc mesh up");
        }
    }
    Ok(())
}

fn prompt_yes_no(prompt: &str) -> bool {
    use std::io::Write;
    print!("{prompt} ");
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    let _ = std::io::stdin().read_line(&mut input);
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::slug_hostname;

    // Mirror of mc-controlplane's slug_hostname tests — the two implementations
    // must produce identical output so federated and standalone provisioning
    // converge on the same mission IDs for a given hostname.

    #[test]
    fn plain_hostname_unchanged() {
        assert_eq!(slug_hostname("excalibur"), "excalibur");
    }

    #[test]
    fn uppercase_is_lowered() {
        assert_eq!(slug_hostname("Excalibur"), "excalibur");
    }

    #[test]
    fn dots_become_dashes() {
        assert_eq!(slug_hostname("node.tailnet.ts.net"), "node-tailnet-ts-net");
    }

    #[test]
    fn collapses_repeated_dashes() {
        assert_eq!(slug_hostname("node...local"), "node-local");
        assert_eq!(slug_hostname("a__b__c"), "a-b-c");
    }

    #[test]
    fn trims_trailing_dashes() {
        assert_eq!(slug_hostname("hostname---"), "hostname");
        assert_eq!(slug_hostname("node."), "node");
    }

    #[test]
    fn leading_invalid_chars_dropped() {
        assert_eq!(slug_hostname("...foo"), "foo");
    }

    #[test]
    fn empty_input_yields_empty() {
        assert_eq!(slug_hostname(""), "");
        assert_eq!(slug_hostname("..."), "");
    }

    #[test]
    fn alphanumeric_mix_preserved() {
        assert_eq!(slug_hostname("node-01"), "node-01");
        assert_eq!(slug_hostname("cloud0"), "cloud0");
    }
}
