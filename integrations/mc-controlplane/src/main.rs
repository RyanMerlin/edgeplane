use mc_controlplane::{build_app, AppConfig};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "mc-controlplane", about = "MissionControl API server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

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

#[derive(Subcommand, Debug)]
enum Command {
    /// Initialize S3-compatible object storage bucket (reads MC_OBJECT_STORAGE_* env vars)
    BucketInit,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "mc_controlplane=info".into()),
        )
        .init();

    let cli = Cli::parse();

    if matches!(cli.command, Some(Command::BucketInit)) {
        return bucket_init().await;
    }

    tracing::info!(bind = %cli.bind, "mc-controlplane starting");

    let db = mc_controlplane::db::connect().await?;

    if !cli.no_migrate {
        tracing::info!("running database migrations");
        sqlx::migrate!("./migrations").run(&db).await
            .map_err(|e| anyhow::anyhow!("migration failed: {e}"))?;
        tracing::info!("migrations complete");
    }

    // Backfill home missions for any pre-existing agents that lack one.
    // Safe to run every boot — idempotent, filters on home_mission_id IS NULL.
    if let Err(e) = backfill_home_missions(&db).await {
        tracing::warn!("home mission backfill failed (non-fatal): {e}");
    }

    let config = AppConfig {
        node_id: cli.node_id.unwrap_or(1),
        advertise_url: cli.advertise_url.clone(),
        api_proxy: cli.api_proxy.clone(),
    };

    let app = build_app(db, config);
    let listener = tokio::net::TcpListener::bind(&cli.bind).await?;
    tracing::info!(bind = %cli.bind, "listening");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn backfill_home_missions(db: &sqlx::PgPool) -> anyhow::Result<()> {
    let rows = sqlx::query("SELECT id, name FROM agent WHERE home_mission_id IS NULL AND archived_at IS NULL")
        .fetch_all(db).await?;
    if rows.is_empty() { return Ok(()); }
    tracing::info!("backfilling home missions for {} agent(s)", rows.len());
    for row in rows {
        let agent_id: i32 = sqlx::Row::get(&row, "id");
        let name: String = sqlx::Row::get(&row, "name");
        if let Err(e) = mc_controlplane::routes::agents::provision_home_mission(db, agent_id, &name).await {
            tracing::warn!("backfill home mission for agent {agent_id} ({name}): {e}");
        }
    }
    Ok(())
}

async fn bucket_init() -> anyhow::Result<()> {
    use s3::{Bucket, BucketConfiguration};
    use s3::creds::Credentials;
    use s3::region::Region;

    let endpoint = std::env::var("MC_OBJECT_STORAGE_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:9000".into());
    let region_name = std::env::var("MC_OBJECT_STORAGE_REGION")
        .unwrap_or_else(|_| "us-east-1".into());
    let bucket_name = std::env::var("MC_OBJECT_STORAGE_BUCKET")
        .unwrap_or_else(|_| "missioncontrol".into());
    let access_key = std::env::var("MC_OBJECT_STORAGE_ACCESS_KEY")
        .map_err(|_| anyhow::anyhow!("MC_OBJECT_STORAGE_ACCESS_KEY not set"))?;
    let secret_key = std::env::var("MC_OBJECT_STORAGE_ACCESS_SECRET")
        .map_err(|_| anyhow::anyhow!("MC_OBJECT_STORAGE_ACCESS_SECRET not set"))?;

    tracing::info!(endpoint = %endpoint, bucket = %bucket_name, "checking object storage bucket");

    let region = Region::Custom {
        region: region_name.clone(),
        endpoint: endpoint.clone(),
    };
    let credentials = Credentials::new(Some(&access_key), Some(&secret_key), None, None, None)
        .map_err(|e| anyhow::anyhow!("invalid credentials: {e}"))?;

    let bucket = Bucket::new(&bucket_name, region.clone(), credentials.clone())
        .map_err(|e| anyhow::anyhow!("bucket config error: {e}"))?
        .with_path_style();

    // Check if bucket already exists by listing (empty prefix, delimiter /)
    let exists = bucket.list("".to_string(), Some("/".to_string())).await.is_ok();

    if exists {
        tracing::info!(bucket = %bucket_name, "bucket already exists");
        return Ok(());
    }

    tracing::info!(bucket = %bucket_name, "bucket not found — creating");
    match Bucket::create_with_path_style(
        &bucket_name,
        region,
        credentials,
        BucketConfiguration::default(),
    )
    .await
    {
        Ok(_) => tracing::info!(bucket = %bucket_name, "bucket created"),
        Err(e) => {
            // Handle race condition: another process may have created it concurrently
            if bucket.list("".to_string(), Some("/".to_string())).await.is_ok() {
                tracing::info!(bucket = %bucket_name, "bucket created by concurrent process");
            } else {
                return Err(anyhow::anyhow!("failed to create bucket '{}': {}", bucket_name, e));
            }
        }
    }

    Ok(())
}
