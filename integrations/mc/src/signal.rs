//! `mc signal <to-agent-id> --content "<prompt>"` — single-shot prompt
//! injection into a persistent ACP agent.
//!
//! Thin wrapper over `POST /agents/{from}/message` that hides the
//! "sender" plumbing: the CLI looks up (or creates on first use) a
//! sender agent identity named `mc-signal-<hostname>` so the recipient
//! sees a stable, recognisable source. The message lands in the
//! recipient's inbox, the ACP supervisor picks it up via the existing
//! message relay, and `handle_signal` renders it as `session/prompt`
//! with the standard `[PEER MESSAGE from … on signal]` provenance
//! prefix.
//!
//! Designed for systemd timers: a unit line like
//!
//!   ExecStart=mc signal aria-operator-acp-test --content "Run /briefing"
//!
//! replaces `aria-trigger.sh` calls in Phase D of the tmux retirement
//! plan (docs/plans/2026-05-11-retire-tmux-via-acp-persistent-sessions.md).
//! The path is identical to what `mc agent remote message` would do
//! by hand — this wrapper just saves operators from having to remember
//! the sender id.

use crate::client::MissionControlClient;
use anyhow::{Context, Result};
use clap::Args;
use serde_json::{Value, json};

#[derive(Args, Debug)]
pub struct SignalArgs {
    /// Recipient agent — public_id (e.g. `aria-operator-e8820c0d`) or
    /// numeric id. The recipient must be a persistent ACP agent for the
    /// `session/prompt` translation to happen.
    pub agent_id: String,
    /// Prompt text to inject. Multi-line content is fine; quote it.
    #[arg(long)]
    pub content: String,
    /// Override the sender identity used when posting the message.
    /// Defaults to `mc-signal-<hostname>`, which the CLI auto-creates on
    /// first use.
    #[arg(long)]
    pub from: Option<String>,
    /// Message type stored on the row. Defaults to `signal` so operators
    /// can filter timer-driven prompts from chat / inter-agent traffic.
    #[arg(long, default_value = "signal")]
    pub message_type: String,
    /// Don't post — just print the request body that would be sent. Used
    /// to validate systemd unit lines before flipping them live.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(args: SignalArgs, client: &MissionControlClient) -> Result<()> {
    let sender = match args.from.clone() {
        Some(s) => s,
        None => default_sender_name(),
    };

    let body = json!({
        "to_agent_id": args.agent_id,
        "content": args.content,
        "message_type": args.message_type,
    });

    // Dry-run skips every network call — operators can validate a
    // systemd unit line without touching the controlplane.
    if args.dry_run {
        println!(
            "{}",
            json!({
                "post": format!("/agents/<sender_public_id>/message"),
                "from": sender,
                "body": body,
            })
        );
        return Ok(());
    }

    // Ensure the sender agent identity exists. Idempotent: the MCP
    // register_agent tool upserts by name and returns the existing
    // public_id when the row is already there. Capabilities are set to
    // `signal` so it's obvious in the agent list what this row is for.
    let pid = ensure_sender_agent(client, &sender)
        .await
        .with_context(|| format!("ensure sender agent `{sender}`"))?;

    let path = format!("/agents/{pid}/message");
    let resp: Value = client
        .post_json(&path, &body)
        .await
        .with_context(|| format!("POST {path}"))?;
    println!("{resp}");
    Ok(())
}

/// Default sender identity — `<hostname>-mc-signal`. Canonical format
/// matches the fleet naming convention: `<node>-<role>[-<hex>]`.
fn default_sender_name() -> String {
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    // Lowercase + dash-safe so the name passes the agent reserved-name
    // policy and the public_id format reader.
    let cleaned: String = host
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    format!("{cleaned}-mc-signal")
}

/// Upsert the sender agent via the MCP `register_agent` tool. Returns
/// the resolved `public_id`. The agent table's ON CONFLICT (name) DO
/// UPDATE makes this idempotent — re-runs leave the row's public_id
/// unchanged. register_agent returns the public_id directly so no
/// follow-up GET is needed.
async fn ensure_sender_agent(client: &MissionControlClient, name: &str) -> Result<String> {
    let body = json!({
        "tool": "register_agent",
        "args": { "name": name, "capabilities": "signal" }
    });
    let resp: Value = client
        .post_json("/mcp/call", &body)
        .await
        .context("mcp register_agent")?;
    if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        anyhow::bail!(
            "register_agent failed: {}",
            resp.get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
        );
    }
    let pid = resp
        .pointer("/result/public_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("register_agent response missing public_id"))?;
    Ok(pid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sender_name_lowercased_dash_safe() {
        // Inject HOSTNAME for determinism.
        // SAFETY: tests run sequentially within a binary by default; this
        // is the only test in this module that touches the env.
        unsafe {
            std::env::set_var("HOSTNAME", "Excalibur.NET");
        }
        let n = default_sender_name();
        unsafe {
            std::env::remove_var("HOSTNAME");
        }
        assert_eq!(n, "excalibur-net-mc-signal");
    }
}
