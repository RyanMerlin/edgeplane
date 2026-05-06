use mc_server::{build_app, AppConfig};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "mc-server", about = "MissionControl API server")]
struct Args {
    /// Serve the mc-ui web frontend
    #[arg(long)]
    ui: bool,

    /// Bind address
    #[arg(long, default_value = "0.0.0.0:8008", env = "MC_BIND")]
    bind: String,

    /// Node ID (informational only — used in /raft/status response)
    #[arg(long, env = "MC_NODE_ID")]
    node_id: Option<u64>,

    /// Advertised URL for this node (returned in /raft/status)
    #[arg(long, env = "MC_ADVERTISE_URL")]
    advertise_url: Option<String>,

    /// Skip automatic database migration on startup
    #[arg(long)]
    no_migrate: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "mc_server=info".into()),
        )
        .init();

    let args = Args::parse();

    tracing::info!(bind = %args.bind, "mc-server starting");

    let db = mc_server::db::connect().await?;

    if !args.no_migrate {
        tracing::info!("running database migrations");
        sqlx::migrate!("./migrations").run(&db).await
            .map_err(|e| anyhow::anyhow!("migration failed: {e}"))?;
        tracing::info!("migrations complete");
    }

    let config = AppConfig {
        node_id: args.node_id.unwrap_or(1),
        advertise_url: args.advertise_url.clone(),
    };

    let app = build_app(db, config);
    let listener = tokio::net::TcpListener::bind(&args.bind).await?;
    tracing::info!(bind = %args.bind, "listening");
    axum::serve(listener, app).await?;

    Ok(())
}
