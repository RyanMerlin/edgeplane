use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::client::MissionControlClient;

/// `mc agent evolve` — self-improvement loop: run agents against MC's own backlog.
#[derive(Args, Debug)]
pub struct EvolveArgs {
    #[command(subcommand)]
    pub command: EvolveCommand,
}

#[derive(Subcommand, Debug)]
pub enum EvolveCommand {
    /// Seed an evolve mission from a JSON spec file.
    Seed(SeedArgs),
    /// Launch an agent against an evolve mission.
    Run(RunArgs),
    /// Show evolve mission progress.
    Status(StatusArgs),
}

#[derive(Args, Debug)]
pub struct SeedArgs {
    /// JSON spec file defining the evolve mission and task backlog.
    #[arg(long)]
    pub spec: String,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Mission ID to run agents against.
    #[arg(long)]
    pub mission: String,

    /// Agent to use (claude, codex, gemini, openclaw).
    #[arg(long, default_value = "claude")]
    pub agent: String,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Mission ID to inspect.
    #[arg(long)]
    pub mission: String,
}

pub async fn run(args: EvolveArgs, client: &MissionControlClient) -> Result<()> {
    match args.command {
        EvolveCommand::Seed(a) => seed(a, client).await,
        EvolveCommand::Run(a) => run_mission(a, client).await,
        EvolveCommand::Status(a) => status(a, client).await,
    }
}

async fn seed(args: SeedArgs, client: &MissionControlClient) -> Result<()> {
    let spec_content = std::fs::read_to_string(&args.spec)
        .map_err(|e| anyhow::anyhow!("cannot read spec file {}: {}", args.spec, e))?;
    let spec: Value = serde_json::from_str(&spec_content)
        .map_err(|e| anyhow::anyhow!("spec must be valid JSON: {}", e))?;
    let body = json!({ "spec": spec });
    let response = client.post_json("/evolve/missions", &body).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn run_mission(args: RunArgs, client: &MissionControlClient) -> Result<()> {
    let body = json!({ "runtime_kind": args.agent });
    let path = format!("/evolve/missions/{}/run", args.mission);
    let response = client.post_json(&path, &body).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn status(args: StatusArgs, client: &MissionControlClient) -> Result<()> {
    let path = format!("/evolve/missions/{}/status", args.mission);
    let response = client.get_json(&path).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
