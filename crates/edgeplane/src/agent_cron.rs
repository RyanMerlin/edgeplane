//! `edgeplane agent cron` — inspection + reload for edgeplaned's cron scheduler.
//!
//! All verbs route through the mgmt_gateway over the Unix socket
//! (`~/.edgeplane/run/mgmt.sock`). Phase 4 of the daemon-absorption
//! plan. Job definitions live in `~/.edgeplane/config/cron.toml` — operators edit
//! that file directly; this CLI only inspects and pokes reload.

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
pub struct DescribeArgs {
    /// Job name as it appears in `cron.toml`.
    pub name: String,
    /// How many recent fires to include in the output.
    #[arg(long, default_value_t = 5)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ReloadArgs {}

#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Filter to one job; default shows fires across all jobs.
    #[arg(long)]
    pub name: Option<String>,
    /// Maximum number of fires to show. Default 20.
    #[arg(short = 'n', long, default_value_t = 20)]
    pub limit: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct GcNowArgs {
    /// Override `cron.toml`'s `[retention] history_days` for this sweep only.
    #[arg(long)]
    pub history_days: Option<u32>,
    /// Override `cron.toml`'s `[retention] max_rows_per_job` for this sweep only.
    #[arg(long)]
    pub max_rows_per_job: Option<u32>,
}

// ─── Runners ─────────────────────────────────────────────────────────────

pub async fn run_list(args: ListArgs) -> Result<()> {
    let resp = call_mgmt("agent.cron.list", json!({})).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    let jobs = resp
        .get("jobs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if jobs.is_empty() {
        println!("(no cron jobs)");
        return Ok(());
    }
    println!(
        "{:<24} {:<14} {:<18} {:<22} {:<10} ENABLED",
        "NAME", "AGENT", "SCHEDULE", "LAST_FIRED_AT", "STATUS"
    );
    for j in jobs {
        let name = j.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let agent = j.get("session").and_then(|v| v.as_str()).unwrap_or("?");
        let kind = j.get("kind").and_then(|v| v.as_str()).unwrap_or("cron");
        let sched_owned = match kind {
            "heartbeat" => {
                let iv = j.get("interval").and_then(|v| v.as_str()).unwrap_or("?");
                format!("heartbeat: {iv}")
            }
            _ => j
                .get("schedule")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string(),
        };
        let last = j
            .get("last_fired_at")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let status = j.get("last_status").and_then(|v| v.as_str()).unwrap_or("-");
        let enabled = j.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
        let enabled_s = if enabled { "yes" } else { "NO" };
        println!("{name:<24} {agent:<14} {sched_owned:<18} {last:<22} {status:<10} {enabled_s}");
    }
    Ok(())
}

pub async fn run_describe(args: DescribeArgs) -> Result<()> {
    let resp = call_mgmt(
        "agent.cron.describe",
        json!({ "name": args.name, "limit": args.limit }),
    )
    .await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    println!("name:          {}", str_or(&resp, "name", "?"));
    println!("agent:         {}", str_or(&resp, "session", "?"));
    let kind = resp.get("kind").and_then(|v| v.as_str()).unwrap_or("cron");
    println!("kind:          {kind}");
    match kind {
        "heartbeat" => println!("interval:      {}", str_or(&resp, "interval", "?")),
        _ => println!("schedule:      {}", str_or(&resp, "schedule", "?")),
    }
    println!("dispatch:      {}", str_or(&resp, "dispatch", "?"));
    println!(
        "enabled:       {}",
        resp.get("enabled")
            .and_then(|v| v.as_bool())
            .map(|b| b.to_string())
            .unwrap_or_else(|| "?".into())
    );
    println!(
        "last_fired_at: {}",
        str_or(&resp, "last_fired_at", "(never)")
    );
    println!("last_status:   {}", str_or(&resp, "last_status", "(never)"));
    if let Some(err) = resp.get("last_error").and_then(|v| v.as_str()) {
        println!("last_error:    {err}");
    }
    println!("\nprompt:");
    if let Some(p) = resp.get("prompt").and_then(|v| v.as_str()) {
        for line in p.lines() {
            println!("  {line}");
        }
    }
    println!("\nrecent fires (last {}):", args.limit);
    let history = resp
        .get("history")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if history.is_empty() {
        println!("  (none)");
    } else {
        for h in history {
            let when = h.get("fired_at").and_then(|v| v.as_str()).unwrap_or("?");
            let status = h.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            let dur = h
                .get("duration_ms")
                .and_then(|v| v.as_i64())
                .map(|n| format!("{n}ms"))
                .unwrap_or_else(|| "-".into());
            println!("  {when}  {status:<8}  {dur}");
            if let Some(e) = h.get("error_message").and_then(|v| v.as_str())
                && !e.is_empty()
            {
                println!("    err: {e}");
            }
        }
    }
    Ok(())
}

pub async fn run_reload(_args: ReloadArgs) -> Result<()> {
    let resp = call_mgmt("agent.cron.reload", json!({})).await?;
    println!(
        "reload queued (next tick will re-parse): {}",
        serde_json::to_string(&resp)?
    );
    Ok(())
}

pub async fn run_history(args: HistoryArgs) -> Result<()> {
    let mut params = json!({ "limit": args.limit });
    if let Some(n) = &args.name {
        params["name"] = json!(n);
    }
    let resp = call_mgmt("agent.cron.history", params).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    let fires = resp
        .get("fires")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if fires.is_empty() {
        println!("(no fires recorded)");
        return Ok(());
    }
    println!("{:<24} {:<24} {:<8} DURATION", "FIRED_AT", "JOB", "STATUS");
    for f in fires {
        let when = f.get("fired_at").and_then(|v| v.as_str()).unwrap_or("?");
        let job = f.get("job_name").and_then(|v| v.as_str()).unwrap_or("?");
        let status = f.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let dur = f
            .get("duration_ms")
            .and_then(|v| v.as_i64())
            .map(|n| format!("{n}ms"))
            .unwrap_or_else(|| "-".into());
        println!("{when:<24} {job:<24} {status:<8} {dur}");
        if let Some(e) = f.get("error_message").and_then(|v| v.as_str())
            && !e.is_empty()
        {
            println!("  err: {e}");
        }
    }
    Ok(())
}

pub async fn run_gc_now(args: GcNowArgs) -> Result<()> {
    let mut params = serde_json::Map::new();
    if let Some(d) = args.history_days {
        params.insert("history_days".into(), json!(d));
    }
    if let Some(n) = args.max_rows_per_job {
        params.insert("max_rows_per_job".into(), json!(n));
    }
    let resp = call_mgmt("agent.cron.gc_now", Value::Object(params)).await?;
    let deleted = resp.get("deleted").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("cron gc swept {deleted} fire_log row(s)");
    if let Some(d) = resp.get("history_days").and_then(|v| v.as_u64()) {
        println!(
            "  history_days={d}  max_rows_per_job={}",
            resp.get("max_rows_per_job")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
        );
    }
    Ok(())
}

// ─── Helpers ─────────────────────────────────────────────────────────────

fn str_or<'a>(v: &'a Value, key: &str, fallback: &'a str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or(fallback)
}

fn mgmt_socket_path() -> std::path::PathBuf {
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
        .with_context(|| format!("parse mgmt response: {}", line.trim()))?;

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
