use crate::client::MissionControlClient;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{Value, json};
use std::fs;

#[derive(Subcommand, Debug)]
pub enum GovernanceCommand {
    /// Manage mission-level roles and memberships.
    #[command(subcommand)]
    Roles(RolesCommand),
    /// Work with governance policies.
    #[command(subcommand)]
    Policy(PolicyCommand),
    /// Inspect governance policy events.
    Events(EventsArgs),
}

#[derive(Subcommand, Debug)]
pub enum RolesCommand {
    /// List role assignments for a mission.
    List(RolesListArgs),
    /// Add or update a role.
    Upsert(RolesUpsertArgs),
    /// Remove a role assignment.
    Remove(RolesRemoveArgs),
}

#[derive(Args, Debug)]
pub struct RolesListArgs {
    #[arg(long)]
    pub mission_id: String,
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
}

#[derive(Args, Debug)]
pub struct RolesUpsertArgs {
    #[arg(long)]
    pub mission_id: String,
    #[arg(long)]
    pub subject: String,
    #[arg(long)]
    pub role: String,
}

#[derive(Args, Debug)]
pub struct RolesRemoveArgs {
    #[arg(long)]
    pub mission_id: String,
    #[arg(long)]
    pub subject: String,
}

#[derive(Subcommand, Debug)]
pub enum PolicyCommand {
    /// Show the active governance policy.
    Active,
    /// List historical policy versions.
    Versions {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Create a new draft from JSON file.
    CreateDraft(CreateDraftArgs),
    /// Publish an existing draft.
    Publish {
        #[arg(long)]
        draft_id: u32,
        #[arg(long)]
        change_note: Option<String>,
    },
    /// Roll back to a specific version.
    Rollback {
        #[arg(long)]
        version: u32,
        #[arg(long)]
        change_note: Option<String>,
    },
    /// Reload the active policy config.
    Reload,
}

#[derive(Args, Debug)]
pub struct CreateDraftArgs {
    #[arg(long)]
    pub file: String,
    #[arg(long)]
    pub change_note: Option<String>,
}

#[derive(Args, Debug)]
pub struct EventsArgs {
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
}

pub async fn run(command: GovernanceCommand, client: &MissionControlClient) -> Result<()> {
    match command {
        GovernanceCommand::Roles(inner) => run_roles(inner, client).await,
        GovernanceCommand::Policy(inner) => run_policy(inner, client).await,
        GovernanceCommand::Events(args) => run_events(args, client).await,
    }
}

async fn run_roles(command: RolesCommand, client: &MissionControlClient) -> Result<()> {
    match command {
        RolesCommand::List(args) => {
            let path = format!("/missions/{}/roles?limit={}", args.mission_id, args.limit);
            let payload = client.get_json(&path).await?;
            print_json(&payload);
        }
        RolesCommand::Upsert(args) => {
            let body = json!({
                "subject": args.subject,
                "role": args.role,
            });
            let path = format!("/missions/{}/roles", args.mission_id);
            let payload = client.post_json(&path, &body).await?;
            print_json(&payload);
        }
        RolesCommand::Remove(args) => {
            let path = format!("/missions/{}/roles/{}", args.mission_id, args.subject);
            let payload = client
                .request_builder(Method::DELETE, &path)?
                .send()
                .await?
                .error_for_status()?
                .json::<Value>()
                .await?;
            print_json(&payload);
        }
    }
    Ok(())
}

async fn run_policy(command: PolicyCommand, client: &MissionControlClient) -> Result<()> {
    match command {
        PolicyCommand::Active => {
            let payload = client.get_json("/governance/policy/active").await?;
            print_json(&payload);
        }
        PolicyCommand::Versions { limit } => {
            let path = format!("/governance/policy/versions?limit={}", limit);
            let payload = client.get_json(&path).await?;
            print_json(&payload);
        }
        PolicyCommand::CreateDraft(args) => {
            let policy = fs::read_to_string(&args.file)
                .with_context(|| format!("failed to read {}", args.file))?;
            let body = json!({
                "policy": serde_json::from_str::<Value>(&policy)
                    .context("policy file must contain JSON")?,
                "change_note": args.change_note,
            });
            let payload = client.post_json("/governance/policy/drafts", &body).await?;
            print_json(&payload);
        }
        PolicyCommand::Publish {
            draft_id,
            change_note,
        } => {
            let body = json!({ "change_note": change_note });
            let path = format!("/governance/policy/drafts/{}/publish", draft_id);
            let payload = client.post_json(&path, &body).await?;
            print_json(&payload);
        }
        PolicyCommand::Rollback {
            version,
            change_note,
        } => {
            let body = json!({
                "version": version,
                "change_note": change_note,
            });
            let payload = client
                .post_json("/governance/policy/rollback", &body)
                .await?;
            print_json(&payload);
        }
        PolicyCommand::Reload => {
            let payload = client
                .post_json("/governance/policy/reload", &json!({}))
                .await?;
            print_json(&payload);
        }
    }
    Ok(())
}

async fn run_events(args: EventsArgs, client: &MissionControlClient) -> Result<()> {
    let path = format!("/governance/policy/events?limit={}", args.limit);
    let payload = client.get_json(&path).await?;
    print_json(&payload);
    Ok(())
}

fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}
