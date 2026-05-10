/// mc-mesh daemon binary.
///
/// Headless — users interact via `mc mesh …` in the mc CLI.
mod acp_session_supervisor;
mod attach_gateway;
mod attach_registry;
mod attach_ws;
mod config;
mod daemon;
mod mgmt_gateway;
mod secrets_gateway;
mod session_supervisor;
mod state;
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
    /// Register this node with mc-controlplane and persist its identity.
    ///
    /// Posts to `POST /runtime/nodes/register` with the supplied bootstrap
    /// token, captures the returned `node_id` + `attach_secret`, and writes
    /// `~/.mc/mc-mesh.state.json` (mode 0600). Run this once per node before
    /// starting the daemon.
    NodeRegister {
        /// One-time bootstrap token from `POST /runtime/join-tokens` (also
        /// available via the `mc` CLI). Used here, then discarded by the
        /// controlplane.
        #[arg(long)]
        bootstrap_token: String,
        /// Display name for this node (must be unique across the org).
        /// Defaults to the system hostname.
        #[arg(long)]
        node_name: Option<String>,
        /// Trust tier label sent to the controlplane. Defaults to
        /// "untrusted"; bump per your security model.
        #[arg(long, default_value = "untrusted")]
        trust_tier: String,
        /// Tailscale IPv4 to register, if not auto-detectable.
        #[arg(long)]
        tailscale_ip: Option<String>,
        /// Tailscale FQDN to register (e.g. `epyc.tailnet.ts.net`).
        #[arg(long)]
        tailscale_fqdn: Option<String>,
        /// Backend URL override. Falls back to `MC_BACKEND_URL` env var, then
        /// `mc auth`'s session.json.
        #[arg(long, env = "MC_BACKEND_URL")]
        backend_url: Option<String>,
        /// Bearer token for the controlplane. Falls back to `MC_TOKEN` env
        /// var, then `mc auth`'s session.json.
        #[arg(long, env = "MC_TOKEN")]
        token: Option<String>,
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
        Commands::NodeRegister {
            bootstrap_token,
            node_name,
            trust_tier,
            tailscale_ip,
            tailscale_fqdn,
            backend_url,
            token,
        } => {
            register_node(
                bootstrap_token,
                node_name,
                trust_tier,
                tailscale_ip,
                tailscale_fqdn,
                backend_url,
                token,
            )
            .await
        }
        Commands::Version => {
            println!("mc-mesh {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

/// Resolve credentials, hit `POST /runtime/nodes/register`, write the state
/// file. Errors carry the controlplane's response body when registration is
/// rejected (e.g. invalid bootstrap token, name conflict).
async fn register_node(
    bootstrap_token: String,
    node_name: Option<String>,
    trust_tier: String,
    tailscale_ip: Option<String>,
    tailscale_fqdn: Option<String>,
    backend_url: Option<String>,
    token: Option<String>,
) -> anyhow::Result<()> {
    use anyhow::{Context, anyhow};

    // Inherit credentials from `mc auth` if not explicitly supplied.
    let (resolved_url, resolved_token) = resolve_credentials(backend_url, token)?;
    if resolved_url.is_empty() {
        anyhow::bail!(
            "no backend_url — pass --backend-url, set MC_BACKEND_URL, or run `mc auth login` first"
        );
    }
    if resolved_token.is_empty() {
        anyhow::bail!(
            "no token — pass --token, set MC_TOKEN, or run `mc auth login` first"
        );
    }

    let node_name = match node_name {
        Some(n) => n,
        None => hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .ok_or_else(|| anyhow!("could not determine hostname; pass --node-name explicitly"))?,
    };

    let body = serde_json::json!({
        "node_name": node_name,
        "hostname": hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_default(),
        "trust_tier": trust_tier,
        "labels": {},
        "capacity": {},
        "capabilities": [],
        "runtime_version": env!("CARGO_PKG_VERSION"),
        "bootstrap_token": bootstrap_token,
        "tailscale_ip": tailscale_ip,
        "tailscale_fqdn": tailscale_fqdn,
    });

    let client = mc_mesh_core::client::BackendClient::new(&resolved_url, &resolved_token);
    let resp = client
        .raw_post_no_throw("/runtime/nodes/register", &body)
        .await
        .context("calling /runtime/nodes/register")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("registration failed ({status}): {body}");
    }

    let json: serde_json::Value = resp.json().await.context("parsing register response")?;
    let node_id = json
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("register response missing `id`"))?
        .to_string();
    let attach_secret = json
        .get("attach_secret")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow!(
                "register response missing `attach_secret`; the controlplane is older than the per-node secret rollout"
            )
        })?
        .to_string();

    let registered_at = chrono::Utc::now().to_rfc3339();
    let state = state::NodeState {
        schema_version: state::STATE_SCHEMA_VERSION,
        node_id: node_id.clone(),
        attach_secret,
        registered_at,
        controlplane_url: resolved_url.clone(),
    };

    let path = state::NodeState::default_path()?;
    state.write_atomic(&path)?;

    println!("Registered node {node_id} with {resolved_url}");
    println!("Wrote state to {} (mode 0600)", path.display());
    println!();
    println!("Next: enroll agents to this node via the mc CLI, then start the daemon.");
    Ok(())
}

/// Read backend_url + token from CLI args first, then env, then `mc auth`'s
/// session.json. Mirrors the resolution order used in `DaemonConfig`.
fn resolve_credentials(
    cli_url: Option<String>,
    cli_token: Option<String>,
) -> anyhow::Result<(String, String)> {
    use anyhow::Context;

    let mut url = cli_url.unwrap_or_default();
    let mut token = cli_token.unwrap_or_default();

    if url.is_empty() || token.is_empty() {
        let session_path = mc_mesh_core::paths::session_file_path();
        if session_path.exists() {
            let content = std::fs::read_to_string(&session_path)
                .with_context(|| format!("reading {}", session_path.display()))?;
            let session: serde_json::Value = serde_json::from_str(&content)
                .with_context(|| format!("parsing {}", session_path.display()))?;
            if url.is_empty() {
                if let Some(s) = session.get("base_url").and_then(|v| v.as_str()) {
                    url = s.to_string();
                }
            }
            if token.is_empty() {
                if let Some(s) = session.get("token").and_then(|v| v.as_str()) {
                    token = s.to_string();
                }
            }
        }
    }

    Ok((url, token))
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
