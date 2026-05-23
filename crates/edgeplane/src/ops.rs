use crate::{
    booster::AgentBooster, client::EdgeplaneClient, mcp_tools, schema_pack::SchemaPack,
};
use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Value, json};

#[derive(Subcommand, Debug)]
pub enum OpsCommand {
    /// Domain-level lifecycle actions that build on workspace leases.
    Domain(DomainOpsArgs),
}

#[derive(Args, Debug)]
pub struct DomainOpsArgs {
    /// Domain action to execute.
    #[arg(long, value_enum)]
    pub action: DomainAction,

    /// Target mission (required for start).
    #[arg(long)]
    pub mission_id: Option<String>,

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
pub enum DomainAction {
    Start,
    Heartbeat,
    Commit,
    Release,
}

pub async fn run(
    command: OpsCommand,
    client: &EdgeplaneClient,
    booster: &AgentBooster,
    schema_pack: &SchemaPack,
) -> Result<()> {
    match command {
        OpsCommand::Domain(args) => run_domain(args, client, booster, schema_pack).await,
    }
}

async fn run_domain(
    args: DomainOpsArgs,
    client: &EdgeplaneClient,
    booster: &AgentBooster,
    schema_pack: &SchemaPack,
) -> Result<()> {
    match args.action {
        DomainAction::Start => {
            let mission_id = args
                .mission_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("--mission-id is required for start"))?;
            let mut payload = json!({ "mission_id": mission_id });
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
                "load_mission_workspace",
                payload,
            )
            .await?;
            print_json(&response);
        }
        DomainAction::Heartbeat => {
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
        DomainAction::Commit => {
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
                "commit_mission_workspace",
                payload,
            )
            .await?;
            print_json(&response);
        }
        DomainAction::Release => {
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
                "release_mission_workspace",
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
