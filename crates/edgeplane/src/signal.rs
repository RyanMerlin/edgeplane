//! `edgeplane signal <to-agent-id> --content "<prompt>"` — single-shot prompt
//! injection into a persistent ACP agent.
//!
//! Thin wrapper over `POST /agents/{from}/message` that hides the
//! "sender" plumbing: the CLI looks up (or creates on first use) a
//! sender agent identity named `edgeplane-signal-<hostname>` so the recipient
//! sees a stable, recognisable source. The message lands in the
//! recipient's inbox, the ACP supervisor picks it up via the existing
//! message relay, and `handle_signal` renders it as `session/prompt`
//! with the standard `[PEER MESSAGE from … on signal]` provenance
//! prefix.
//!
//! Designed for systemd timers: a unit line like
//!
//!   ExecStart=edgeplane signal my-agent-operator --content "Run /briefing"
//!
//! The path is identical to what `edgeplane agent remote message` would do
//! by hand — this wrapper just saves operators from having to remember
//! the sender id.

use crate::client::EdgeplaneClient;
use anyhow::{Context, Result};
use clap::Args;
use serde_json::{Value, json};

#[derive(Args, Debug)]
pub struct SignalArgs {
    /// Recipient agent — public_id (e.g. `my-agent-operator-e8820c0d`) or
    /// numeric id. The recipient must be a persistent ACP agent for the
    /// `session/prompt` translation to happen.
    pub agent_id: String,
    /// Prompt text to inject. Multi-line content is fine; quote it.
    #[arg(long)]
    pub content: String,
    /// Override the sender identity used when posting the message.
    /// Defaults to `edgeplane-signal-<hostname>`, which the CLI auto-creates on
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

pub async fn run(args: SignalArgs, client: &EdgeplaneClient) -> Result<()> {
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

    // Ensure the sender agent identity exists. Idempotent: POST /agents
    // upserts by name and returns the existing public_id when the row is
    // already there. Capabilities are set to `signal` so it's obvious in
    // the agent list what this row is for.
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

/// Default sender identity — `<hostname>-edgeplane-signal`. Canonical format
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
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("{cleaned}-edgeplane-signal")
}

/// Upsert the sender agent via `POST /agents` and return its `public_id`.
///
/// `POST /agents` is idempotent: the tower's `create_agent` handler uses
/// `ON CONFLICT (name) DO UPDATE SET …` so a duplicate name refreshes
/// `capabilities` and `updated_at` rather than erroring. Both the insert
/// and the upsert-update paths return `200 OK` with the full agent JSON,
/// including `public_id` at the top level. No follow-up GET is needed.
///
/// The previous implementation called the MCP `register_agent` tool, which
/// was removed in ADR 0006 (tower MCP catalogue reduction). Callers of
/// `ensure_sender_agent` are unchanged.
async fn ensure_sender_agent(client: &EdgeplaneClient, name: &str) -> Result<String> {
    let body = json!({ "name": name, "capabilities": "signal" });
    let resp: Value = client
        .post_json("/agents", &body)
        .await
        .context("POST /agents")?;
    let pid = resp
        .get("public_id")
        .and_then(|v| v.as_str())
        .or_else(|| resp.get("id").and_then(|v| v.as_str()))
        .ok_or_else(|| anyhow::anyhow!("POST /agents response missing public_id: {resp}"))?;
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
            std::env::set_var("HOSTNAME", "MyNode.local");
        }
        let n = default_sender_name();
        unsafe {
            std::env::remove_var("HOSTNAME");
        }
        assert_eq!(n, "mynode-local-edgeplane-signal");
    }
}
