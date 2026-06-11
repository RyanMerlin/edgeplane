use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::client::EdgeplaneClient;

/// `edgeplane agent evolve` — self-improvement loop: run agents against EdgePlane's own backlog.
#[derive(Args, Debug)]
pub struct EvolveArgs {
    #[command(subcommand)]
    pub command: EvolveCommand,
}

#[derive(Subcommand, Debug)]
pub enum EvolveCommand {
    /// Seed an evolve domain from a JSON spec file.
    Seed(SeedArgs),
    /// Launch an agent against an evolve domain.
    Run(RunArgs),
    /// Show evolve domain progress.
    Status(StatusArgs),
}

#[derive(Args, Debug)]
pub struct SeedArgs {
    /// JSON spec file defining the evolve domain and task backlog.
    #[arg(long)]
    pub spec: String,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Domain ID to run agents against.
    #[arg(long)]
    pub domain: String,

    /// Agent to use (claude, codex, gemini, openclaw).
    #[arg(long, default_value = "claude")]
    pub agent: String,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Domain ID to inspect.
    #[arg(long)]
    pub domain: String,
}

pub async fn run(args: EvolveArgs, client: &EdgeplaneClient) -> Result<()> {
    match args.command {
        EvolveCommand::Seed(a) => seed(a, client).await,
        EvolveCommand::Run(a) => run_domain(a, client).await,
        EvolveCommand::Status(a) => status(a, client).await,
    }
}

async fn seed(args: SeedArgs, client: &EdgeplaneClient) -> Result<()> {
    let spec_content = std::fs::read_to_string(&args.spec)
        .map_err(|e| anyhow::anyhow!("cannot read spec file {}: {}", args.spec, e))?;
    let spec: Value = serde_json::from_str(&spec_content)
        .map_err(|e| anyhow::anyhow!("spec must be valid JSON: {}", e))?;
    let body = json!({ "spec": spec });
    let response = client.post_json("/evolve/domains", &body).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn run_domain(args: RunArgs, client: &EdgeplaneClient) -> Result<()> {
    let body = json!({ "runtime_kind": args.agent });
    let path = format!("/evolve/domains/{}/run", args.domain);
    let response = client.post_json(&path, &body).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

async fn status(args: StatusArgs, client: &EdgeplaneClient) -> Result<()> {
    let path = format!("/evolve/domains/{}/status", args.domain);
    let response = client.get_json(&path).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
