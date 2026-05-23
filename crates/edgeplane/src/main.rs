use clap::Parser;
use edgeplane::{
    auth::resolve_startup_base_url, booster::AgentBooster, client::EdgeplaneClient,
    commands::McCommand, config::McConfig, output::OutputMode, secrets,
};
use tracing::Level;
use tracing_subscriber::{EnvFilter, fmt};

const DEFAULT_BASE_URL: &str = "http://localhost:8008";

/// Top-level CLI options that control the edgeplane experience.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct CliOpts {
    /// Base URL pointing at an existing Edgeplane deployment.
    /// If omitted, falls back to EP_BASE_URL env, then ~/.ep/config.json,
    /// then http://localhost:8008.
    #[arg(long, env = "EP_BASE_URL")]
    base_url: Option<String>,

    /// Optional agent identifier that is propagated throughout approvals and sync calls.
    #[arg(long, env = "EP_AGENT_ID")]
    agent_id: Option<String>,

    /// Optional runtime session identifier propagated for per-instance attribution.
    #[arg(long, env = "EP_RUNTIME_SESSION_ID")]
    runtime_session_id: Option<String>,

    /// Optional profile name propagated for per-profile attribution.
    #[arg(long, env = "EP_AGENT_PROFILE")]
    profile_name: Option<String>,

    /// Timeout (in seconds) for all outbound calls.
    #[arg(long, env = "EP_TIMEOUT_SECS", default_value_t = 10)]
    timeout_secs: u64,

    /// Allow invalid TLS certificates when running against local or self-signed endpoints.
    #[arg(long, env = "EP_ALLOW_INSECURE", default_value_t = false)]
    allow_insecure: bool,

    /// Optional WASM booster module path.
    #[arg(long, env = "EP_BOOSTER_WASM")]
    booster_wasm: Option<std::path::PathBuf>,

    /// Disable the booster hook even if a module is configured.
    #[arg(long, env = "EP_DISABLE_BOOSTER", default_value_t = false)]
    disable_booster: bool,

    /// Allow booster modules to short-circuit MCP tool execution.
    /// Disabled by default so authoritative reads/mutations always hit Edgeplane.
    #[arg(long, env = "EP_ALLOW_BOOSTER_SHORT_CIRCUIT", default_value_t = false)]
    allow_booster_short_circuit: bool,

    /// Emit machine-readable JSON output.
    #[arg(long, global = true, default_value_t = false)]
    json: bool,

    #[command(subcommand)]
    command: McCommand,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .init();

    let opts = CliOpts::parse();

    // Resolve base_url: flag/env → ~/.ep/config.json → hardcoded default.
    let base_url = resolve_startup_base_url(opts.base_url.clone(), DEFAULT_BASE_URL);

    // Token comes exclusively from ~/.ep/session.json (written by `edgeplane auth login` OIDC flow).
    let token = edgeplane::config::load_session_token(&base_url);
    if token.is_some() {
        tracing::debug!("session token available from ~/.ep/session.json");
    }
    let token = if let Some(raw) = token {
        Some(
            secrets::resolve_maybe_secret_ref(&raw)
                .await
                .map_err(|e| anyhow::anyhow!("failed to resolve MC token secret ref: {e}"))?,
        )
    } else {
        None
    };

    let config = McConfig::from_parts(
        &base_url,
        token,
        opts.agent_id.clone(),
        opts.runtime_session_id.clone(),
        opts.profile_name.clone(),
        opts.timeout_secs,
        opts.allow_insecure,
        !opts.disable_booster,
        opts.allow_booster_short_circuit,
        opts.booster_wasm.clone(),
    )?;
    let client = EdgeplaneClient::new(&config)?;
    let booster = AgentBooster::load(&config)?;

    let output_mode = if opts.json {
        OutputMode::Json
    } else {
        OutputMode::Human
    };

    edgeplane::commands::run(opts.command, client, booster, config, output_mode).await
}
