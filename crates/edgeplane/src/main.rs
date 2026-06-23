use clap::Parser;
use edgeplane::{
    auth::resolve_startup_base_url, booster::AgentBooster, client::EdgeplaneClient,
    commands::CliRoot, config::EdgeplaneConfig, output::OutputMode, secrets,
};
use tracing::Level;
use tracing_subscriber::{EnvFilter, fmt};

/// Placeholder URL used for offline/bootstrap commands that never dial a server.
/// Using an obviously-invalid domain ensures accidental dials fail loudly
/// (DNS NXDOMAIN) rather than silently hitting localhost.
const OFFLINE_PLACEHOLDER_BASE_URL: &str = "http://server-not-configured.invalid";

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

    // Resolve base_url: explicit flag/env → active context → legacy config.json → None.
    // Online commands require a configured server; offline/bootstrap commands
    // (context, auth, completion, version, init) run with a placeholder URL.
    let base_url = match resolve_startup_base_url(opts.base_url.clone()) {
        Some(url) => url,
        None if opts.command.allows_offline() => OFFLINE_PLACEHOLDER_BASE_URL.to_string(),
        None => {
            eprintln!("edgeplane: no EdgePlane server configured.");
            eprintln!("  Configure one with:  edgeplane context add <name> --url <url>");
            eprintln!("  then activate it:    edgeplane context use <name>");
            eprintln!("  or set EP_BASE_URL, or pass --base-url <url>.");
            std::process::exit(2);
        }
    };

    // Token comes exclusively from ~/.ep/session.json (written by `edgeplane auth login` OIDC flow).
    let token = edgeplane::config::load_session_token(&base_url);
    if token.is_some() {
        tracing::debug!("session token available from ~/.ep/session.json");
    }
    let token = if let Some(raw) = token {
        Some(
            secrets::resolve_maybe_secret_ref(&raw)
                .await
                .map_err(|e| anyhow::anyhow!("failed to resolve EdgePlane token secret ref: {e}"))?,
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

    edgeplane::commands::run(opts.command, client, booster, config, output_mode, opts.base_url).await
}
