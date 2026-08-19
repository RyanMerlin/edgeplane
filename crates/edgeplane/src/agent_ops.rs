//! `edgeplane agent signal|cancel|list|describe` — local-first agent ops.
//!
//! Phase 3 of the daemon-absorption plan. Each verb auto-resolves:
//!   1. Ask edgeplaned's mgmt_gateway via `agent.describe_local <id>`.
//!   2. If found locally → dispatch via mgmt_gateway (`agent.local.*`).
//!   3. If not found locally → fall through to controlplane (existing
//!      `edgeplane signal` / agent endpoints).
//!   4. If neither has it → bail with a structured error showing what was
//!      checked.
//!
//! `--local` / `--remote` flags force a single path when the operator
//! wants to skip auto-resolve (e.g. when an ID collides).
//!
//! ## Transport
//!
//! Local: Unix socket `~/.edgeplane/run/mgmt.sock`, newline-delimited
//! JSON-RPC 2.0, no auth (filesystem perms gate access).
//! Remote: existing controlplane HTTP via [`EdgeplaneClient`].

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::client::EdgeplaneClient;

// ─── CLI args ────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct SignalArgs {
    /// Agent id. For local agents this is the profile name (e.g. `work`);
    /// for controlplane agents it's the `public_id`.
    pub agent_id: String,
    /// Prompt text to inject. Multi-line is fine; quote it.
    #[arg(long)]
    pub content: String,
    /// Force the local mgmt-gateway path; skip the controlplane fallback.
    #[arg(long, conflicts_with = "remote")]
    pub local: bool,
    /// Force the controlplane path; skip the local lookup.
    #[arg(long, conflicts_with = "local")]
    pub remote: bool,
}

#[derive(Args, Debug)]
pub struct CancelArgs {
    pub agent_id: String,
    #[arg(long, conflicts_with = "remote")]
    pub local: bool,
    #[arg(long, conflicts_with = "local")]
    pub remote: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Which source to list. Default: `all` (both local + controlplane).
    #[arg(long, value_enum, default_value_t = ListSource::All)]
    pub source: ListSource,
    /// Emit raw JSON instead of the table view.
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum ListSource {
    Local,
    Remote,
    All,
}

#[derive(Args, Debug)]
pub struct DescribeArgs {
    pub agent_id: String,
    #[arg(long, conflicts_with = "remote")]
    pub local: bool,
    #[arg(long, conflicts_with = "local")]
    pub remote: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RegisterArgs {
    /// Agent name (must be unique on the controlplane).
    #[arg(long)]
    pub name: String,
    /// Comma-separated capability tags (e.g. `fleet-management,code-editing`).
    #[arg(long, default_value = "")]
    pub capabilities: String,
    /// Optional JSON metadata string (e.g. `{"runtime":"claude-code","node_id":"node-0"}`).
    #[arg(long)]
    pub metadata: Option<String>,
    /// Emit raw JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SetStatusArgs {
    /// Agent id or public_id on the controlplane.
    #[arg(long)]
    pub id: String,
    /// New status value (e.g. `online`, `offline`, `busy`).
    #[arg(long)]
    pub status: String,
    /// Emit raw JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// Agent public_id or numeric id to delete.
    pub agent_id: String,
    /// Skip the confirmation prompt.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Emit raw JSON instead of a human-readable summary.
    #[arg(long)]
    pub json: bool,
}

// ─── Command runners ─────────────────────────────────────────────────────

pub async fn run_signal(args: SignalArgs, client: &EdgeplaneClient) -> Result<()> {
    if args.remote {
        return signal_remote(&args.agent_id, &args.content, client).await;
    }
    if args.local {
        return signal_local(&args.agent_id, &args.content).await;
    }
    // Auto-resolve: ask edgeplaned if it knows the agent locally.
    match describe_local(&args.agent_id).await? {
        Some(_) => signal_local(&args.agent_id, &args.content).await,
        None => signal_remote(&args.agent_id, &args.content, client).await,
    }
}

pub async fn run_cancel(args: CancelArgs, client: &EdgeplaneClient) -> Result<()> {
    if args.remote {
        return cancel_remote(&args.agent_id, client).await;
    }
    if args.local {
        return cancel_local(&args.agent_id).await;
    }
    match describe_local(&args.agent_id).await? {
        Some(_) => cancel_local(&args.agent_id).await,
        None => cancel_remote(&args.agent_id, client).await,
    }
}

pub async fn run_describe(args: DescribeArgs, client: &EdgeplaneClient) -> Result<()> {
    let (source, body) = if args.remote {
        ("remote", describe_remote(&args.agent_id, client).await?)
    } else if args.local {
        (
            "local",
            describe_local(&args.agent_id)
                .await?
                .ok_or_else(|| anyhow!("no local agent named '{}'", args.agent_id))?,
        )
    } else {
        match describe_local(&args.agent_id).await? {
            Some(v) => ("local", v),
            None => ("remote", describe_remote(&args.agent_id, client).await?),
        }
    };

    let envelope = json!({ "source": source, "agent": body });
    if args.json {
        println!("{}", serde_json::to_string_pretty(&envelope)?);
    } else {
        print_describe_human(&envelope);
    }
    Ok(())
}

pub async fn run_list(args: ListArgs, client: &EdgeplaneClient) -> Result<()> {
    let mut local_agents: Vec<Value> = vec![];
    let mut remote_agents: Vec<Value> = vec![];

    if matches!(args.source, ListSource::Local | ListSource::All) {
        local_agents = match list_local().await {
            Ok(v) => v,
            Err(e) => {
                // Local edgeplaned unavailable shouldn't kill `--source all`.
                if matches!(args.source, ListSource::Local) {
                    return Err(e);
                }
                eprintln!("warning: local list failed: {e:#}; continuing with remote only");
                vec![]
            }
        };
    }
    if matches!(args.source, ListSource::Remote | ListSource::All) {
        remote_agents = list_remote(client).await.unwrap_or_else(|e| {
            if matches!(args.source, ListSource::Remote) {
                eprintln!("error: remote list failed: {e:#}");
            }
            vec![]
        });
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "local": local_agents,
                "remote": remote_agents,
            }))?
        );
    } else {
        print_list_human(&local_agents, &remote_agents);
    }
    Ok(())
}

pub async fn run_register(args: RegisterArgs, client: &EdgeplaneClient) -> Result<()> {
    let mut body = json!({
        "name": args.name,
        "capabilities": args.capabilities,
    });
    if let Some(m) = args.metadata {
        body["metadata"] = json!(m);
    }
    let result: Value = client.post_json("/agents", &body).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        let id = result
            .get("id")
            .or_else(|| result.get("public_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let name = result
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&args.name);
        println!("registered agent '{name}' id={id}");
    }
    Ok(())
}

pub async fn run_set_status(args: SetStatusArgs, client: &EdgeplaneClient) -> Result<()> {
    let body = json!({ "status": args.status });
    let result: Value = client
        .patch_json(&format!("/agents/{}", args.id), &body)
        .await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        let status = result
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or(&args.status);
        println!("agent {} status -> {status}", args.id);
    }
    Ok(())
}

pub async fn run_delete(args: DeleteArgs, client: &EdgeplaneClient) -> Result<()> {
    // Confirm the agent exists on the controlplane and extract display fields.
    let info = describe_remote(&args.agent_id, client).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("HTTP 404") {
            anyhow!("agent '{}' not found on the controlplane", args.agent_id)
        } else if msg.contains("HTTP 403") {
            anyhow!(
                "forbidden: insufficient permissions to describe agent '{}'",
                args.agent_id
            )
        } else {
            e
        }
    })?;

    let name = info
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&args.agent_id);
    let display_id = info
        .get("public_id")
        .or_else(|| info.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or(&args.agent_id);
    let home_domain_id = info
        .get("home_domain_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    if !args.yes {
        eprint!(
            "Delete agent {} ({})? This cannot be undone. [y/N]: ",
            name, display_id
        );
        use std::io::{BufRead, Write};
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::BufReader::new(std::io::stdin()).read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y") {
            eprintln!("aborted");
            return Ok(());
        }
    }

    client
        .delete(&format!("/agents/{}", args.agent_id))
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("HTTP 404") {
                anyhow!("agent '{}' not found on the controlplane", args.agent_id)
            } else if msg.contains("HTTP 403") {
                anyhow!(
                    "forbidden: insufficient permissions to delete agent '{}'",
                    args.agent_id
                )
            } else {
                e
            }
        })?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "deleted": true,
                "agent_id": display_id,
            }))?
        );
    } else {
        println!("deleted agent {display_id}");
    }

    if let Some(domain_id) = home_domain_id {
        eprintln!(
            "note: home domain {domain_id} is not auto-removed \
             (domains are not currently deletable via the API)"
        );
    }

    Ok(())
}

// ─── Local dispatch (mgmt_gateway over Unix socket) ──────────────────────

async fn signal_local(agent_id: &str, text: &str) -> Result<()> {
    let resp = call_mgmt(
        "agent.local.signal",
        json!({ "agent_id": agent_id, "kind": "user_input", "text": text }),
    )
    .await?;
    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

async fn cancel_local(agent_id: &str) -> Result<()> {
    let resp = call_mgmt(
        "agent.local.signal",
        json!({ "agent_id": agent_id, "kind": "cancel" }),
    )
    .await?;
    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

async fn list_local() -> Result<Vec<Value>> {
    let resp = call_mgmt("agent.local.list", json!({})).await?;
    let agents = resp
        .get("agents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(agents)
}

/// Returns `Ok(Some(info))` when the agent is known locally, `Ok(None)`
/// otherwise. Errors only when the mgmt socket itself is unreachable.
pub async fn describe_local(agent_id: &str) -> Result<Option<Value>> {
    let resp = call_mgmt("agent.describe_local", json!({ "agent_id": agent_id })).await?;
    let found = resp.get("found").and_then(|v| v.as_bool()).unwrap_or(false);
    if found { Ok(Some(resp)) } else { Ok(None) }
}

// ─── Remote dispatch (controlplane HTTP) ─────────────────────────────────

async fn signal_remote(agent_id: &str, content: &str, client: &EdgeplaneClient) -> Result<()> {
    // Validate the recipient exists on the controlplane before spending a
    // round-trip on sender registration. A typo or stale ID produces a
    // cryptic 404 that names the *sender* public_id — not the bad recipient —
    // so we fail early with an actionable message instead.
    if let Err(e) = describe_remote(agent_id, client).await {
        // Pull the local profile list for the suggestion string. Failures
        // here are soft — we'd rather show an empty list than mask the
        // real error.
        let local_names: String = {
            let agents = list_local().await.unwrap_or_default();
            if agents.is_empty() {
                "none".to_string()
            } else {
                agents
                    .iter()
                    .map(|a| {
                        a.get("agent_id")
                            .or_else(|| a.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        };
        return Err(anyhow!(
            "agent '{}' not found on the controlplane and is not a local profile \
             (lookup error: {:#}). \
             Local profiles: {}. \
             To target a local agent, use its profile name (auto-resolves) or pass --local; \
             for a controlplane agent pass its public_id.",
            agent_id,
            e,
            local_names,
        ));
    }

    // Recipient verified — proceed with sender registration and POST.
    let args = crate::signal::SignalArgs {
        agent_id: agent_id.to_string(),
        content: content.to_string(),
        from: None,
        message_type: "signal".to_string(),
        dry_run: false,
    };
    crate::signal::run(args, client).await
}

async fn cancel_remote(_agent_id: &str, _client: &EdgeplaneClient) -> Result<()> {
    bail!(
        "remote cancel not yet implemented for controlplane agents. \
         Use `edgeplane agent signal <id> --content 'cancel'` to send a cancel prompt, \
         or implement Cancel routing through the controlplane in a follow-up PR."
    )
}

async fn describe_remote(agent_id: &str, client: &EdgeplaneClient) -> Result<Value> {
    let path = format!("/agents/{agent_id}");
    let v: Value = client
        .get_json(&path)
        .await
        .with_context(|| format!("GET {path}"))?;
    Ok(v)
}

async fn list_remote(client: &EdgeplaneClient) -> Result<Vec<Value>> {
    let v: Value = client.get_json("/agents").await.context("GET /agents")?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

// ─── Mgmt-socket JSON-RPC client ─────────────────────────────────────────

fn mgmt_socket_path() -> std::path::PathBuf {
    // Delegates to edgeplaned_paths::mgmt_socket_path() — `~/.edgeplane/run/mgmt.sock`.
    edgeplaned_paths::mgmt_socket_path()
}

async fn call_mgmt(method: &str, params: Value) -> Result<Value> {
    let path = mgmt_socket_path();
    let stream = tokio::net::UnixStream::connect(&path)
        .await
        .with_context(|| {
            format!(
                "connect to edgeplaned mgmt socket {} (is edgeplaned running?)",
                path.display()
            )
        })?;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let mut bytes = serde_json::to_vec(&request).context("serialize request")?;
    bytes.push(b'\n');
    write_half
        .write_all(&bytes)
        .await
        .context("write request")?;

    let mut line = String::new();
    reader.read_line(&mut line).await.context("read response")?;

    let parsed: Value = serde_json::from_str(line.trim())
        .with_context(|| format!("parse response from mgmt socket: {}", line.trim()))?;

    if let Some(err) = parsed.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        bail!("mgmt-gateway error {code}: {msg}");
    }

    Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
}

// ─── Pretty printers ─────────────────────────────────────────────────────

fn print_describe_human(envelope: &Value) {
    let source = envelope
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let agent = envelope.get("agent").cloned().unwrap_or(Value::Null);
    println!("source:        {source}");
    if let Some(id) = agent.get("agent_id").and_then(|v| v.as_str()) {
        println!("agent_id:      {id}");
    }
    if let Some(rk) = agent.get("runtime_kind").and_then(|v| v.as_str()) {
        println!("runtime_kind:  {rk}");
    }
    if let Some(s) = agent.get("source").and_then(|v| v.as_str()) {
        println!("source_tag:    {s}");
    }
    if let Some(m) = agent.get("domain_id").and_then(|v| v.as_str())
        && !m.is_empty()
    {
        println!("domain_id:    {m}");
    }
    if let Some(zs) = agent.get("zellij_session").and_then(|v| v.as_str()) {
        println!("zellij_session:{zs}");
    }
    if let Some(vf) = agent.get("vault_folder").and_then(|v| v.as_str()) {
        println!("vault_folder:  {vf}");
    }
    if let Some(sv) = agent.get("supervised").and_then(|v| v.as_bool()) {
        println!("supervised:    {sv}");
    }
}

fn print_list_human(local: &[Value], remote: &[Value]) {
    if local.is_empty() && remote.is_empty() {
        println!("(no agents)");
        return;
    }
    if !local.is_empty() {
        println!("LOCAL ({}):", local.len());
        for a in local {
            let id = a.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?");
            let rk = a
                .get("runtime_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let zs = a
                .get("zellij_session")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let suffix = if zs.is_empty() {
                String::new()
            } else {
                format!("  zellij:{zs}")
            };
            println!("  {id:<24} {rk:<18}{suffix}");
        }
    }
    if !remote.is_empty() {
        if !local.is_empty() {
            println!();
        }
        println!("REMOTE ({}):", remote.len());
        for a in remote {
            let id = a
                .get("public_id")
                .or_else(|| a.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let rk = a
                .get("runtime_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let status = a.get("status").and_then(|v| v.as_str()).unwrap_or("");
            println!("  {id:<32} {rk:<18}  {status}");
        }
    }
}
