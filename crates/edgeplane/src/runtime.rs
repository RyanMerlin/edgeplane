use crate::{client::EdgeplaneClient, output, output::OutputMode};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

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
    /// Register a node with Edgeplane and persist its identity locally.
    Register(NodeAgentRegisterArgs),
    /// [removed] The node daemon is now `edgeplaned` — use `edgeplaned run`.
    Run(NodeAgentRunArgs),
    /// Inspect local node-agent readiness.
    Doctor(NodeAgentDoctorArgs),
    /// Manage node join tokens (single-use bootstrap credentials).
    #[command(subcommand)]
    JoinToken(JoinTokenCommand),
    /// Delete a runtime node from the controlplane.
    Delete(NodeDeleteArgs),
    /// List runtime nodes visible to the current principal.
    Ls(NodeLsArgs),
}

#[derive(Subcommand, Debug)]
pub enum JoinTokenCommand {
    /// Create a new join token for bootstrapping a node.
    Create(JoinTokenCreateArgs),
    /// Get a join token by ID.
    Get(JoinTokenGetArgs),
    /// Rotate a join token (invalidates the old one, issues a new secret).
    Rotate(JoinTokenGetArgs),
}

#[derive(Args, Debug)]
pub struct JoinTokenCreateArgs {
    /// Token TTL in seconds (default: 600 — 10 minutes).
    #[arg(long, default_value = "600")]
    pub ttl_seconds: u32,
}

#[derive(Args, Debug)]
pub struct JoinTokenGetArgs {
    /// Join token ID (returned by `create`).
    pub token_id: String,
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
    pub hostname: String,
    #[arg(long, default_value = "untrusted")]
    pub trust_tier: String,
}

#[derive(Args, Debug)]
pub struct NodeAgentRegisterArgs {
    #[arg(long)]
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
    pub domain_id: String,
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

#[derive(Args, Debug)]
pub struct NodeDeleteArgs {
    /// Node ID to delete.
    pub node_id: String,
    /// Detach assigned agents before deleting.  Without this flag the request
    /// is refused with an error if any meshagent rows are assigned to the node.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

#[derive(Args, Debug, Default)]
pub struct NodeLsArgs {
    /// Filter by node status (e.g. `online`, `offline`, `registered`).
    #[arg(long)]
    pub status: Option<String>,
}

pub async fn run(
    command: RuntimeCommand,
    client: &EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        RuntimeCommand::Nodes(cmd) => run_nodes(cmd, client, output_mode).await,
        RuntimeCommand::Jobs(cmd) => run_jobs(cmd, client, output_mode).await,
        RuntimeCommand::Leases(cmd) => run_leases(cmd, client, output_mode).await,
        RuntimeCommand::Sessions(cmd) => run_sessions(cmd, client, output_mode).await,
    }
}

pub async fn run_node_agent(command: NodeAgentCommand, client: &EdgeplaneClient) -> Result<()> {
    match command {
        NodeAgentCommand::Register(args) => run_node_register(args, client).await,
        NodeAgentCommand::Run(args) => run_node_run(args, client).await,
        NodeAgentCommand::Doctor(args) => run_node_doctor(args).await,
        NodeAgentCommand::JoinToken(cmd) => run_join_token(cmd, client).await,
        NodeAgentCommand::Delete(args) => run_node_delete(args, client).await,
        NodeAgentCommand::Ls(args) => run_node_ls(args, client).await,
    }
}

async fn run_join_token(command: JoinTokenCommand, client: &EdgeplaneClient) -> Result<()> {
    match command {
        JoinTokenCommand::Create(args) => {
            let resp = client
                .post_json(
                    "/runtime/join-tokens",
                    &json!({"ttl_seconds": args.ttl_seconds}),
                )
                .await
                .context("create join token")?;
            // Print the token plaintext prominently — it's shown only once.
            if let Some(token) = resp.get("token").and_then(|v| v.as_str()) {
                println!("Join token (plaintext — copy now, not stored):");
                println!("{token}");
                println!();
            }
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        JoinTokenCommand::Get(args) => {
            let resp = client
                .get_json(&format!("/runtime/join-tokens/{}", args.token_id))
                .await
                .context("get join token")?;
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
        JoinTokenCommand::Rotate(args) => {
            let resp = client
                .post_json(
                    &format!("/runtime/join-tokens/{}/rotate", args.token_id),
                    &json!({}),
                )
                .await
                .context("rotate join token")?;
            if let Some(token) = resp.get("token").and_then(|v| v.as_str()) {
                println!("New join token (plaintext — copy now, not stored):");
                println!("{token}");
                println!();
            }
            println!("{}", serde_json::to_string_pretty(&resp)?);
        }
    }
    Ok(())
}

async fn run_nodes(
    command: RuntimeNodesCommand,
    client: &EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        RuntimeNodesCommand::Register(args) => {
            let response = client
                .post_json(
                    "/runtime/nodes/register",
                    &json!({"node_name": args.hostname, "hostname": args.hostname, "trust_tier": args.trust_tier}),
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
    client: &EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        RuntimeJobsCommand::Submit(args) => {
            let response = client
                .post_json(
                    "/runtime/jobs",
                    &json!({"domain_id": args.domain_id,"runtime_session_id": args.runtime_session_id,"runtime_class": args.runtime_class,"image": args.image,"command": args.command}),
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
    client: &EdgeplaneClient,
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
    client: &EdgeplaneClient,
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
    crate::config::ep_home_dir()
        .join("runtime")
        .join("node.json")
}

fn node_config_path() -> PathBuf {
    crate::config::ep_home_dir()
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

async fn run_node_register(args: NodeAgentRegisterArgs, client: &EdgeplaneClient) -> Result<()> {
    let config = load_node_config()?.unwrap_or_else(|| NodeRuntimeConfig {
        node_name: args.hostname.clone(),
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

const NODE_RUN_REMOVED_MSG: &str = "`edgeplane node run` has been removed — the node daemon is now `edgeplaned`.\n\
\n\
Enroll and run a node with:\n\
\x20  edgeplaned register --join-token <TOKEN> --endpoint <TOWER_URL>\n\
\x20  edgeplaned run\n\
\n\
See https://github.com/RyanMerlin/edgeplane/tree/main/crates/edgeplaned";

async fn run_node_run(_args: NodeAgentRunArgs, _client: &EdgeplaneClient) -> Result<()> {
    Err(anyhow::anyhow!(NODE_RUN_REMOVED_MSG))
}

async fn run_node_delete(args: NodeDeleteArgs, client: &EdgeplaneClient) -> Result<()> {
    let path = if args.force {
        format!("/runtime/nodes/{}?force=true", args.node_id)
    } else {
        format!("/runtime/nodes/{}", args.node_id)
    };

    // The server returns 409 + JSON detail when agents are assigned and force
    // is not set.  check_response turns non-2xx into an anyhow error carrying
    // the server's text body, which already includes the human-readable hint.
    // We intercept that error to surface a cleaner message.
    match client.delete_json(&path).await {
        Ok(summary) => {
            println!("{}", serde_json::to_string_pretty(&summary)?);
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("409") {
                // Re-surface the server hint without a raw stack trace.
                anyhow::bail!(
                    "refused: node has assigned agents — re-run with --force to detach and delete\n\
                     Server detail: {msg}"
                );
            }
            return Err(e);
        }
    }
    Ok(())
}

async fn run_node_ls(args: NodeLsArgs, client: &EdgeplaneClient) -> Result<()> {
    let path = match args.status.as_deref() {
        Some(s) if !s.trim().is_empty() => format!("/runtime/nodes?status={s}"),
        _ => "/runtime/nodes".to_string(),
    };
    let nodes = client.get_json(&path).await?;
    let arr = nodes.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        println!("(no nodes)");
        return Ok(());
    }
    // Print a concise table: id | name | status | last_heartbeat_at
    println!(
        "{:<40}  {:<24}  {:<12}  LAST_HEARTBEAT",
        "ID", "NAME", "STATUS"
    );
    println!("{}", "-".repeat(100));
    for node in &arr {
        let id = node["id"].as_str().unwrap_or("-");
        let name = node["node_name"].as_str().unwrap_or("-");
        let status = node["status"].as_str().unwrap_or("-");
        let heartbeat = node["last_heartbeat_at"].as_str().unwrap_or("-");
        println!("{id:<40}  {name:<24}  {status:<12}  {heartbeat}");
    }
    Ok(())
}

async fn attach_session(args: RuntimeSessionAttachArgs, client: &EdgeplaneClient) -> Result<()> {
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
                if let Ok(value) = serde_json::from_str::<Value>(&text)
                    && let Some(content) = value.get("content").and_then(Value::as_str)
                {
                    stdout.write_all(content.as_bytes()).await?;
                    stdout.flush().await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn node_run_removed_message_points_to_edgeplaned() {
        assert!(super::NODE_RUN_REMOVED_MSG.contains("edgeplaned register"));
        assert!(super::NODE_RUN_REMOVED_MSG.contains("edgeplaned run"));
        assert!(super::NODE_RUN_REMOVED_MSG.contains("removed"));
    }

    // ── node delete arg-parse ────────────────────────────────────────────────

    /// Minimal CLI wrapper used only for arg-parse tests — mirrors the shape
    /// of the real `edgeplane agent node` subcommand.
    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(subcommand)]
        cmd: NodeAgentCommand,
    }

    #[test]
    fn node_delete_parses_node_id() {
        let cli = TestCli::try_parse_from(["test", "delete", "node-abc-123"]).unwrap();
        let NodeAgentCommand::Delete(args) = cli.cmd else {
            panic!("expected Delete");
        };
        assert_eq!(args.node_id, "node-abc-123");
        assert!(!args.force);
    }

    #[test]
    fn node_delete_parses_force_flag() {
        let cli = TestCli::try_parse_from(["test", "delete", "node-xyz", "--force"]).unwrap();
        let NodeAgentCommand::Delete(args) = cli.cmd else {
            panic!("expected Delete");
        };
        assert_eq!(args.node_id, "node-xyz");
        assert!(args.force);
    }

    #[test]
    fn node_delete_requires_node_id() {
        // Missing positional arg must produce a parse error, not a panic.
        assert!(TestCli::try_parse_from(["test", "delete"]).is_err());
    }

    #[test]
    fn node_ls_parses_no_args() {
        let cli = TestCli::try_parse_from(["test", "ls"]).unwrap();
        let NodeAgentCommand::Ls(args) = cli.cmd else {
            panic!("expected Ls");
        };
        assert!(args.status.is_none());
    }

    #[test]
    fn node_ls_parses_status_filter() {
        let cli = TestCli::try_parse_from(["test", "ls", "--status", "online"]).unwrap();
        let NodeAgentCommand::Ls(args) = cli.cmd else {
            panic!("expected Ls");
        };
        assert_eq!(args.status.as_deref(), Some("online"));
    }
}
