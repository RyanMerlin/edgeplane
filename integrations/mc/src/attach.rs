//! `mc agent attach <agent-id>` — WebSocket client that opens a viewer
//! into a persistent ACP session on a mesh node.
//!
//! The end-to-end path closed by `pump_acp` in mc-mesh is:
//!
//!   mc agent attach
//!     ⇄ WS — controlplane `GET /runtime/nodes/{n}/agents/{a}/attach`
//!     ⇄ HMAC'd dial to mc-mesh `attach_ws`
//!     ⇄ `pump_acp` ⇆ supervisor ⇆ `AcpSession` ⇆ `claude-agent-acp`
//!
//! This module is the CLI consumer at the top of that stack. It exists to
//! validate Phase 2 of the persistent-session work end-to-end with a
//! scriptable surface, and to give operators a fallback when the web
//! conversation pane is unavailable. See
//! `docs/plans/2026-05-11-retire-tmux-via-acp-persistent-sessions.md`
//! (Phase B).
//!
//! Wire framing matches `attach_ws::pump_acp`:
//! - Outbound from the agent: text frames carrying `SessionNotification`
//!   JSON. Rendered human-readably by default; raw with `--json`.
//! - Inbound to the agent: text frames of the form
//!   `{"kind":"prompt","text":"…"}` or `{"kind":"cancel"}`.

use crate::client::MissionControlClient;
use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[derive(Args, Debug)]
pub struct AttachArgs {
    /// Agent public_id (e.g. `aria-operator-e8820c0d`) or numeric id.
    pub agent_id: String,
    /// Stream raw `SessionNotification` JSON, one frame per line. Default
    /// is a human-readable rendering of assistant turns, tool calls, etc.
    #[arg(long)]
    pub json: bool,
    /// Override node id when the agent registry doesn't know which node
    /// hosts this agent (rare; mostly useful during early bringup before
    /// linkage is fully populated).
    #[arg(long)]
    pub node_id: Option<String>,
}

pub async fn run(args: AttachArgs, client: &MissionControlClient) -> Result<()> {
    let node_id = match args.node_id.clone() {
        Some(n) => n,
        None => resolve_node_id(client, &args.agent_id)
            .await
            .with_context(|| {
                format!(
                    "could not resolve agent `{}` to a node — pass --node-id explicitly",
                    args.agent_id
                )
            })?,
    };
    let ws_url = build_ws_url(client.base_url().as_str(), &node_id, &args.agent_id)?;
    let token = client
        .token()
        .ok_or_else(|| anyhow!("no auth token configured — run `mc auth login` first"))?
        .to_string();

    eprintln!(
        "attaching to agent {} on node {} via {}…",
        args.agent_id, node_id, ws_url
    );

    let mut request = ws_url.into_client_request().context("build ws request")?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {token}").parse().context("token header")?,
    );

    let (mut ws, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .context("websocket connect")?;

    // Outbound stdin → WS: spawned so it doesn't block the inbound pump.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Message>(8);
    let stdin_tx = out_tx.clone();
    tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let stdin = tokio::io::stdin();
        let mut lines = BufReader::new(stdin).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let env = serde_json::json!({ "kind": "prompt", "text": trimmed });
            if stdin_tx
                .send(Message::Text(env.to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // Ctrl-C → cancel envelope + clean exit.
    let cancel_tx = out_tx.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let env = serde_json::json!({ "kind": "cancel" });
        let _ = cancel_tx
            .send(Message::Text(env.to_string().into()))
            .await;
        // Give the cancel frame a moment to flush, then exit hard.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        std::process::exit(0);
    });

    let json_mode = args.json;
    loop {
        tokio::select! {
            biased;
            // Pump outbound from stdin / signal handler.
            send = out_rx.recv() => {
                match send {
                    Some(msg) => {
                        if let Err(e) = ws.send(msg).await {
                            tracing::debug!("ws send: {e}");
                            break;
                        }
                    }
                    None => break,
                }
            }
            // Pump inbound from the agent.
            recv = ws.next() => {
                match recv {
                    Some(Ok(Message::Text(txt))) => render_inbound(&txt, json_mode),
                    Some(Ok(Message::Close(frame))) => {
                        if let Some(f) = frame {
                            eprintln!("session closed: {} {}", f.code, f.reason);
                        } else {
                            eprintln!("session closed");
                        }
                        break;
                    }
                    Some(Ok(_)) => {
                        // Ping/Pong/Binary — tungstenite handles ping/pong;
                        // ACP doesn't use binary frames.
                    }
                    Some(Err(e)) => {
                        eprintln!("ws error: {e}");
                        break;
                    }
                    None => {
                        eprintln!("session ended");
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Look up which `runtime_node` hosts this agent by scanning the federated
/// node listings. Returns the first hit. Errors include enough context for
/// the operator to use `--node-id` as a workaround.
async fn resolve_node_id(client: &MissionControlClient, agent_id: &str) -> Result<String> {
    let nodes: Value = client
        .get_json("/runtime/nodes")
        .await
        .context("list runtime nodes")?;
    let arr = nodes
        .as_array()
        .ok_or_else(|| anyhow!("/runtime/nodes did not return an array"))?;
    for node in arr {
        let Some(nid) = node.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let path = format!("/runtime/nodes/{nid}/agents");
        let agents: Value = match client.get_json(&path).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let agents_arr = match agents.as_array() {
            Some(a) => a,
            None => continue,
        };
        for a in agents_arr {
            let matches = a
                .get("public_id")
                .and_then(|v| v.as_str())
                .map(|s| s == agent_id)
                .unwrap_or(false)
                || a.get("agent_public_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s == agent_id)
                    .unwrap_or(false)
                || a.get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s == agent_id)
                    .unwrap_or(false);
            if matches {
                return Ok(nid.to_string());
            }
        }
    }
    Err(anyhow!("agent {agent_id} not found on any registered node"))
}

fn build_ws_url(base: &str, node_id: &str, agent_id: &str) -> Result<String> {
    let trimmed = base.trim_end_matches('/');
    let ws_base = if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        bail!("base url has no http/https scheme: {base}");
    };
    Ok(format!(
        "{ws_base}/runtime/nodes/{node_id}/agents/{agent_id}/attach"
    ))
}

/// Format one inbound text frame for the terminal. Currently handles the
/// subset of `SessionUpdate` variants the supervisor's `pump_acp` actually
/// emits today; anything else falls back to a one-line JSON dump so we
/// never silently drop the frame.
fn render_inbound(txt: &str, json_mode: bool) {
    if json_mode {
        println!("{txt}");
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(txt) else {
        println!("{txt}");
        return;
    };

    // The hello frame from pump_acp ({"kind":"hello","protocol":"acp/1"}).
    if v.get("kind").and_then(|k| k.as_str()) == Some("hello") {
        eprintln!("[acp] connected (protocol {})", v.get("protocol").and_then(|p| p.as_str()).unwrap_or("?"));
        return;
    }

    // SessionNotification: { sessionId, update: { sessionUpdate, ... } }
    let Some(update) = v.get("update") else {
        println!("{}", v);
        return;
    };
    let kind = update
        .get("sessionUpdate")
        .and_then(|k| k.as_str())
        .unwrap_or("");
    match kind {
        "agent_message_chunk" => {
            if let Some(text) = update
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(|t| t.as_str())
            {
                print!("{text}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
        "agent_thought_chunk" => {
            if let Some(text) = update
                .get("content")
                .and_then(|c| c.get("text"))
                .and_then(|t| t.as_str())
            {
                eprintln!("[thinking] {text}");
            }
        }
        "tool_call" => {
            let title = update
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("tool");
            eprintln!("[tool] {title}");
        }
        "tool_call_update" => {
            let status = update
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let title = update
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("tool");
            if matches!(status, "completed" | "failed" | "cancelled") {
                eprintln!("[tool] {title} {status}");
            }
        }
        "plan" => {
            eprintln!("[plan] {}", update);
        }
        _ => {
            // Unknown update kind — fall back to JSON so we don't lose data.
            println!("{}", v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_to_ws_scheme() {
        let u = build_ws_url("http://localhost:8008/", "node-1", "aria-work-abc12345").unwrap();
        assert_eq!(
            u,
            "ws://localhost:8008/runtime/nodes/node-1/agents/aria-work-abc12345/attach"
        );
    }

    #[test]
    fn https_to_wss_scheme() {
        let u = build_ws_url("https://missioncontrol/", "n", "a").unwrap();
        assert_eq!(u, "wss://missioncontrol/runtime/nodes/n/agents/a/attach");
    }

    #[test]
    fn missing_scheme_errors() {
        assert!(build_ws_url("missioncontrol", "n", "a").is_err());
    }
}
