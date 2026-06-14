use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Sse},
    routing::{get, post},
};
use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};
use std::{convert::Infallible, net::SocketAddr};
use tokio::{
    io::BufReader,
    net::TcpListener,
    sync::{broadcast, mpsc},
};
use tokio_stream::wrappers::BroadcastStream;

use crate::mcp_stdio;

const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Subcommand, Debug)]
pub enum ChannelCommand {
    /// Claude channel integrations.
    #[command(subcommand)]
    Claude(ClaudeChannelCommand),
}

#[derive(Subcommand, Debug)]
pub enum ClaudeChannelCommand {
    /// Expose a local webhook that forwards inbound messages to Claude via channel notifications.
    Webhook(ClaudeWebhookArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ClaudeWebhookArgs {
    /// Host/interface for the local webhook listener.
    #[arg(long, default_value = "127.0.0.1")]
    pub listen_host: String,

    /// Port for the local webhook listener.
    #[arg(long, default_value_t = 8788)]
    pub listen_port: u16,

    /// Name used in channel metadata.
    #[arg(long, default_value = "edgeplane")]
    pub channel_name: String,

    /// Optional instructions to pass to Claude for this channel.
    #[arg(long)]
    pub instructions: Option<String>,

    /// Expose a standard MCP reply tool.
    #[arg(long, default_value_t = false)]
    pub enable_reply: bool,

    /// Log protocol traffic to stderr.
    #[arg(long, default_value_t = false)]
    pub debug_protocol: bool,
}

#[derive(Clone)]
struct WebhookState {
    tx: mpsc::Sender<Value>,
    replies: broadcast::Sender<Value>,
    source: String,
}

#[derive(Clone)]
struct ChannelRuntimeConfig {
    channel_name: String,
    instructions: Option<String>,
    enable_reply: bool,
    debug_protocol: bool,
}

pub async fn run(command: ChannelCommand) -> Result<()> {
    match command {
        ChannelCommand::Claude(ClaudeChannelCommand::Webhook(args)) => {
            let runtime = ChannelRuntimeConfig {
                channel_name: args.channel_name,
                instructions: args.instructions,
                enable_reply: args.enable_reply,
                debug_protocol: args.debug_protocol,
            };
            let transport = ChannelTransport::Webhook {
                listen_host: args.listen_host,
                listen_port: args.listen_port,
            };
            run_claude_channel(runtime, transport).await
        }
    }
}

enum ChannelTransport {
    Webhook {
        listen_host: String,
        listen_port: u16,
    },
}

async fn run_claude_channel(
    runtime: ChannelRuntimeConfig,
    transport: ChannelTransport,
) -> Result<()> {
    let (inbound_tx, mut inbound_rx) = mpsc::channel::<Value>(256);
    let (reply_tx, _reply_rx) = broadcast::channel::<Value>(128);

    let server = match transport {
        ChannelTransport::Webhook {
            listen_host,
            listen_port,
        } => Some(start_webhook_transport(
            listen_host,
            listen_port,
            runtime.channel_name.clone(),
            inbound_tx.clone(),
            reply_tx.clone(),
        )?),
    };

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut session_format = mcp_stdio::MessageFormat::ContentLength;

    loop {
        tokio::select! {
            inbound = inbound_rx.recv() => {
                let Some(notif) = inbound else {
                    break;
                };
                let serialized = serde_json::to_string(&notif)?;
                if runtime.debug_protocol {
                    eprintln!("edgeplane channel --> {}", serialized);
                }
                mcp_stdio::write_message(&mut stdout, &serialized, session_format).await?;
            }
            result = mcp_stdio::read_next_message(&mut reader) => {
                let (raw, format) = match result {
                    Ok(Some(msg)) => msg,
                    Ok(None) => break,
                    Err(e) => return Err(e).context("failed reading channel stdio message"),
                };
                session_format = format;
                if runtime.debug_protocol {
                    eprintln!("edgeplane channel <-- {}", raw);
                }
                let msg: Value = serde_json::from_str(&raw).context("invalid json-rpc payload")?;
                if let Some(response) = dispatch_claude_channel(&msg, &runtime, &reply_tx).await? {
                    let serialized = serde_json::to_string(&response)?;
                    if runtime.debug_protocol {
                        eprintln!("edgeplane channel --> {}", serialized);
                    }
                    mcp_stdio::write_message(&mut stdout, &serialized, format).await?;
                }
            }
        }
    }

    if let Some(handle) = server {
        handle.abort();
    }
    Ok(())
}

fn start_webhook_transport(
    listen_host: String,
    listen_port: u16,
    source: String,
    tx: mpsc::Sender<Value>,
    replies: broadcast::Sender<Value>,
) -> Result<tokio::task::JoinHandle<Result<()>>> {
    let bind_addr: SocketAddr = format!("{}:{}", listen_host, listen_port)
        .parse()
        .context("invalid listen host/port")?;

    let app_state = WebhookState {
        tx,
        replies,
        source,
    };

    let app = Router::new()
        .route("/", post(webhook_receive))
        .route("/healthz", get(webhook_healthz))
        .route("/events", get(webhook_events))
        .with_state(app_state);

    Ok(tokio::spawn(async move {
        let listener = TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("failed to bind webhook listener at {bind_addr}"))?;
        axum::serve(listener, app)
            .await
            .context("channel webhook server exited unexpectedly")
    }))
}

async fn dispatch_claude_channel(
    msg: &Value,
    runtime: &ChannelRuntimeConfig,
    reply_tx: &broadcast::Sender<Value>,
) -> Result<Option<Value>> {
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = msg.get("id").cloned().unwrap_or(Value::Null);

    if method.is_empty() {
        return Ok(None);
    }

    let is_notification = msg.get("id").is_none();

    let response = match method {
        "initialize" => {
            let mut capabilities = Map::new();
            capabilities.insert("experimental".to_string(), json!({ "claude/channel": {} }));
            if runtime.enable_reply {
                capabilities.insert("tools".to_string(), json!({ "listChanged": false }));
            }

            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": Value::Object(capabilities),
                    "serverInfo": {
                        "name": format!("{}-channel", runtime.channel_name),
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": runtime.instructions.clone().unwrap_or_else(default_instructions),
                }
            }))
        }
        "initialized" | "notifications/initialized" => {
            if msg.get("id").is_some() {
                Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} }))
            } else {
                None
            }
        }
        "tools/list" => {
            let tools = if runtime.enable_reply {
                vec![json!({
                    "name": "reply",
                    "description": "Send a reply to a channel conversation.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "chat_id": { "type": "string" },
                            "text": { "type": "string" }
                        },
                        "required": ["chat_id", "text"],
                        "additionalProperties": true
                    }
                })]
            } else {
                Vec::new()
            };
            Some(json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } }))
        }
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !runtime.enable_reply {
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": "reply tool is disabled" }],
                        "isError": true
                    }
                }))
            } else if name != "reply" {
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": format!("unknown tool: {name}") }],
                        "isError": true
                    }
                }))
            } else {
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let chat_id = arguments
                    .get("chat_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let text = arguments
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();

                if chat_id.trim().is_empty() || text.trim().is_empty() {
                    Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": "reply requires chat_id and text" }],
                            "isError": true
                        }
                    }))
                } else {
                    let reply_event = json!({
                        "ts": chrono::Utc::now().to_rfc3339(),
                        "chat_id": chat_id,
                        "text": text,
                    });
                    let _ = reply_tx.send(reply_event);
                    Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": "ok" }],
                            "isError": false
                        }
                    }))
                }
            }
        }
        "ping" => Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
        _ if is_notification => None,
        _ => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {method}") }
        })),
    };

    Ok(response)
}

async fn webhook_receive(
    State(state): State<WebhookState>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let notification = build_channel_notification(&state.source, body)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    state.tx.send(notification).await.map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "channel queue unavailable".to_string(),
        )
    })?;
    Ok(Json(json!({ "ok": true })))
}

async fn webhook_healthz() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

async fn webhook_events(State(state): State<WebhookState>) -> impl IntoResponse {
    let stream = tokio_stream::StreamExt::filter_map(
        BroadcastStream::new(state.replies.subscribe()),
        |item| match item {
            Ok(payload) => Some(Ok::<axum::response::sse::Event, Infallible>(
                axum::response::sse::Event::default()
                    .event("reply")
                    .data(payload.to_string()),
            )),
            Err(_) => None,
        },
    );
    Sse::new(stream)
}

fn build_channel_notification(source: &str, body: Value) -> Result<Value> {
    if body
        .get("jsonrpc")
        .and_then(Value::as_str)
        .is_some_and(|v| v == "2.0")
        && body
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|v| v == "notifications/claude/channel")
    {
        return Ok(body);
    }

    let text = extract_text(&body).ok_or_else(|| anyhow!("missing inbound text/content field"))?;
    let mut meta = normalize_meta(body.get("meta"));

    if let Some(chat_id) = body.get("chat_id").and_then(Value::as_str) {
        meta.insert("chat_id".to_string(), Value::String(chat_id.to_string()));
    }

    meta.insert("source".to_string(), Value::String(source.to_string()));

    Ok(json!({
        "jsonrpc": "2.0",
        "method": "notifications/claude/channel",
        "params": {
            "content": text,
            "meta": meta,
        }
    }))
}

fn extract_text(body: &Value) -> Option<String> {
    ["text", "content", "message", "body"]
        .iter()
        .find_map(|key| {
            body.get(*key)
                .and_then(Value::as_str)
                .map(|v| v.to_string())
        })
}

fn normalize_meta(meta: Option<&Value>) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(Value::Object(obj)) = meta {
        for (k, v) in obj {
            let normalized = k
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            if normalized.is_empty() {
                continue;
            }
            out.insert(normalized, v.clone());
        }
    }
    out
}

fn default_instructions() -> String {
    "Messages arrive via <channel> with metadata in meta (including chat_id when available). Use the reply tool for conversational responses and pass the same chat_id.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_notification_normalizes_meta_keys() {
        let notif = build_channel_notification(
            "edgeplane",
            json!({
                "text": "hello",
                "chat_id": "c1",
                "meta": {
                    "thread-id": "t1",
                    "ok_key": "v"
                }
            }),
        )
        .expect("notification");

        let params = notif
            .get("params")
            .and_then(Value::as_object)
            .expect("params");
        let meta = params.get("meta").and_then(Value::as_object).expect("meta");
        assert_eq!(meta.get("thread_id").and_then(Value::as_str), Some("t1"));
        assert_eq!(meta.get("ok_key").and_then(Value::as_str), Some("v"));
        assert_eq!(meta.get("chat_id").and_then(Value::as_str), Some("c1"));
    }

    #[test]
    fn passthrough_notification_is_preserved() {
        let input = json!({
            "jsonrpc": "2.0",
            "method": "notifications/claude/channel",
            "params": {"content": "x", "meta": {}}
        });
        let notif = build_channel_notification("edgeplane", input.clone()).expect("ok");
        assert_eq!(notif, input);
    }
}
