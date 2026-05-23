//! `edgeplane agent supervise` — Phase 5 watchdog absorption CLI.
//!
//! Inspection + control for edgeplaned's systemd unit-liveness loop. All verbs
//! route through the mgmt-gateway over the Unix socket. Operator edits
//! to which agents get supervised happen in `fleet-profiles.toml`
//! (importer populates `systemd_service`); this CLI is for state +
//! pause/resume + manual restart.

use anyhow::{Context, Result, bail};
use clap::Args;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ─── CLI args ────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Agent id (e.g. "work", "operator", or the full agent_id).
    pub agent_id: String,
    /// How many recent restart events to show. Default 5.
    #[arg(long, default_value_t = 5)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RestartArgs {
    /// Agent id to restart. Logged as reason="manual".
    pub agent_id: String,
}

#[derive(Args, Debug)]
pub struct PauseResumeArgs {
    /// Agent id to pause/resume. Mutually exclusive with --all.
    pub agent_id: Option<String>,
    /// Apply to every supervised agent on this node.
    #[arg(long, conflicts_with = "agent_id")]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Filter to one agent's restart history. Default shows recent
    /// restarts across all supervised agents.
    #[arg(long)]
    pub agent_id: Option<String>,
    /// Maximum entries to show. Default 20.
    #[arg(short = 'n', long, default_value_t = 20)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct EventsArgs {
    /// Emit raw JSON event frames (one per line) instead of pretty-printed lines.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// Snapshot poll interval in seconds. Default 5.
    #[arg(long, default_value_t = 5)]
    pub poll_secs: u64,
    /// Maximum events to retain in the scrollback. Default 200.
    #[arg(long, default_value_t = 200)]
    pub tail_size: usize,
}

// ─── Runners ─────────────────────────────────────────────────────────────

pub async fn run_list(args: ListArgs) -> Result<()> {
    let resp = call_mgmt("agent.supervise.list", json!({})).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    let agents = resp.get("agents").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if agents.is_empty() {
        println!("(no supervised agents)");
        return Ok(());
    }
    println!(
        "{:<24} {:<22} {:<10} {:<8} {}",
        "AGENT", "SYSTEMD_SERVICE", "STATE", "PAUSED", "SOURCE"
    );
    for a in agents {
        let id = a.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?");
        let svc = a.get("systemd_service").and_then(|v| v.as_str()).unwrap_or("-");
        let state = a.get("unit_state").and_then(|v| v.as_str()).unwrap_or("?");
        let paused = a.get("supervise_paused").and_then(|v| v.as_bool()).unwrap_or(false);
        let paused_s = if paused { "YES" } else { "no" };
        let source = a.get("source").and_then(|v| v.as_str()).unwrap_or("-");
        println!("{id:<24} {svc:<22} {state:<10} {paused_s:<8} {source}");
    }
    Ok(())
}

pub async fn run_status(args: StatusArgs) -> Result<()> {
    let resp = call_mgmt(
        "agent.supervise.status",
        json!({ "agent_id": args.agent_id, "limit": args.limit }),
    )
    .await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    println!("agent:           {}", str_or(&resp, "agent_id", "?"));
    println!("source:          {}", str_or(&resp, "source", "?"));
    println!("systemd_service: {}", str_or(&resp, "systemd_service", "-"));
    println!("unit_state:      {}", str_or(&resp, "unit_state", "?"));
    println!(
        "supervise_paused:{} {}",
        if resp.get("supervise_paused").and_then(|v| v.as_bool()).unwrap_or(false) { "" } else { "  " },
        resp.get("supervise_paused").and_then(|v| v.as_bool()).map(|b| b.to_string()).unwrap_or_else(|| "?".into())
    );
    println!("\nrecent restarts (last {}):", args.limit);
    let history = resp.get("history").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if history.is_empty() {
        println!("  (none)");
    } else {
        for h in history {
            let when = h.get("triggered_at").and_then(|v| v.as_str()).unwrap_or("?");
            let reason = h.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
            let result = h.get("result").and_then(|v| v.as_str()).unwrap_or("?");
            let exit = h
                .get("systemctl_exit")
                .and_then(|v| v.as_i64())
                .map(|n| format!("exit={n}"))
                .unwrap_or_else(|| "-".into());
            println!("  {when}  reason={reason:<8} result={result:<10} {exit}");
            if let Some(notes) = h.get("notes").and_then(|v| v.as_str()) {
                if !notes.is_empty() {
                    println!("    {notes}");
                }
            }
        }
    }
    Ok(())
}

pub async fn run_restart(args: RestartArgs) -> Result<()> {
    let resp = call_mgmt(
        "agent.supervise.restart",
        json!({ "agent_id": args.agent_id }),
    )
    .await?;
    let result = resp.get("result").and_then(|v| v.as_str()).unwrap_or("?");
    let exit = resp.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if result == "started" {
        println!("Restarted {} (systemctl exit {})", args.agent_id, exit);
    } else {
        println!(
            "Restart of {} failed (systemctl exit {}). Check `edgeplane agent supervise status {}`.",
            args.agent_id, exit, args.agent_id
        );
    }
    Ok(())
}

pub async fn run_pause(args: PauseResumeArgs) -> Result<()> {
    pause_or_resume(args, true).await
}

pub async fn run_resume(args: PauseResumeArgs) -> Result<()> {
    pause_or_resume(args, false).await
}

async fn pause_or_resume(args: PauseResumeArgs, paused: bool) -> Result<()> {
    let mut params = serde_json::Map::new();
    if args.all {
        params.insert("all".into(), json!(true));
    } else if let Some(id) = args.agent_id {
        params.insert("agent_id".into(), json!(id));
    } else {
        bail!("specify an agent_id or --all");
    }
    let method = if paused {
        "agent.supervise.pause"
    } else {
        "agent.supervise.resume"
    };
    let resp = call_mgmt(method, Value::Object(params)).await?;
    let count = resp.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
    let verb = if paused { "paused" } else { "resumed" };
    println!("{verb} {count} agent(s)");
    Ok(())
}

pub async fn run_history(args: HistoryArgs) -> Result<()> {
    let mut params = serde_json::Map::new();
    params.insert("limit".into(), json!(args.limit));
    if let Some(id) = &args.agent_id {
        params.insert("agent_id".into(), json!(id));
    }
    let resp = call_mgmt("agent.supervise.history", Value::Object(params)).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    let rows = resp.get("restarts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if rows.is_empty() {
        println!("(no restart events recorded)");
        return Ok(());
    }
    println!(
        "{:<24} {:<14} {:<10} {:<10} {}",
        "TRIGGERED_AT", "AGENT", "REASON", "RESULT", "EXIT"
    );
    for r in rows {
        let when = r.get("triggered_at").and_then(|v| v.as_str()).unwrap_or("?");
        let id = r.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?");
        let reason = r.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
        let result = r.get("result").and_then(|v| v.as_str()).unwrap_or("?");
        let exit = r
            .get("systemctl_exit")
            .and_then(|v| v.as_i64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".into());
        println!("{when:<24} {id:<14} {reason:<10} {result:<10} {exit}");
    }
    Ok(())
}

// ─── Helpers (mirror agent_cron.rs's call_mgmt) ─────────────────────────

fn str_or<'a>(v: &'a Value, key: &str, fallback: &'a str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or(fallback)
}

/// Subscribe to edgeplaned's SupervisorEvent broadcast and print frames as they
/// arrive. Blocks until the user Ctrl-C's, edgeplaned shuts down, or the broadcast
/// channel issues a fatal lag signal.
pub async fn run_events(args: EventsArgs) -> Result<()> {
    let path = mgmt_socket_path();
    let stream = tokio::net::UnixStream::connect(&path).await.with_context(|| {
        format!(
            "connect to edgeplaned mgmt socket {} (is edgeplaned running?)",
            path.display()
        )
    })?;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    // Send events.subscribe.
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "events.subscribe",
        "params": {},
    });
    let mut bytes = serde_json::to_vec(&request).context("serialize subscribe")?;
    bytes.push(b'\n');
    write_half.write_all(&bytes).await.context("write subscribe")?;

    // Read ack — fails fast if the gateway returns an error (e.g. unit-health
    // loop not running).
    let mut ack_line = String::new();
    reader.read_line(&mut ack_line).await.context("read ack")?;
    let ack: Value = serde_json::from_str(ack_line.trim())
        .with_context(|| format!("parse ack: {}", ack_line.trim()))?;
    if let Some(err) = ack.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown");
        bail!("mgmt-gateway error {code}: {msg}");
    }
    if !args.json {
        eprintln!("subscribed — streaming SupervisorEvents (Ctrl-C to exit)");
    }

    // Read event frames until EOF (edgeplaned shutdown or broadcast lag).
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await.context("read event")?;
        if n == 0 {
            // edgeplaned closed the connection.
            if !args.json {
                eprintln!("stream closed by edgeplaned");
            }
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let frame: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("warn: bad event frame: {e}");
                continue;
            }
        };
        // Lag terminator frame from the gateway — print + exit non-zero so
        // operators don't miss the gap.
        if frame.get("error").and_then(|v| v.as_str()) == Some("lag") {
            let skipped = frame.get("skipped").and_then(|v| v.as_u64()).unwrap_or(0);
            bail!("broadcast lag — skipped {skipped} events. Subscriber too slow.");
        }
        if args.json {
            println!("{trimmed}");
        } else {
            println!("{}", pretty_event(&frame));
        }
    }
}

/// Render a SupervisorEvent JSON frame as a single human-readable line.
fn pretty_event(ev: &Value) -> String {
    let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
    let at = ev.get("at").and_then(|v| v.as_str()).unwrap_or("?");
    let agent = ev.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?");
    match kind {
        "unit_dead_detected" => format!("{at}  DEAD       {agent}"),
        "unit_restarted" => {
            let result = ev.get("result").and_then(|v| v.as_str()).unwrap_or("?");
            let reason = ev.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{at}  RESTART    {agent} reason={reason} result={result}")
        }
        "supervise_paused" => format!("{at}  PAUSED     {agent}"),
        "supervise_resumed" => format!("{at}  RESUMED    {agent}"),
        "nightly_restart_fired" => format!("{at}  NIGHTLY    {agent}"),
        other => format!("{at}  {other:10} {agent}"),
    }
}

fn mgmt_socket_path() -> std::path::PathBuf {
    crate::config::ep_home_dir().join("edgeplaned").join("mgmt.sock")
}

async fn call_mgmt(method: &str, params: Value) -> Result<Value> {
    let path = mgmt_socket_path();
    let stream = tokio::net::UnixStream::connect(&path).await.with_context(|| {
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
    write_half.write_all(&bytes).await.context("write request")?;

    let mut line = String::new();
    reader.read_line(&mut line).await.context("read response")?;

    let parsed: Value =
        serde_json::from_str(line.trim()).with_context(|| format!("parse mgmt response: {}", line.trim()))?;

    if let Some(err) = parsed.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        bail!("mgmt-gateway error {code}: {msg}");
    }

    Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
}
