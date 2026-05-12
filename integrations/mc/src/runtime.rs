use crate::{client::MissionControlClient, output, output::OutputMode};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use futures_util::{SinkExt, StreamExt};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use uuid::Uuid;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::sleep,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::info;

#[derive(Subcommand, Debug)]
pub enum RuntimeCommand {
    /// Runtime node operations.
    #[command(subcommand)]
    Nodes(RuntimeNodesCommand),
    /// Runtime job operations.
    #[command(subcommand)]
    Jobs(RuntimeJobsCommand),
    /// Runtime lease helpers.
    #[command(subcommand)]
    Leases(RuntimeLeasesCommand),
    /// Runtime execution-session helpers.
    #[command(subcommand)]
    Sessions(RuntimeSessionsCommand),
}

#[derive(Subcommand, Debug)]
pub enum NodeAgentCommand {
    /// Register a node with MissionControl and persist its identity locally.
    Register(NodeAgentRegisterArgs),
    /// Run the resident node loop.
    Run(NodeAgentRunArgs),
    /// Inspect local node-agent readiness.
    Doctor(NodeAgentDoctorArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeNodesCommand {
    Register(RuntimeNodeRegisterArgs),
    List(RuntimeListArgs),
    Heartbeat(RuntimeNodeHeartbeatArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeJobsCommand {
    Submit(RuntimeJobSubmitArgs),
    List(RuntimeListArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeLeasesCommand {
    Create(RuntimeLeaseCreateArgs),
    Status(RuntimeLeaseStatusArgs),
    Complete(RuntimeLeaseCompleteArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeSessionsCommand {
    Attach(RuntimeSessionAttachArgs),
}

#[derive(Args, Debug)]
pub struct RuntimeNodeRegisterArgs {
    #[arg(long)]
    pub node_name: String,
    #[arg(long, default_value = "")]
    pub hostname: String,
    #[arg(long, default_value = "untrusted")]
    pub trust_tier: String,
}

#[derive(Args, Debug)]
pub struct NodeAgentRegisterArgs {
    #[arg(long)]
    pub node_name: String,
    #[arg(long, default_value = "")]
    pub hostname: String,
    #[arg(long, default_value = "untrusted")]
    pub trust_tier: String,
}

#[derive(Args, Debug)]
pub struct NodeAgentRunArgs {
    #[arg(long, default_value = "30")]
    pub poll_seconds: u64,
    #[arg(long, default_value = "15")]
    pub heartbeat_seconds: u64,
    #[arg(long, default_value = "node")]
    pub node_name: String,
    #[arg(long, default_value = "")]
    pub hostname: String,
    #[arg(long, default_value = "untrusted")]
    pub trust_tier: String,
    #[arg(long, default_value = "container,host_process")]
    pub capabilities: String,
    #[arg(long, default_value = "")]
    pub labels: String,
}

#[derive(Args, Debug)]
pub struct NodeAgentDoctorArgs {
    #[arg(long, default_value = "node")]
    pub node_name: String,
}

#[derive(Args, Debug)]
pub struct RuntimeNodeHeartbeatArgs {
    #[arg(long)]
    pub node_id: String,
    #[arg(long, default_value = "online")]
    pub status: String,
}

#[derive(Args, Debug)]
pub struct RuntimeJobSubmitArgs {
    #[arg(long, default_value = "")]
    pub mission_id: String,
    #[arg(long, default_value = "")]
    pub runtime_session_id: String,
    #[arg(long, default_value = "container")]
    pub runtime_class: String,
    #[arg(long, default_value = "")]
    pub image: String,
    #[arg(long, default_value = "")]
    pub command: String,
}

#[derive(Args, Debug)]
pub struct RuntimeLeaseCreateArgs {
    #[arg(long)]
    pub job_id: String,
    #[arg(long)]
    pub node_id: String,
}

#[derive(Args, Debug)]
pub struct RuntimeLeaseStatusArgs {
    #[arg(long)]
    pub lease_id: String,
    #[arg(long)]
    pub status: String,
}

#[derive(Args, Debug)]
pub struct RuntimeLeaseCompleteArgs {
    #[arg(long)]
    pub lease_id: String,
    #[arg(long, default_value_t = 0)]
    pub exit_code: i32,
    #[arg(long, default_value = "")]
    pub error_message: String,
}

#[derive(Args, Debug)]
pub struct RuntimeSessionAttachArgs {
    #[arg(long)]
    pub session_id: String,
    #[arg(long, default_value_t = false)]
    pub raw: bool,
}

#[derive(Args, Debug, Default)]
pub struct RuntimeListArgs {
    #[arg(long)]
    pub status: Option<String>,
}

pub async fn run(
    command: RuntimeCommand,
    client: &MissionControlClient,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        RuntimeCommand::Nodes(cmd) => run_nodes(cmd, client, output_mode).await,
        RuntimeCommand::Jobs(cmd) => run_jobs(cmd, client, output_mode).await,
        RuntimeCommand::Leases(cmd) => run_leases(cmd, client, output_mode).await,
        RuntimeCommand::Sessions(cmd) => run_sessions(cmd, client, output_mode).await,
    }
}

pub async fn run_node_agent(
    command: NodeAgentCommand,
    client: &MissionControlClient,
) -> Result<()> {
    match command {
        NodeAgentCommand::Register(args) => run_node_register(args, client).await,
        NodeAgentCommand::Run(args) => run_node_run(args, client).await,
        NodeAgentCommand::Doctor(args) => run_node_doctor(args).await,
    }
}

async fn run_nodes(
    command: RuntimeNodesCommand,
    client: &MissionControlClient,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        RuntimeNodesCommand::Register(args) => {
            let response = client
                .post_json(
                    "/runtime/nodes/register",
                    &json!({"node_name": args.node_name,"hostname": args.hostname,"trust_tier": args.trust_tier}),
                )
                .await?;
            output::print_value(output_mode, &response);
        }
        RuntimeNodesCommand::List(_) => {
            let response = client.get_json("/runtime/nodes").await?;
            output::print_value(output_mode, &response);
        }
        RuntimeNodesCommand::Heartbeat(args) => {
            let response = client
                .post_json(
                    &format!("/runtime/nodes/{}/heartbeat", args.node_id),
                    &json!({"status": args.status}),
                )
                .await?;
            output::print_value(output_mode, &response);
        }
    }
    Ok(())
}

async fn run_jobs(
    command: RuntimeJobsCommand,
    client: &MissionControlClient,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        RuntimeJobsCommand::Submit(args) => {
            let response = client
                .post_json(
                    "/runtime/jobs",
                    &json!({"mission_id": args.mission_id,"runtime_session_id": args.runtime_session_id,"runtime_class": args.runtime_class,"image": args.image,"command": args.command}),
                )
                .await?;
            output::print_value(output_mode, &response);
        }
        RuntimeJobsCommand::List(args) => {
            let path = match args.status {
                Some(status) if !status.trim().is_empty() => {
                    format!("/runtime/jobs?status={status}")
                }
                _ => "/runtime/jobs".to_string(),
            };
            let response = client.get_json(&path).await?;
            output::print_value(output_mode, &response);
        }
    }
    Ok(())
}

async fn run_leases(
    command: RuntimeLeasesCommand,
    client: &MissionControlClient,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        RuntimeLeasesCommand::Create(args) => {
            let response = client
                .post_json(
                    &format!("/runtime/jobs/{}/leases", args.job_id),
                    &json!({"node_id": args.node_id}),
                )
                .await?;
            output::print_value(output_mode, &response);
        }
        RuntimeLeasesCommand::Status(args) => {
            let response = client
                .post_json(
                    &format!("/runtime/leases/{}/status", args.lease_id),
                    &json!({"status": args.status}),
                )
                .await?;
            output::print_value(output_mode, &response);
        }
        RuntimeLeasesCommand::Complete(args) => {
            let response = client
                .post_json(
                    &format!("/runtime/leases/{}/complete", args.lease_id),
                    &json!({"exit_code": args.exit_code,"error_message": args.error_message}),
                )
                .await?;
            output::print_value(output_mode, &response);
        }
    }
    Ok(())
}

async fn run_sessions(
    command: RuntimeSessionsCommand,
    client: &MissionControlClient,
    output_mode: OutputMode,
) -> Result<()> {
    let _ = output_mode;
    match command {
        RuntimeSessionsCommand::Attach(args) => attach_session(args, client).await,
    }
}

#[derive(Clone, Debug)]
pub struct NodeState {
    pub node_id: String,
    pub node_name: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct NodeRuntimeConfig {
    node_name: String,
    hostname: String,
    trust_tier: String,
    bootstrap_token: String,
    upgrade_channel: String,
    desired_version: String,
    poll_seconds: u64,
    heartbeat_seconds: u64,
    capabilities: Vec<String>,
    labels: serde_json::Map<String, Value>,
    upgrade_manifest_url: String,
}

impl Default for NodeRuntimeConfig {
    fn default() -> Self {
        Self {
            node_name: String::new(),
            hostname: String::new(),
            trust_tier: "untrusted".to_string(),
            bootstrap_token: String::new(),
            upgrade_channel: "stable".to_string(),
            desired_version: String::new(),
            poll_seconds: 30,
            heartbeat_seconds: 15,
            capabilities: Vec::new(),
            labels: serde_json::Map::new(),
            upgrade_manifest_url: String::new(),
        }
    }
}

fn node_state_path() -> PathBuf {
    crate::config::mc_home_dir()
        .join("runtime")
        .join("node.json")
}

fn node_config_path() -> PathBuf {
    crate::config::mc_home_dir()
        .join("runtime")
        .join("node-config.json")
}

pub fn load_node_state() -> Result<Option<NodeState>> {
    let path = node_state_path();
    let raw = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let json: Value = serde_json::from_str(&raw).context("invalid node state json")?;
    let node_id = json
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let node_name = json
        .get("node_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if node_id.is_empty() || node_name.is_empty() {
        return Ok(None);
    }
    Ok(Some(NodeState { node_id, node_name }))
}

pub fn persist_node_state(state: &NodeState) -> Result<()> {
    let path = node_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        serde_json::to_string_pretty(&json!({
            "node_id": state.node_id,
            "node_name": state.node_name,
        }))?,
    )?;
    Ok(())
}

fn load_node_config() -> Result<Option<NodeRuntimeConfig>> {
    let path = node_config_path();
    let raw = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let config: NodeRuntimeConfig =
        serde_json::from_str(&raw).context("invalid node config json")?;
    Ok(Some(config))
}

fn persist_node_config(config: &NodeRuntimeConfig) -> Result<()> {
    let path = node_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}

fn default_node_config(args: &NodeAgentRunArgs) -> NodeRuntimeConfig {
    NodeRuntimeConfig {
        node_name: std::env::var("MC_NODE_NAME").unwrap_or_else(|_| args.node_name.clone()),
        hostname: std::env::var("MC_NODE_HOSTNAME").unwrap_or_else(|_| args.hostname.clone()),
        trust_tier: std::env::var("MC_NODE_TRUST_TIER").unwrap_or_else(|_| args.trust_tier.clone()),
        bootstrap_token: std::env::var("MC_NODE_BOOTSTRAP_TOKEN").unwrap_or_default(),
        upgrade_channel: std::env::var("MC_NODE_UPGRADE_CHANNEL")
            .unwrap_or_else(|_| "stable".to_string()),
        desired_version: std::env::var("MC_NODE_DESIRED_VERSION").unwrap_or_default(),
        poll_seconds: std::env::var("MC_NODE_POLL_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(args.poll_seconds),
        heartbeat_seconds: std::env::var("MC_NODE_HEARTBEAT_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(args.heartbeat_seconds),
        capabilities: {
            let mut caps: Vec<String> = args
                .capabilities
                .split(',')
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect();
            // Auto-detect installed runtimes
            if which_binary("goose").is_some() && !caps.contains(&"goose".to_string()) {
                caps.push("goose".to_string());
            }
            caps
        },
        labels: parse_kv_pairs(&args.labels)
            .as_object()
            .cloned()
            .unwrap_or_default(),
        upgrade_manifest_url: std::env::var("MC_NODE_UPGRADE_MANIFEST_URL").unwrap_or_default(),
    }
}

async fn run_node_register(
    args: NodeAgentRegisterArgs,
    client: &MissionControlClient,
) -> Result<()> {
    let config = load_node_config()?.unwrap_or_else(|| NodeRuntimeConfig {
        node_name: args.node_name.clone(),
        hostname: args.hostname.clone(),
        trust_tier: args.trust_tier.clone(),
        ..NodeRuntimeConfig::default()
    });
    let response = client
        .post_json(
            "/runtime/nodes/register",
            &json!({
                "node_name": config.node_name,
                "hostname": config.hostname,
                "trust_tier": config.trust_tier,
                "bootstrap_token": config.bootstrap_token,
                "labels": config.labels,
                "capabilities": config.capabilities,
                "runtime_version": config.desired_version,
            }),
        )
        .await?;
    let state = NodeState {
        node_id: response
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        node_name: response
            .get("node_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    };
    persist_node_state(&state)?;
    output::print_value(OutputMode::Json, &response);
    Ok(())
}

async fn run_node_doctor(args: NodeAgentDoctorArgs) -> Result<()> {
    let state = load_node_state()?;
    let config = load_node_config()?;
    let payload = json!({
        "ok": state.is_some() && config.is_some(),
        "node_name": args.node_name,
        "state_path": node_state_path(),
        "config_path": node_config_path(),
        "registered": state.as_ref().map(|s| s.node_name.clone()),
        "configured": config.as_ref().map(|c| c.node_name.clone()),
    });
    output::print_value(OutputMode::Json, &payload);
    Ok(())
}

async fn run_node_run(args: NodeAgentRunArgs, client: &MissionControlClient) -> Result<()> {
    // ── DEPRECATION NOTICE ────────────────────────────────────────────────────
    // `mc node run` is the legacy node-agent path.  It will be removed once
    // `mc mesh` reaches full feature parity.  The recommended path is:
    //
    //   mc mesh up                          # start the mc-mesh daemon
    //   mc mesh agent enroll --mission <id> --runtime claude-code
    //   mc mesh task run <kluster-id> --title "my task"
    //
    // See: https://github.com/RyanMerlin/missioncontrol/tree/main/integrations/mc-mesh
    // ─────────────────────────────────────────────────────────────────────────
    eprintln!(
        "⚠  mc node run is deprecated. Use `mc mesh up` instead.\n\
         See `mc mesh --help` for the new work-model commands.\n"
    );

    let mut config = load_node_config()?.unwrap_or_else(|| default_node_config(&args));
    if config.bootstrap_token.is_empty() {
        config.bootstrap_token = std::env::var("MC_NODE_BOOTSTRAP_TOKEN").unwrap_or_default();
    }
    if config.bootstrap_token.is_empty() {
        return Err(anyhow::anyhow!(
            "node bootstrap token missing; seed ~/.mc/runtime/node-config.json or MC_NODE_BOOTSTRAP_TOKEN"
        ));
    }
    persist_node_config(&config)?;

    let heartbeat_interval = Duration::from_secs(config.heartbeat_seconds.max(1));
    let poll_interval = Duration::from_secs(config.poll_seconds.max(1));
    let state = match load_node_state()? {
        Some(state) => state,
        None => {
            let response = client
                .post_json(
                    "/runtime/nodes/register",
                    &json!({
                        "node_name": config.node_name,
                        "hostname": config.hostname,
                        "trust_tier": config.trust_tier,
                        "labels": config.labels,
                        "capabilities": config.capabilities,
                        "runtime_version": config.desired_version,
                        "bootstrap_token": config.bootstrap_token,
                    }),
                )
                .await?;
            let state = NodeState {
                node_id: response
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                node_name: response
                    .get("node_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            };
            persist_node_state(&state)?;
            state
        }
    };

    info!(node_id = %state.node_id, node_name = %state.node_name, "runtime node loop starting");
    let mut last_heartbeat = tokio::time::Instant::now() - heartbeat_interval;
    loop {
        if last_heartbeat.elapsed() >= heartbeat_interval {
            let _ = client
                .post_json(
                    &format!("/runtime/nodes/{}/heartbeat", state.node_id),
                    &json!({"status":"online","capabilities":config.capabilities,"labels":config.labels,"runtime_version":config.desired_version}),
                )
                .await;
            last_heartbeat = tokio::time::Instant::now();
        }

        if let Ok(remote) = client
            .get_json(&format!("/runtime/nodes/{}/config", state.node_id))
            .await
        {
            if let Some(spec) = remote.get("spec") {
                let desired = spec
                    .get("desired_version")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !desired.is_empty() && desired != config.desired_version {
                    config.desired_version = desired;
                    persist_node_config(&config)?;
                }
                if let Some(drain_state) = spec.get("drain_state").and_then(Value::as_str) {
                    if drain_state == "cordoned" || drain_state == "draining" {
                        sleep(poll_interval).await;
                        continue;
                    }
                    if drain_state == "upgrading" && !config.upgrade_manifest_url.trim().is_empty()
                    {
                        let _ = client
                            .post_json(
                                &format!("/runtime/nodes/{}/reconcile", state.node_id),
                                &json!({
                                    "desired_version": config.desired_version,
                                    "drain_state": "upgrading",
                                    "health_summary": "self-update requested",
                                }),
                            )
                            .await;
                        let mut child = Command::new(std::env::current_exe()?);
                        child.args([
                            "system",
                            "update",
                            "self-update",
                            "--manifest-url",
                            &config.upgrade_manifest_url,
                        ]);
                        let status = child
                            .status()
                            .await
                            .context("failed to launch self-update")?;
                        if status.success() {
                            break Ok(());
                        }
                    }
                }
            }
        }

        let claim = client
            .post_json(
                &format!("/runtime/nodes/{}/leases/claim", state.node_id),
                &json!({}),
            )
            .await
            .unwrap_or_else(|_| json!({"claimed":false}));

        if claim
            .get("claimed")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let lease = claim.get("lease").cloned().unwrap_or_else(|| json!({}));
            let job = claim.get("job").cloned().unwrap_or_else(|| json!({}));
            let lease_id = lease
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if lease_id.is_empty() {
                sleep(poll_interval).await;
                continue;
            }
            let runtime_class = job
                .get("runtime_class")
                .and_then(Value::as_str)
                .unwrap_or("container");
            let command = job.get("command").and_then(Value::as_str).unwrap_or("");
            let args_vec = job
                .get("args")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|value| value.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>();
            let env_map = job
                .get("env")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let cwd = job.get("cwd").and_then(Value::as_str).unwrap_or("");

            let _ = client
                .post_json(
                    &format!("/runtime/leases/{}/status", lease_id),
                    &json!({"status":"running"}),
                )
                .await;
            let pty_requested = runtime_class == "host_process";
            let session = client
                .post_json(
                    "/runtime/execution-sessions",
                    &json!({
                        "lease_id": lease_id,
                        "runtime_class": runtime_class,
                        "pty_requested": pty_requested
                    }),
                )
                .await
                .ok();
            let session_id = session
                .as_ref()
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let result = execute_job(
                client,
                &session_id,
                &lease_id,
                runtime_class,
                command,
                &args_vec,
                env_map,
                cwd,
                pty_requested,
            )
            .await;
            match result {
                Ok(exit_code) => {
                    let _ = client
                        .post_json(
                            &format!("/runtime/leases/{}/complete", lease_id),
                            &json!({"exit_code": exit_code,"error_message": ""}),
                        )
                        .await;
                }
                Err(err) => {
                    let _ = client
                        .post_json(
                            &format!("/runtime/leases/{}/complete", lease_id),
                            &json!({"exit_code": 1,"error_message": err.to_string()}),
                        )
                        .await;
                }
            }
        }

        sleep(poll_interval).await;
    }
}

async fn execute_job(
    client: &MissionControlClient,
    session_id: &str,
    lease_id: &str,
    runtime_class: &str,
    command: &str,
    args: &[String],
    env_map: serde_json::Map<String, Value>,
    cwd: &str,
    pty_requested: bool,
) -> Result<i32> {
    if pty_requested {
        return execute_pty_job(client, session_id, lease_id, command, args, env_map, cwd).await;
    }
    let mut cmd = if runtime_class == "container" {
        build_container_command(command, args, &env_map, cwd)?
    } else if runtime_class == "goose" {
        build_goose_command(command, &env_map, cwd)?
    } else {
        build_host_command(command, args, &env_map, cwd)
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("failed to spawn runtime job")?;
    if let Some(stdout) = child.stdout.take() {
        let client = client.clone();
        let session_id = session_id.to_string();
        let lease_id = lease_id.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = client
                    .post_json(
                        &format!("/runtime/execution-sessions/{}/attach", session_id),
                        &json!({"session_id": session_id,"stream":"stdout","content":line}),
                    )
                    .await;
                let _ = client
                    .post_json(
                        &format!("/runtime/leases/{}/logs", lease_id),
                        &json!({"stream":"stdout","content":line}),
                    )
                    .await;
            }
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let client = client.clone();
        let session_id = session_id.to_string();
        let lease_id = lease_id.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = client
                    .post_json(
                        &format!("/runtime/execution-sessions/{}/attach", session_id),
                        &json!({"session_id": session_id,"stream":"stderr","content":line}),
                    )
                    .await;
                let _ = client
                    .post_json(
                        &format!("/runtime/leases/{}/logs", lease_id),
                        &json!({"stream":"stderr","content":line}),
                    )
                    .await;
            }
        });
    }
    let status = child.wait().await.context("runtime job wait failed")?;
    info!(
        exit_code = status.code().unwrap_or(1),
        runtime_class,
        "job complete"
    );
    Ok(status.code().unwrap_or(1))
}

async fn execute_pty_job(
    client: &MissionControlClient,
    session_id: &str,
    lease_id: &str,
    command: &str,
    args: &[String],
    env_map: serde_json::Map<String, Value>,
    cwd: &str,
) -> Result<i32> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let mut builder = if command.trim().is_empty() {
        CommandBuilder::new("sh")
    } else {
        CommandBuilder::new(command)
    };
    if !command.trim().is_empty() {
        for arg in args {
            builder.arg(arg);
        }
    } else {
        builder.arg("-lc");
        builder.arg("true");
    }
    for (key, value) in env_map {
        if let Some(value) = value.as_str() {
            builder.env(key, value);
        }
    }
    if !cwd.trim().is_empty() {
        builder.cwd(cwd);
    }
    let mut child = pair.slave.spawn_command(builder)?;
    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let url = client.ws_url(&format!("/runtime/execution-sessions/{}/pty", session_id))?;
    let (mut ws, _) = connect_async(url.as_str()).await?;
    let stdin_task = tokio::spawn(async move {
        while let Some(msg) = ws.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&text) {
                        if value.get("type").and_then(Value::as_str) == Some("input") {
                            if let Some(content) = value.get("content").and_then(Value::as_str) {
                                let _ = writer.write_all(content.as_bytes());
                            }
                        }
                    }
                }
                Ok(Message::Binary(bytes)) => {
                    let _ = writer.write_all(&bytes);
                }
                _ => break,
            }
        }
    });
    let mut buf = [0u8; 4096];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let text = String::from_utf8_lossy(&buf[..n]).to_string();
        let _ = client
            .post_json(
                &format!("/runtime/leases/{}/logs", lease_id),
                &json!({"stream":"pty","content":text}),
            )
            .await;
    }
    let status = child.wait()?;
    stdin_task.abort();
    Ok(status.exit_code() as i32)
}

async fn attach_session(
    args: RuntimeSessionAttachArgs,
    client: &MissionControlClient,
) -> Result<()> {
    let mut url = client.ws_url(&format!(
        "/runtime/execution-sessions/{}/pty",
        args.session_id
    ))?;
    if let Some(token) = client.token() {
        url.query_pairs_mut().append_pair("token", token);
    }
    let (ws, _) = connect_async(url.as_str()).await?;
    let (mut sink, mut stream) = ws.split();
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    let writer = tokio::spawn(async move {
        let mut buf = [0u8; 1024];
        loop {
            let n = stdin.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            let text = String::from_utf8_lossy(&buf[..n]).to_string();
            sink.send(Message::Text(
                json!({"type":"input","content":text}).to_string(),
            ))
            .await
            .map_err(|err| anyhow::anyhow!(err))?;
        }
        Ok::<(), anyhow::Error>(())
    });

    while let Some(msg) = stream.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    if let Some(content) = value.get("content").and_then(Value::as_str) {
                        stdout.write_all(content.as_bytes()).await?;
                        stdout.flush().await?;
                    }
                }
            }
            Message::Binary(bytes) => {
                stdout.write_all(&bytes).await?;
                stdout.flush().await?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    writer.abort();
    Ok(())
}

/// Returns the absolute path of `name` if found on PATH, otherwise None.
fn which_binary(name: &str) -> Option<std::path::PathBuf> {
    std::env::var("PATH").ok().and_then(|path_env| {
        path_env.split(':').find_map(|dir| {
            let candidate = std::path::Path::new(dir).join(name);
            if candidate.is_file() { Some(candidate) } else { None }
        })
    })
}

/// Build a Goose subprocess command from a `RuntimeJob` with `runtime_class="goose"`.
///
/// The `command` field of the job carries one of:
/// - An absolute path or a `.yaml`/`.json` suffix → passed directly as `--recipe`
/// - A JSON object or YAML header (`version:`) → written to a temp file, passed as `--recipe`
/// - Anything else → passed as `--text` (headless prompt)
///
/// `LITELLM_HOST` and `LITELLM_API_KEY` are read from the job `env`; GOOSE_PROVIDER is
/// forced to `litellm`.  All other env entries are forwarded as-is.
fn build_goose_command(
    command: &str,
    env_map: &serde_json::Map<String, Value>,
    cwd: &str,
) -> Result<Command> {
    let goose_bin = std::env::var("GOOSE_BIN").unwrap_or_else(|_| "goose".into());
    let litellm_host = env_map
        .get("LITELLM_HOST")
        .and_then(Value::as_str)
        .unwrap_or("http://litellm:4000");
    let litellm_api_key = env_map
        .get("LITELLM_API_KEY")
        .and_then(Value::as_str)
        .unwrap_or("sk-goose");
    let goose_model = env_map
        .get("GOOSE_MODEL")
        .and_then(Value::as_str)
        .map(|s| s.to_owned())
        .unwrap_or_else(|| {
            std::env::var("MC_GOOSE_MODEL").unwrap_or_else(|_| "local-agent".to_owned())
        });

    let trimmed = command.trim();

    let mut cmd = Command::new(&goose_bin);
    cmd.arg("run");

    if trimmed.starts_with('/') || trimmed.ends_with(".yaml") || trimmed.ends_with(".json") {
        // Explicit recipe file path
        cmd.arg("--recipe").arg(trimmed);
    } else if trimmed.starts_with('{') || trimmed.starts_with("version:") {
        // Inline recipe content — write to a temp file
        let tmp = std::env::temp_dir().join(format!("mc-goose-{}.json", Uuid::new_v4()));
        fs::write(&tmp, trimmed.as_bytes()).context("failed to write inline goose recipe")?;
        cmd.arg("--recipe").arg(tmp);
    } else if !trimmed.is_empty() {
        cmd.arg("--text").arg(trimmed);
    }

    cmd.arg("--quiet")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--no-session")
        .arg("--max-turns")
        .arg("50");

    cmd.env("GOOSE_PROVIDER", "litellm")
        .env("LITELLM_HOST", litellm_host)
        .env("LITELLM_API_KEY", litellm_api_key)
        .env("GOOSE_MODEL", goose_model)
        .env("GOOSE_MODE", "Auto");

    for (key, value) in env_map {
        if let Some(v) = value.as_str() {
            cmd.env(key, v);
        }
    }

    if !cwd.trim().is_empty() {
        cmd.current_dir(cwd);
    }

    Ok(cmd)
}

fn build_host_command(
    command: &str,
    args: &[String],
    env_map: &serde_json::Map<String, Value>,
    cwd: &str,
) -> Command {
    let mut cmd = if command.trim().is_empty() {
        let mut shell = Command::new("sh");
        shell.arg("-lc").arg("true");
        shell
    } else {
        let mut process = Command::new(command);
        process.args(args);
        process
    };
    if !cwd.trim().is_empty() {
        cmd.current_dir(cwd);
    }
    for (key, value) in env_map {
        if let Some(value) = value.as_str() {
            cmd.env(key, value);
        }
    }
    cmd
}

fn build_container_command(
    command: &str,
    args: &[String],
    env_map: &serde_json::Map<String, Value>,
    cwd: &str,
) -> Result<Command> {
    let runtime = container_runtime_binary()
        .ok_or_else(|| anyhow::anyhow!("no container runtime found on PATH"))?;
    let mut cmd = Command::new(runtime);
    cmd.arg("run").arg("--rm").arg("-i");
    if !cwd.trim().is_empty() {
        cmd.arg("-w").arg(cwd);
    }
    for (key, value) in env_map {
        if let Some(value) = value.as_str() {
            cmd.arg("-e").arg(format!("{key}={value}"));
        }
    }
    let image = default_container_image();
    if image.is_empty() {
        return Err(anyhow::anyhow!("container job missing image"));
    }
    cmd.arg(image);
    if command.trim().is_empty() {
        cmd.arg("sh").arg("-lc").arg("true");
    } else {
        cmd.arg(command);
        cmd.args(args);
    }
    Ok(cmd)
}

fn container_runtime_binary() -> Option<String> {
    ["docker", "podman"].iter().find_map(|candidate| {
        std::process::Command::new("sh")
            .arg("-lc")
            .arg(format!("command -v {candidate} >/dev/null 2>&1"))
            .status()
            .ok()
            .and_then(|status| {
                if status.success() {
                    Some((*candidate).to_string())
                } else {
                    None
                }
            })
    })
}

fn default_container_image() -> String {
    std::env::var("MC_RUNTIME_DEFAULT_IMAGE").unwrap_or_else(|_| "alpine:3".to_string())
}

fn parse_kv_pairs(input: &str) -> Value {
    let mut map = serde_json::Map::new();
    for item in input.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            map.insert(
                key.trim().to_string(),
                Value::String(value.trim().to_string()),
            );
        }
    }
    Value::Object(map)
}
