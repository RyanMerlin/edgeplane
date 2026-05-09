use mc_controlplane::{build_app, AppConfig};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "mc-controlplane", about = "MissionControl API server")]
struct Args {
    /// Bind address
    #[arg(long, default_value = "0.0.0.0:8008", env = "MC_BIND")]
    bind: String,

    /// Node ID (informational only — used in /raft/status response)
    #[arg(long, env = "MC_NODE_ID")]
    node_id: Option<u64>,

    /// Advertised URL for this node (returned in /raft/status)
    #[arg(long, env = "MC_ADVERTISE_URL")]
    advertise_url: Option<String>,

    /// Proxy unknown routes to this upstream base URL (e.g. http://legacy-api:3000)
    #[arg(long, env = "MC_API_PROXY")]
    api_proxy: Option<String>,

    /// Skip automatic database migration on startup
    #[arg(long)]
    no_migrate: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "mc_controlplane=info".into()),
        )
        .init();

    let args = Args::parse();

    tracing::info!(bind = %args.bind, "mc-controlplane starting");

    let db = mc_controlplane::db::connect().await?;

    if !args.no_migrate {
        tracing::info!("running database migrations");
        sqlx::migrate!("./migrations").run(&db).await
            .map_err(|e| anyhow::anyhow!("migration failed: {e}"))?;
        tracing::info!("migrations complete");
    }

    let config = AppConfig {
        node_id: args.node_id.unwrap_or(1),
        advertise_url: args.advertise_url.clone(),
        api_proxy: args.api_proxy.clone(),
    };

    let app = build_app(db, config);
    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    tracing::info!(bind = %args.bind, "listening");
    axum::serve(listener, app).await?;

    Ok(())
}
