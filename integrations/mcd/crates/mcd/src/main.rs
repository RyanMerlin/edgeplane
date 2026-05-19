/// mcd — MissionControl daemon binary.
///
/// Headless — users interact via `mc daemon …` in the mc CLI.
mod acp_session_supervisor;
mod attach_gateway;
mod attach_registry;
mod attach_ws;
mod config;
mod daemon;
mod doctor;
mod fleet_import;
mod local_registry;
mod mgmt_gateway;
mod reconcile;
mod replay_broadcast;
mod secrets_gateway;
mod session_supervisor;
mod singleton;
mod state;
mod supervisor;
mod task_loop;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "mcd",
    version,
    about = "mcd — MissionControl daemon, agent coordination"
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
        #[arg(long, env = "MCD_WORK_DIR", default_value = "")]
        work_dir: String,
        #[arg(long, default_value = "30")]
        offline_grace_secs: u64,
        /// Forcefully replace a running mcd. Sends SIGTERM to the holder,
        /// waits 5s, sends SIGKILL if still alive, then takes the lock.
        /// Use only when the existing daemon is hung — prefer
        /// `systemctl --user restart mcd.service` otherwise.
        #[arg(long)]
        kill_existing: bool,
        /// Allow startup to continue when a required TCP port is already
        /// bound (attach_ws 8009, mgmt 7731). Default is fatal. Use only
        /// when you knowingly want partial functionality.
        #[arg(long)]
        allow_degraded: bool,
    },
    /// Health check: lock state, port reachability, registry, runtimes.
    /// Read-only — does not connect to a running daemon, safe to run anytime.
    Doctor,
    /// Fetch a secret from the running mcd secrets broker.
    ///
    /// Reads MC_SECRETS_SOCKET and MC_SECRETS_SESSION from the environment
    /// (injected by mcd when spawning agent subprocesses). Prints the
    /// resolved value to stdout. Exits non-zero on any error.
    ///
    /// Example (inside an agent subprocess):
    ///   VALUE=$(mcd get-secret MY_API_KEY)
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
                "mcd=info,mcd_core=info,mcd_work=info,mcd_runtimes=info".into()
            }),
        )
        .init();

    mcd_core::migrate::migrate_once();

    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            backend_url,
            token,
            work_dir,
            offline_grace_secs,
            kill_existing,
            allow_degraded,
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
                kill_existing,
                allow_degraded,
            })
            .await
        }
        Commands::Doctor => doctor::run().await,
        Commands::GetSecret { name } => get_secret(&name),
        Commands::Version => {
            println!("mcd {}", env!("CARGO_PKG_VERSION"));
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
        anyhow::anyhow!("MC_SECRETS_SOCKET not set — are you running inside an mcd agent subprocess?")
    })?;
    let session = std::env::var("MC_SECRETS_SESSION").map_err(|_| {
        anyhow::anyhow!("MC_SECRETS_SESSION not set — are you running inside an mcd agent subprocess?")
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
