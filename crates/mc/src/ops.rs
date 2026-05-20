use crate::{
    booster::AgentBooster, client::MissionControlClient, mcp_tools, schema_pack::SchemaPack,
};
use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Value, json};

#[derive(Subcommand, Debug)]
pub enum OpsCommand {
    /// Mission-level lifecycle actions that build on workspace leases.
    Mission(MissionOpsArgs),
}

#[derive(Args, Debug)]
pub struct MissionOpsArgs {
    /// Mission action to execute.
    #[arg(long, value_enum)]
    pub action: MissionAction,

    /// Target kluster (required for start).
    #[arg(long)]
    pub kluster_id: Option<String>,

    /// Lease ID to manage.
    #[arg(long)]
    pub lease_id: Option<String>,

    /// Optional workspace label created during start.
    #[arg(long)]
    pub workspace_label: Option<String>,

    /// Optional agent identifier for the lease.
    #[arg(long)]
    pub agent_id: Option<String>,

    /// Lease duration in seconds.
    #[arg(long)]
    pub lease_seconds: Option<u32>,

    /// Change set JSON for commits.
    #[arg(long, default_value = "{}")]
    pub change_set: String,

    /// Validation mode used when committing.
    #[arg(long)]
    pub validation_mode: Option<String>,

    /// Optional release reason.
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum MissionAction {
    Start,
    Heartbeat,
    Commit,
    Release,
}

pub async fn run(
    command: OpsCommand,
    client: &MissionControlClient,
    booster: &AgentBooster,
    schema_pack: &SchemaPack,
) -> Result<()> {
    match command {
        OpsCommand::Mission(args) => run_mission(args, client, booster, schema_pack).await,
    }
}

async fn run_mission(
    args: MissionOpsArgs,
    client: &MissionControlClient,
    booster: &AgentBooster,
    schema_pack: &SchemaPack,
) -> Result<()> {
    match args.action {
        MissionAction::Start => {
            let kluster_id = args
                .kluster_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--kluster-id is required for start"))?;
            let mut payload = json!({ "kluster_id": kluster_id });
            if let Some(label) = args.workspace_label {
                payload["workspace_label"] = json!(label);
            }
            if let Some(agent_id) = args.agent_id {
                payload["agent_id"] = json!(agent_id);
            }
            if let Some(seconds) = args.lease_seconds {
                payload["lease_seconds"] = json!(seconds);
            }
            let response = mcp_tools::call_tool(
                client,
                Some(booster),
                Some(schema_pack),
                "load_kluster_workspace",
                payload,
            )
            .await?;
            print_json(&response);
        }
        MissionAction::Heartbeat => {
            let lease_id = args
                .lease_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--lease-id is required for heartbeat"))?;
            let payload = json!({ "lease_id": lease_id });
            let response = mcp_tools::call_tool(
                client,
                Some(booster),
                Some(schema_pack),
                "heartbeat_workspace_lease",
                payload,
            )
            .await?;
            print_json(&response);
        }
        MissionAction::Commit => {
            let lease_id = args
                .lease_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--lease-id is required for commit"))?;
            let change_set: Value =
                serde_json::from_str(&args.change_set).context("change-set must be valid JSON")?;
            let mut payload = json!({
                "lease_id": lease_id,
                "change_set": change_set,
            });
            if let Some(mode) = args.validation_mode {
                payload["validation_mode"] = json!(mode);
            }
            let response = mcp_tools::call_tool(
                client,
                Some(booster),
                Some(schema_pack),
                "commit_kluster_workspace",
                payload,
            )
            .await?;
            print_json(&response);
        }
        MissionAction::Release => {
            let lease_id = args
                .lease_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--lease-id is required for release"))?;
            let mut payload = json!({ "lease_id": lease_id });
            if let Some(reason) = args.reason {
                payload["reason"] = json!(reason);
            }
            let response = mcp_tools::call_tool(
                client,
                Some(booster),
                Some(schema_pack),
                "release_kluster_workspace",
                payload,
            )
            .await?;
            print_json(&response);
        }
    }
    Ok(())
}

fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}
