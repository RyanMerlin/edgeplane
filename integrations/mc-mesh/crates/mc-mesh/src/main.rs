/// mc-mesh daemon binary.
///
/// Headless — users interact via `mc mesh …` in the mc CLI.
mod attach_gateway;
mod attach_registry;
mod attach_ws;
mod config;
mod daemon;
mod mgmt_gateway;
mod secrets_gateway;
mod session_supervisor;
mod supervisor;
mod task_loop;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mc-mesh",
    version,
    about = "mc-mesh daemon — agent coordination for MissionControl"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the daemon (supervisor + task loops for all configured agents).
    Run {
        #[arg(long, env = "MC_BACKEND_URL", default_value = "")]
        backend_url: String,
        #[arg(long, env = "MC_TOKEN", default_value = "")]
        token: String,
        #[arg(long, env = "MC_MESH_WORK_DIR", default_value = "")]
        work_dir: String,
        #[arg(long, default_value = "30")]
        offline_grace_secs: u64,
    },
    /// Fetch a secret from the running mc-mesh secrets broker.
    ///
    /// Reads MC_SECRETS_SOCKET and MC_SECRETS_SESSION from the environment
    /// (injected by mc-mesh when spawning agent subprocesses). Prints the
    /// resolved value to stdout. Exits non-zero on any error.
    ///
    /// Example (inside an agent subprocess):
    ///   VALUE=$(mc-mesh get-secret MY_API_KEY)
    GetSecret {
        /// Name of the credential to fetch (the inject_as key).
        name: String,
    },
    /// Print version.
    Version,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "mc_mesh=info,mc_mesh_core=info,mc_mesh_work=info,mc_mesh_runtimes=info".into()
            }),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            backend_url,
            token,
            work_dir,
            offline_grace_secs,
        } => {
            let work_dir = if work_dir.is_empty() {
                config::DaemonConfig::load_or_default().work_dir
            } else {
                std::path::PathBuf::from(work_dir)
            };
            daemon::run(daemon::CliOverrides {
                backend_url,
                token,
                work_dir,
                offline_grace_secs,
            })
            .await
        }
        Commands::GetSecret { name } => get_secret(&name),
        Commands::Version => {
            println!("mc-mesh {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

/// Connect to the secrets gateway socket and fetch a single credential value.
/// Synchronous — no tokio runtime needed for this one-shot operation.
fn get_secret(name: &str) -> anyhow::Result<()> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let socket = std::env::var("MC_SECRETS_SOCKET").map_err(|_| {
        anyhow::anyhow!("MC_SECRETS_SOCKET not set — are you running inside an mc-mesh agent subprocess?")
    })?;
    let session = std::env::var("MC_SECRETS_SESSION").map_err(|_| {
        anyhow::anyhow!("MC_SECRETS_SESSION not set — are you running inside an mc-mesh agent subprocess?")
    })?;

    let mut stream = UnixStream::connect(&socket)
        .map_err(|e| anyhow::anyhow!("failed to connect to secrets socket at {socket}: {e}"))?;

    let req = serde_json::json!({"op": "get", "session": session, "name": name});
    stream.write_all(format!("{req}\n").as_bytes())?;
    stream.flush()?;

    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;

    let resp: serde_json::Value = serde_json::from_str(line.trim())
        .map_err(|e| anyhow::anyhow!("invalid response from secrets gateway: {e}"))?;

    if resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let value = resp["value"].as_str().unwrap_or("");
        println!("{value}");
        Ok(())
    } else {
        let err = resp["error"].as_str().unwrap_or("unknown error");
        anyhow::bail!("{err}")
    }
}
