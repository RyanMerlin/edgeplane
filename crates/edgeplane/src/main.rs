use clap::Parser;
use edgeplane::{
    auth::resolve_startup_base_url, booster::AgentBooster, client::EdgeplaneClient,
    commands::CliRoot, config::EdgeplaneConfig, output::OutputMode, secrets,
};
use tracing::Level;
use tracing_subscriber::{EnvFilter, fmt};

const DEFAULT_BASE_URL: &str = "http://localhost:8008";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::from_default_env().add_directive(Level::INFO.into()))
        .init();

    let opts = CliRoot::parse();

    // Self-heal any legacy CLI config/session files stranded at the old
    // `~/.edgeplane/` root by the daemon-only, sentinel-gated path migration.
    // Idempotent + soft-fail; must run before context/base_url resolution.
    edgeplane::migrate::heal_legacy_cli_paths();

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

    let config = EdgeplaneConfig::from_parts(
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
