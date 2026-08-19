//! `edgeplane serve` — stdio JSON-RPC 2.0 MCP server.
//!
//! Speaks the MCP protocol over stdin/stdout (Content-Length framing), proxying
//! tool calls to the Edgeplane backend. Designed to be the single binary
//! remote agents install: `edgeplane serve` in `mcpServers.command`.
//!
//! ## Usage
//!
//! ```text
//! edgeplane serve
//! # or with debug logging:
//! edgeplane serve --debug-protocol
//! ```
//!
//! ## Protocol
//!
//! - JSON-RPC 2.0 over stdio with Content-Length framing (same as LSP)
//! - Protocol version: "2024-11-05"
//! - Methods: initialize, initialized, tools/list, tools/call, ping
//!
//! ## Reliability design
//!
//! 1. Cache is warmed *synchronously* inside the `initialized` handler before
//!    `notifications/tools/list_changed` is sent. This eliminates the race
//!    where Claude Code calls `tools/list` before the (formerly background)
//!    warm-up task completes.
//!
//! 2. If the backend is down at init time, a retry task runs with exponential
//!    backoff. When tools become available it sends a fresh `listChanged`
//!    notification through an mpsc channel that the main loop writes out.
//!
//! 3. `fetch_tools` returns an empty list on transient errors rather than
//!    propagating them as JSON-RPC errors to the client.

use crate::{client::EdgeplaneClient, mcp_stdio, mcp_tools};
use anyhow::{Context, Result};
use clap::Args;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::BufReader;
use tokio::sync::mpsc;

const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const LEGACY_PROTOCOL_VERSION: &str = "2024-11-05";

// ── CLI args ──────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct ServeMcpArgs {
    /// Tools cache TTL in seconds (default: 60)
    #[arg(long, default_value = "60")]
    pub tools_cache_ttl: u64,

    /// Run a preflight health check before entering the message loop.
    ///
    /// Disabled by default because an stdio MCP server must respond to
    /// `initialize` immediately; blocking on a network call delays startup
    /// and causes agents (e.g. Codex) to time out waiting for the handshake.
    /// Enable only when invoking `edgeplane serve` outside an agent context.
    #[arg(long)]
    pub preflight: bool,

    /// Log MCP messages to stderr for debugging
    #[arg(long)]
    pub debug_protocol: bool,
}

// ── Tool cache ────────────────────────────────────────────────────────────────

struct ToolsCache {
    tools: Vec<Value>,
    fetched_at: Option<Instant>,
    ttl: Duration,
}

impl ToolsCache {
    fn new(ttl_secs: u64) -> Self {
        Self {
            tools: Vec::new(),
            fetched_at: None,
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    fn is_fresh(&self) -> bool {
        self.fetched_at
            .map(|t| t.elapsed() < self.ttl)
            .unwrap_or(false)
    }

    fn set(&mut self, tools: Vec<Value>) {
        self.tools = tools;
        self.fetched_at = Some(Instant::now());
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(args: &ServeMcpArgs, client: &EdgeplaneClient) -> Result<()> {
    // Optional preflight: verify connectivity before entering the message loop.
    // Off by default — stdio servers must respond to `initialize` immediately.
    if args.preflight {
        client.get_json("/mcp/health").await.context(
            "preflight health check failed — run `edgeplane auth login` and verify EP_BASE_URL",
        )?;
        tracing::debug!("mcp_server: preflight ok");
    }

    let cache = Arc::new(Mutex::new(ToolsCache::new(args.tools_cache_ttl)));
    let debug = args.debug_protocol;

    // Channel for background tasks (retry warm-up) to push outbound notifications.
    let (notif_tx, mut notif_rx) = mpsc::channel::<Value>(8);

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();

    // Track the framing format negotiated during the session (default CL).
    let mut session_format = mcp_stdio::MessageFormat::ContentLength;

    loop {
        tokio::select! {
            // Outbound notifications sent by background retry task.
            Some(notif) = notif_rx.recv() => {
                let serialized = serde_json::to_string(&notif)?;
                if debug {
                    eprintln!("edgeplane serve --> (bg) {}", serialized);
                }
                mcp_stdio::write_message(&mut stdout, &serialized, session_format).await?;
            }

            // Inbound messages from the agent host.
            result = mcp_stdio::read_next_message(&mut reader) => {
                let (raw, format) = match result {
                    Ok(Some(msg)) => msg,
                    Ok(None) => break, // EOF — host closed the pipe
                    Err(e) => {
                        tracing::warn!("mcp_server: failed to read message: {}", e);
                        break;
                    }
                };

                // Remember framing format for outbound notifications.
                session_format = format;

                if debug {
                    eprintln!("edgeplane serve <-- {}", raw);
                }

                let (response, follow_up) = match serde_json::from_str::<Value>(&raw) {
                    Ok(msg) => dispatch(msg, client, &cache, &notif_tx).await,
                    Err(e) => (Some(error_response(
                        Value::Null,
                        -32700,
                        &format!("parse error: {}", e),
                    )), None),
                };

                for msg in [response, follow_up].into_iter().flatten() {
                    let serialized = serde_json::to_string(&msg)?;
                    if debug {
                        eprintln!("edgeplane serve --> {}", serialized);
                    }
                    mcp_stdio::write_message(&mut stdout, &serialized, format).await?;
                }
            }
        }
    }

    Ok(())
}

// ── Message dispatch ──────────────────────────────────────────────────────────

async fn dispatch(
    msg: Value,
    client: &EdgeplaneClient,
    cache: &Arc<Mutex<ToolsCache>>,
    notif_tx: &mpsc::Sender<Value>,
) -> (Option<Value>, Option<Value>) {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    // Notifications (no "id" field) receive no response.
    let is_notification = msg.get("id").is_none();

    // Helper: wrap a single response with no follow-up.
    macro_rules! resp {
        ($v:expr) => {
            (Some($v), None)
        };
    }

    match method.as_str() {
        "initialize" => {
            // Client hello — return server capabilities.
            let _client_info = params.get("clientInfo");
            let negotiated_version =
                select_protocol_version(params.get("protocolVersion").and_then(|v| v.as_str()));
            resp!(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": negotiated_version,
                    "capabilities": {
                        "tools": { "listChanged": true }
                    },
                    "serverInfo": {
                        "name": "edgeplane",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }))
        }

        // After the client acknowledges initialization:
        // 1. Warm the cache synchronously so tools/list hits cache on first call.
        // 2. If warm fails (backend down), spawn a retry task that will send
        //    another listChanged once the backend becomes available.
        // 3. Send listChanged to trigger the client to call tools/list.
        "initialized" | "notifications/initialized" => {
            let warmed = match crate::mcp_tools::fetch_tools_from_backend(client).await {
                Ok(tools) if !tools.is_empty() => {
                    let count = tools.len();
                    let mut c = cache.lock().unwrap();
                    c.set(tools);
                    tracing::debug!("mcp_server: cache warmed ({} tools)", count);
                    true
                }
                Ok(_) => {
                    tracing::warn!("mcp_server: backend returned 0 tools at init; will retry");
                    false
                }
                Err(e) => {
                    tracing::warn!("mcp_server: cache warm failed: {}; will retry", e);
                    false
                }
            };

            // If warm failed, kick off a background retry with exponential backoff.
            // The retry task sends a fresh listChanged through the channel when
            // tools become available, prompting Claude Code to re-fetch the list.
            if !warmed {
                let client_clone = client.clone();
                let cache_clone = Arc::clone(cache);
                let tx = notif_tx.clone();
                tokio::spawn(async move {
                    let mut delay = Duration::from_secs(2);
                    for attempt in 1..=6u32 {
                        tokio::time::sleep(delay).await;
                        tracing::debug!("mcp_server: retry warm attempt {}", attempt);
                        match crate::mcp_tools::fetch_tools_from_backend(&client_clone).await {
                            Ok(tools) if !tools.is_empty() => {
                                let count = tools.len();
                                {
                                    let mut c = cache_clone.lock().unwrap();
                                    c.set(tools);
                                }
                                tracing::info!(
                                    "mcp_server: retry warm succeeded ({} tools); sending listChanged",
                                    count
                                );
                                let _ = tx
                                    .send(json!({
                                        "jsonrpc": "2.0",
                                        "method": "notifications/tools/list_changed",
                                        "params": {}
                                    }))
                                    .await;
                                return;
                            }
                            Ok(_) => {
                                tracing::warn!("mcp_server: retry {}: 0 tools", attempt);
                            }
                            Err(e) => {
                                tracing::warn!("mcp_server: retry {}: {}", attempt, e);
                            }
                        }
                        delay = (delay * 2).min(Duration::from_secs(30));
                    }
                    tracing::error!("mcp_server: all retry attempts exhausted; tools unavailable");
                });
            }

            // Always send listChanged immediately. If warm succeeded, tools/list
            // will hit the hot cache. If not, the retry task will send another
            // listChanged later when the backend is ready.
            //
            // Important protocol nuance: some hosts send `initialized` as a request
            // with an id (instead of a pure notification). In that case we must
            // return a regular JSON-RPC result response, and emit listChanged as a
            // separate outbound notification.  We return it as the second element of
            // the tuple so the main loop writes result first, then notification —
            // deterministically, with no spawned-task race.
            let list_changed = json!({
                "jsonrpc": "2.0",
                "method": "notifications/tools/list_changed",
                "params": {}
            });

            if msg.get("id").is_some() {
                (
                    Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
                    Some(list_changed),
                )
            } else {
                (Some(list_changed), None)
            }
        }

        "notifications/cancelled" => (None, None),

        "tools/list" => match fetch_tools(client, cache).await {
            Ok(tools) => resp!(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": tools }
            })),
            Err(e) => resp!(error_response(
                id,
                -32603,
                &format!("tools/list failed: {}", e),
            )),
        },

        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let tool_args = params.get("arguments").cloned().unwrap_or(json!({}));

            // Gateway-local meta-tools — handled without a tower round-trip.
            if tool_name == "discover" {
                let path: Vec<String> = tool_args
                    .get("path")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let deep = tool_args
                    .get("deep")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let result = crate::cli_schema::discover_to_value(&path, deep);
                let text = result_to_text(&result);
                return resp!(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": text }],
                        "isError": false
                    }
                }));
            }

            if tool_name == "exec" {
                let cli_args: Vec<String> = tool_args
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let exe = std::env::current_exe()
                    .unwrap_or_else(|_| std::path::PathBuf::from("edgeplane"));
                let output = std::process::Command::new(&exe).args(&cli_args).output();
                let result = match output {
                    Ok(out) => json!({
                        "stdout": String::from_utf8_lossy(&out.stdout),
                        "stderr": String::from_utf8_lossy(&out.stderr),
                        "exit_code": out.status.code().unwrap_or(-1)
                    }),
                    Err(e) => json!({ "error": format!("exec failed: {}", e) }),
                };
                let text = result_to_text(&result);
                return resp!(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{ "type": "text", "text": text }],
                        "isError": false
                    }
                }));
            }

            match mcp_tools::call_tool(client, None, None, &tool_name, tool_args).await {
                Ok(result) => {
                    maybe_update_context_json(&result);
                    let text = result_to_text(&result);
                    resp!(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": text }],
                            "isError": false
                        }
                    }))
                }
                Err(e) => {
                    let text = format!("tool '{}' failed: {}", tool_name, e);
                    resp!(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": text }],
                            "isError": true
                        }
                    }))
                }
            }
        }

        "ping" => resp!(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),

        _ => {
            if is_notification {
                (None, None)
            } else {
                resp!(error_response(
                    id,
                    -32601,
                    &format!("method not found: {}", method),
                ))
            }
        }
    }
}

// ── Context file update ───────────────────────────────────────────────────────

/// After a successful tool call, check if the result contains domain_id or
/// mission_id and update `$EP_INSTANCE_HOME/edgeplane/context.json` accordingly.
///
/// This keeps the context file current so the PreCompact hook script can
/// re-inject the agent's active domain/mission after compaction.
fn maybe_update_context_json(result: &Value) {
    let instance_home = match std::env::var("EP_INSTANCE_HOME") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => return,
    };
    let context_path = instance_home.join("edgeplane").join("context.json");

    // Read existing context (best-effort; skip on error).
    let mut ctx: Value = if context_path.exists() {
        match fs::read_to_string(&context_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(v) => v,
            None => return,
        }
    } else {
        return;
    };

    let obj = match ctx.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    let mut changed = false;

    // Extract domain_id from common response shapes.
    let domain_id = result
        .get("domain_id")
        .or_else(|| {
            result
                .get("id")
                .filter(|_| result.get("northstar_md").is_some())
        })
        .and_then(|v| v.as_str());
    if let Some(mid) = domain_id {
        obj.insert(
            "active_domain_id".to_string(),
            Value::String(mid.to_string()),
        );
        changed = true;
    }

    // Extract mission_id from common response shapes.
    let mission_id = result
        .get("mission_id")
        .or_else(|| {
            result
                .get("id")
                .filter(|_| result.get("workstream_md").is_some())
        })
        .and_then(|v| v.as_str());
    if let Some(kid) = mission_id {
        obj.insert(
            "active_mission_id".to_string(),
            Value::String(kid.to_string()),
        );
        changed = true;
    }

    if changed {
        obj.insert(
            "last_sync_at".to_string(),
            Value::String(chrono::Utc::now().to_rfc3339()),
        );
        if let Ok(serialized) = serde_json::to_string_pretty(&ctx) {
            let _ = fs::write(&context_path, serialized);
        }
    }
}

// ── Tools fetch with TTL cache ────────────────────────────────────────────────

async fn fetch_tools(
    client: &EdgeplaneClient,
    cache: &Arc<Mutex<ToolsCache>>,
) -> Result<Vec<Value>> {
    tracing::info!("mcp_server: fetch_tools start");
    // Check freshness under the lock; clone if still valid.
    {
        let c = cache.lock().unwrap();
        if c.is_fresh() {
            tracing::info!("mcp_server: fetch_tools cache hit ({})", c.tools.len());
            return Ok(c.tools.clone());
        }
    }

    // Cache miss — fetch from backend. Return empty list on transient failures
    // rather than propagating the error, which would cause Claude Code to see
    // a JSON-RPC error instead of an empty tool list. The retry task (spawned
    // during initialized) will send a fresh listChanged when ready.
    match mcp_tools::fetch_tools_from_backend(client).await {
        Ok(mut tools) => {
            tracing::info!(
                "mcp_server: fetch_tools backend returned {} tools",
                tools.len()
            );
            // Inject gateway-local meta-tools at the end of the list.
            tools.extend(gateway_meta_tools());
            let mut c = cache.lock().unwrap();
            c.set(tools.clone());
            Ok(tools)
        }
        Err(e) => {
            tracing::warn!("mcp_server: fetch_tools error: {}; returning empty list", e);
            // Still advertise the meta-tools even when the backend is down.
            Ok(gateway_meta_tools())
        }
    }
}

/// The two gateway-local meta-tools injected alongside the tower's runtime set.
fn gateway_meta_tools() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "discover",
            "description": "Walk the edgeplane CLI command tree. Returns the capability subtree at `path` (default: top-level nouns). Use discover('domain') to see CRUD subcommands, discover('--deep') for the full tree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Drill into a subcommand path, e.g. [\"domain\"] or [\"agent\", \"signal\"]. Omit for top-level."
                    },
                    "deep": {
                        "type": "boolean",
                        "description": "Return the full subtree (default: 1 level)."
                    }
                }
            }
        }),
        serde_json::json!({
            "name": "exec",
            "description": "Run an edgeplane CLI command and return its output. Use this to call any management operation. Example: exec([\"domain\", \"list\", \"--output\", \"json\"])",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "CLI arguments to pass to edgeplane, e.g. [\"domain\", \"create\", \"--name\", \"my-domain\"]"
                    }
                },
                "required": ["args"]
            }
        }),
    ]
}

// ── I/O helpers ───────────────────────────────────────────────────────────────

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

/// Flatten a backend result Value into a human-readable string for MCP content.
fn result_to_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn select_protocol_version(requested: Option<&str>) -> &str {
    match requested {
        Some(version)
            if version == DEFAULT_PROTOCOL_VERSION || version == LEGACY_PROTOCOL_VERSION =>
        {
            version
        }
        _ => DEFAULT_PROTOCOL_VERSION,
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;

    #[test]
    fn selects_latest_protocol_when_client_omits_version() {
        assert_eq!(select_protocol_version(None), DEFAULT_PROTOCOL_VERSION);
    }

    #[test]
    fn echoes_known_requested_protocol_versions() {
        assert_eq!(
            select_protocol_version(Some(DEFAULT_PROTOCOL_VERSION)),
            DEFAULT_PROTOCOL_VERSION
        );
        assert_eq!(
            select_protocol_version(Some(LEGACY_PROTOCOL_VERSION)),
            LEGACY_PROTOCOL_VERSION
        );
    }

    #[test]
    fn falls_back_to_latest_for_unknown_versions() {
        assert_eq!(
            select_protocol_version(Some("2099-01-01")),
            DEFAULT_PROTOCOL_VERSION
        );
    }
}
