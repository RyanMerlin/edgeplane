/// Management gateway — Unix socket + TCP listener serving JSON-RPC 2.0.
///
/// Unix socket: `~/.missioncontrol/mgmt.sock` (mode 0600, no auth)
/// TCP socket:  `0.0.0.0:<MC_MESH_MGMT_PORT>` (default 7731)
///              Requires AUTH handshake when `MC_TOKEN` env var is set.
///
/// Both endpoints serve the same JSON-RPC 2.0 protocol (newline-delimited).
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use mcd_core::capability_dispatcher::{CapabilityDispatcher, DispatchRequest};
use mcd_core::paths;
use mcd_core::types::AgentSignal;
use mcd_packs::PackRegistry;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::attach_gateway::RuntimeMap;
use crate::local_registry::LocalRegistry;
use crate::supervisor::Supervisor;

// ─── MgmtGateway ─────────────────────────────────────────────────────────────

pub struct MgmtGateway {
    dispatcher: Arc<CapabilityDispatcher>,
    registry: Arc<PackRegistry>,
    mc_token: Option<String>,
    socket_path: PathBuf,
    tcp_port: u16,
    /// Local agent ops dependencies. Populated by `daemon::run` after the
    /// supervisor + runtime_map are wired up. Required for the
    /// `agent.local.*` and `agent.describe_local` JSON-RPC methods that
    /// Phase 3 added; the legacy `dispatch` / `capabilities.*` methods
    /// don't read it.
    agent_ops: Option<Arc<AgentOpsHandle>>,
}

/// Shared deps the JSON-RPC `agent.*` handlers need. Constructed once by
/// `daemon::run`; `LocalRegistry` is opened per-call inside the handlers
/// (rusqlite::Connection is !Sync so we can't hold one here) using
/// `registry_path` as the address.
pub struct AgentOpsHandle {
    pub supervisor: Arc<Supervisor>,
    pub runtime_map: RuntimeMap,
    pub registry_path: PathBuf,
    /// Cron handle for `agent.cron.reload`. `None` when the daemon
    /// didn't spawn a cron loop (e.g. test harnesses).
    pub cron: Option<crate::cron::CronHandle>,
    /// Path to `cron.toml` for the `agent.cron.list/describe` handlers
    /// to re-parse on demand. `None` when cron is not wired.
    pub cron_config_path: Option<PathBuf>,
    /// Broadcast sender for Phase 5 SupervisorEvents. `None` when the
    /// unit-health loop wasn't spawned (test harnesses). Subscribers
    /// can clone this and call `.subscribe()` to get a receiver.
    pub supervisor_events:
        Option<tokio::sync::broadcast::Sender<mcd_core::types::SupervisorEvent>>,
}

impl MgmtGateway {
    pub fn new(dispatcher: Arc<CapabilityDispatcher>, registry: Arc<PackRegistry>) -> Self {
        let mc_token = mcd_core::paths::state_file_path()
            .parent()
            .and_then(|_| {
                let content = std::fs::read_to_string(mcd_core::paths::state_file_path()).ok()?;
                let v: serde_json::Value = serde_json::from_str(&content).ok()?;
                let active = v.get("active_profile")?.as_str()?;
                let token = v.get("profiles")?.get(active)?.get("auth")?.get("token")?.as_str()?;
                if token.is_empty() { None } else { Some(token.to_string()) }
            });
        let tcp_port = std::env::var("MC_MESH_MGMT_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(7731);
        let socket_path = paths::mgmt_socket_path();

        MgmtGateway {
            dispatcher,
            registry,
            mc_token,
            socket_path,
            tcp_port,
            agent_ops: None,
        }
    }

    /// Wire the agent-ops handle (supervisor + runtime_map + registry path).
    /// Required before serving the `agent.local.*` and `agent.describe_local`
    /// JSON-RPC methods. Builder-style so existing callsites that only need
    /// capabilities dispatch don't have to construct the handle.
    pub fn with_agent_ops(mut self, ops: AgentOpsHandle) -> Self {
        self.agent_ops = Some(Arc::new(ops));
        self
    }

    pub async fn run(self) -> Result<()> {
        let gateway = Arc::new(self);

        let unix_gw = Arc::clone(&gateway);
        let tcp_gw = Arc::clone(&gateway);

        let unix_handle = tokio::spawn(async move {
            if let Err(e) = unix_gw.run_unix().await {
                tracing::error!("mgmt unix listener error: {e}");
            }
        });

        let tcp_handle = tokio::spawn(async move {
            if let Err(e) = tcp_gw.run_tcp().await {
                tracing::error!("mgmt tcp listener error: {e}");
            }
        });

        let _ = tokio::join!(unix_handle, tcp_handle);
        Ok(())
    }

    async fn run_unix(self: &Arc<Self>) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        use tokio::net::UnixListener;

        let path = &self.socket_path;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Remove stale socket from a previous run.
        let _ = std::fs::remove_file(path);

        let listener = UnixListener::bind(path)
            .map_err(|e| anyhow::anyhow!("mgmt unix bind {}: {e}", path.display()))?;

        // Restrict to owner only.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;

        tracing::info!("mgmt unix socket listening on {}", path.display());

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let gw = Arc::clone(self);
                    tokio::spawn(async move {
                        // Unix connections are always considered authenticated.
                        if let Err(e) = gw.handle_connection(stream).await {
                            tracing::debug!("mgmt unix session ended: {e}");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!("mgmt unix accept error: {e}");
                }
            }
        }
    }

    async fn run_tcp(self: &Arc<Self>) -> Result<()> {
        use tokio::net::TcpListener;

        let addr = format!("0.0.0.0:{}", self.tcp_port);
        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| anyhow::anyhow!("mgmt tcp bind {addr}: {e}"))?;

        tracing::info!("mgmt tcp listener on {addr}");

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    tracing::debug!("mgmt tcp connection from {peer}");
                    let gw = Arc::clone(self);
                    tokio::spawn(async move {
                        if let Err(e) = gw.handle_tcp_connection(stream).await {
                            tracing::debug!("mgmt tcp session ended: {e}");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!("mgmt tcp accept error: {e}");
                }
            }
        }
    }

    /// Handle a TCP connection — AUTH handshake before JSON-RPC.
    async fn handle_tcp_connection(&self, stream: tokio::net::TcpStream) -> Result<()> {
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        // AUTH handshake only when MC_TOKEN is configured.
        if let Some(expected_token) = &self.mc_token {
            let mut line = String::new();
            reader.read_line(&mut line).await?;
            let line = line.trim();

            if let Some(token) = line.strip_prefix("AUTH ") {
                if token == expected_token.as_str() {
                    write_half.write_all(b"OK\n").await?;
                } else {
                    write_half.write_all(b"ERR unauthorized\n").await?;
                    return Ok(());
                }
            } else {
                write_half.write_all(b"ERR unauthorized\n").await?;
                return Ok(());
            }
        }

        // Rejoin halves into a unified async stream for handle_connection.
        // We use a wrapper that chains our already-buffered reader with the write half.
        handle_jsonrpc_loop(
            &self.dispatcher,
            &self.registry,
            self.agent_ops.as_ref(),
            reader,
            write_half,
        )
        .await
    }

    /// Handle a Unix socket connection — always authenticated.
    async fn handle_connection<S>(&self, stream: S) -> Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        let reader = BufReader::new(read_half);
        handle_jsonrpc_loop(
            &self.dispatcher,
            &self.registry,
            self.agent_ops.as_ref(),
            reader,
            write_half,
        )
        .await
    }
}

// ─── JSON-RPC loop ────────────────────────────────────────────────────────────

async fn handle_jsonrpc_loop<R, W>(
    dispatcher: &CapabilityDispatcher,
    registry: &PackRegistry,
    agent_ops: Option<&Arc<AgentOpsHandle>>,
    mut reader: BufReader<R>,
    mut writer: W,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            // EOF — client disconnected.
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Peek at the method to detect streaming subscriptions. Streaming
        // hijacks the connection — after ack, the gateway pushes
        // newline-delimited event frames until the client disconnects.
        // No further JSON-RPC requests are processed on this connection.
        if let Some(req) = serde_json::from_str::<Value>(trimmed).ok() {
            if req.get("method").and_then(|v| v.as_str()) == Some("events.subscribe") {
                let id = req.get("id").cloned().unwrap_or(Value::Null);
                let sender = agent_ops.and_then(|ops| ops.supervisor_events.as_ref());
                stream_supervisor_events(sender, id, &mut writer).await?;
                // Connection is done after a stream ends (always EOF or fatal lag).
                break;
            }
        }

        let response = dispatch_jsonrpc(dispatcher, registry, agent_ops, trimmed).await;
        let mut response_bytes = serde_json::to_vec(&response)
            .unwrap_or_else(|_| br#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"serialization error"}}"#.to_vec());
        response_bytes.push(b'\n');
        writer.write_all(&response_bytes).await?;
    }
    Ok(())
}

/// Stream SupervisorEvents to the client until disconnect or fatal broadcast
/// lag. Sends one ack frame (`{"ok": true, "subscribed": true}`), then one JSON
/// frame per event. Lagged subscribers receive `{"ok": false, "error": "lag",
/// "skipped": N}` and the stream terminates.
async fn stream_supervisor_events<W>(
    sender: Option<&tokio::sync::broadcast::Sender<mcd_core::types::SupervisorEvent>>,
    id: Value,
    writer: &mut W,
) -> Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::sync::broadcast::error::RecvError;

    // No broadcast sender means either agent_ops isn't wired (test harnesses)
    // or the unit-health loop isn't running. Either way: return JSON-RPC
    // error and close the connection.
    let Some(sender) = sender else {
        let err = jsonrpc_error(id, -32603, "supervisor events not wired (unit-health loop not running)");
        let mut bytes = serde_json::to_vec(&err)?;
        bytes.push(b'\n');
        writer.write_all(&bytes).await?;
        return Ok(());
    };
    let mut rx = sender.subscribe();

    // Ack — tells the client the stream is live. After this frame, no further
    // JSON-RPC responses; only `SupervisorEvent` frames serialized via serde.
    let ack = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "subscribed": true }
    });
    let mut ack_bytes = serde_json::to_vec(&ack)?;
    ack_bytes.push(b'\n');
    writer.write_all(&ack_bytes).await?;
    writer.flush().await?;

    loop {
        match rx.recv().await {
            Ok(event) => {
                let mut bytes = serde_json::to_vec(&event)?;
                bytes.push(b'\n');
                if writer.write_all(&bytes).await.is_err() {
                    // Client gone.
                    break;
                }
                let _ = writer.flush().await;
            }
            Err(RecvError::Lagged(skipped)) => {
                let frame = serde_json::json!({
                    "ok": false,
                    "error": "lag",
                    "skipped": skipped,
                });
                let mut bytes = serde_json::to_vec(&frame)?;
                bytes.push(b'\n');
                let _ = writer.write_all(&bytes).await;
                let _ = writer.flush().await;
                break;
            }
            Err(RecvError::Closed) => break,
        }
    }
    Ok(())
}

async fn dispatch_jsonrpc(
    dispatcher: &CapabilityDispatcher,
    registry: &PackRegistry,
    agent_ops: Option<&Arc<AgentOpsHandle>>,
    raw: &str,
) -> Value {
    // Parse the request.
    let req: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => {
            return jsonrpc_error(Value::Null, -32700, "parse error");
        }
    };

    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = match req.get("method").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => return jsonrpc_error(id, -32600, "invalid request: missing method"),
    };
    let params = req.get("params").cloned().unwrap_or(Value::Object(Default::default()));

    match method {
        "dispatch" => handle_dispatch(dispatcher, id, &params).await,
        "capabilities.list" => handle_capabilities_list(registry, id, &params),
        "capabilities.describe" => handle_capabilities_describe(registry, id, &params),
        "agent.local.signal" => match agent_ops {
            Some(ops) => handle_agent_local_signal(ops, id, &params).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.local.list" => match agent_ops {
            Some(ops) => handle_agent_local_list(ops, id).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.describe_local" => match agent_ops {
            Some(ops) => handle_agent_describe_local(ops, id, &params).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.cron.list" => match agent_ops {
            Some(ops) => handle_agent_cron_list(ops, id).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.cron.describe" => match agent_ops {
            Some(ops) => handle_agent_cron_describe(ops, id, &params).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.cron.reload" => match agent_ops {
            Some(ops) => handle_agent_cron_reload(ops, id),
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.cron.history" => match agent_ops {
            Some(ops) => handle_agent_cron_history(ops, id, &params).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.cron.gc_now" => match agent_ops {
            Some(ops) => handle_agent_cron_gc_now(ops, id, &params).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.supervise.list" => match agent_ops {
            Some(ops) => handle_supervise_list(ops, id).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.supervise.status" => match agent_ops {
            Some(ops) => handle_supervise_status(ops, id, &params).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.supervise.restart" => match agent_ops {
            Some(ops) => handle_supervise_restart(ops, id, &params).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.supervise.pause" => match agent_ops {
            Some(ops) => handle_supervise_pause_or_resume(ops, id, &params, true).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.supervise.resume" => match agent_ops {
            Some(ops) => handle_supervise_pause_or_resume(ops, id, &params, false).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        "agent.supervise.history" => match agent_ops {
            Some(ops) => handle_supervise_history(ops, id, &params).await,
            None => jsonrpc_error(id, -32603, "agent ops not wired"),
        },
        _ => jsonrpc_error(id, -32601, "method not found"),
    }
}

async fn handle_dispatch(dispatcher: &CapabilityDispatcher, id: Value, params: &Value) -> Value {
    let full_name = match params.get("full_name").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return jsonrpc_error(id, -32602, "invalid params: missing full_name"),
    };

    let args = params.get("args").cloned().unwrap_or(serde_json::json!({}));
    let dry_run = params.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
    let timeout_secs = params.get("timeout_secs").and_then(|v| v.as_u64());
    let mission_id = params.get("mission_id").and_then(|v| v.as_str()).map(String::from);
    let agent_id = params.get("agent_id").and_then(|v| v.as_str()).map(String::from);

    let profile = params.get("profile")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let env_str = params.get("env")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let req = DispatchRequest {
        full_name,
        args,
        profile,
        env: env_str,
        dry_run,
        timeout_secs,
        mission_id,
        agent_id,
    };

    let result = dispatcher.dispatch(req).await;

    jsonrpc_result(id, serde_json::json!({
        "ok": result.ok,
        "data": result.data,
        "receipt_id": result.receipt_id,
        "execution_time_ms": result.execution_time_ms,
        "exit_code": result.exit_code,
        "hint": result.hint,
        "example": result.example,
    }))
}

fn handle_capabilities_list(registry: &PackRegistry, id: Value, params: &Value) -> Value {
    let tag_filter = params.get("tag").and_then(|v| v.as_str());
    let summaries = registry.capabilities(tag_filter);
    let items: Vec<Value> = summaries
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.full_name,
                "summary": s.description,
                "tags": s.tags,
                "risk": s.risk.to_string(),
            })
        })
        .collect();
    jsonrpc_result(id, Value::Array(items))
}

fn handle_capabilities_describe(registry: &PackRegistry, id: Value, params: &Value) -> Value {
    let full_name = match params.get("full_name").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return jsonrpc_error(id, -32602, "invalid params: missing full_name"),
    };

    match registry.get_by_full_name(full_name) {
        Some(manifest) => match serde_json::to_value(manifest) {
            Ok(v) => jsonrpc_result(id, v),
            Err(e) => jsonrpc_error(id, -32603, &format!("internal error: {e}")),
        },
        None => jsonrpc_error(id, -32602, &format!("capability '{}' not found", full_name)),
    }
}

// ─── agent.local.* + agent.describe_local handlers (Phase 3) ──────────────

/// `agent.local.signal` — invoke `AgentRuntime::signal` on a locally
/// supervised agent. Params:
///   { agent_id, kind: "user_input"|"peer_message"|"cancel",
///     text?, from_agent_id?, channel?, body? }
///
/// Returns `{ "ok": true }` on success; structured error otherwise.
async fn handle_agent_local_signal(
    ops: &Arc<AgentOpsHandle>,
    id: Value,
    params: &Value,
) -> Value {
    let agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return jsonrpc_error(id, -32602, "missing param: agent_id"),
    };
    let kind = params.get("kind").and_then(|v| v.as_str()).unwrap_or("user_input");

    let signal = match kind {
        "user_input" => {
            let text = match params.get("text").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => return jsonrpc_error(id, -32602, "missing param: text (required for user_input)"),
            };
            AgentSignal::UserInput { text }
        }
        "cancel" => AgentSignal::Cancel,
        "peer_message" => {
            let from_agent_id = params
                .get("from_agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("local")
                .to_string();
            let channel = params
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("signal")
                .to_string();
            let body = params.get("body").cloned().unwrap_or(Value::Null);
            AgentSignal::PeerMessage { from_agent_id, channel, body }
        }
        other => {
            return jsonrpc_error(
                id,
                -32602,
                &format!("unknown signal kind '{other}' (expected user_input|peer_message|cancel)"),
            );
        }
    };

    // Look up the supervised agent. `with_agent` returns Some((runtime,
    // handle_clone)) when registered; we clone what we need so we don't
    // hold the supervisor lock while awaiting the signal call.
    let lookup = ops
        .supervisor
        .with_agent(&agent_id, |supervised| {
            (
                supervised.runtime.clone(),
                mcd_core::types::AgentHandle {
                    agent_id: supervised.handle.agent_id.clone(),
                    runtime_kind: supervised.handle.runtime_kind.clone(),
                    pid: supervised.handle.pid,
                },
            )
        })
        .await;

    let (runtime, handle) = match lookup {
        Some(pair) => pair,
        None => {
            return jsonrpc_error(
                id,
                -32004,
                &format!("agent '{agent_id}' is not supervised locally"),
            );
        }
    };

    match runtime.signal(&handle, signal).await {
        Ok(()) => jsonrpc_result(id, serde_json::json!({ "ok": true })),
        Err(e) => jsonrpc_error(id, -32000, &format!("signal failed: {e:#}")),
    }
}

/// `agent.local.list` — enumerate locally-supervised agents from the
/// registry (joined with their `agent_launch_context` row when present).
/// Used by `mc agent list` to show the local set.
async fn handle_agent_local_list(ops: &Arc<AgentOpsHandle>, id: Value) -> Value {
    let registry_path = ops.registry_path.clone();
    let agents = match tokio::task::spawn_blocking(move || list_local_agents(&registry_path)).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return jsonrpc_error(id, -32001, &format!("registry read failed: {e:#}")),
        Err(e) => return jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    };
    jsonrpc_result(id, serde_json::json!({ "agents": agents }))
}

/// `agent.describe_local` — return a description of a single agent if it
/// exists in any local registry source. The `mc` CLI calls this first when
/// auto-resolving `mc agent signal/attach/describe <id>` — found → local
/// dispatch; missing → fall through to controlplane.
async fn handle_agent_describe_local(
    ops: &Arc<AgentOpsHandle>,
    id: Value,
    params: &Value,
) -> Value {
    let agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return jsonrpc_error(id, -32602, "missing param: agent_id"),
    };

    let registry_path = ops.registry_path.clone();
    let lookup_id = agent_id.clone();
    let entry = match tokio::task::spawn_blocking(move || {
        describe_local_agent(&registry_path, &lookup_id)
    })
    .await
    {
        Ok(Ok(opt)) => opt,
        Ok(Err(e)) => return jsonrpc_error(id, -32001, &format!("registry read failed: {e:#}")),
        Err(e) => return jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    };

    match entry {
        Some(mut info) => {
            let supervised = ops.supervisor.with_agent(&agent_id, |_| ()).await.is_some();
            if let Value::Object(ref mut map) = info {
                map.insert("supervised".into(), Value::Bool(supervised));
                map.insert("found".into(), Value::Bool(true));
            }
            jsonrpc_result(id, info)
        }
        None => jsonrpc_result(id, serde_json::json!({ "found": false })),
    }
}

/// Read all agents from the local registry, joined with their launch
/// context. Returns a flat JSON array. Blocking (rusqlite); call from
/// `spawn_blocking`.
fn list_local_agents(registry_path: &std::path::Path) -> anyhow::Result<Vec<Value>> {
    let reg = LocalRegistry::open(registry_path)?;
    // Sources we consider "local" — everything except controlplane-synced
    // rows. Right now that's `local` + `fleet_import`.
    let mut out = Vec::new();
    for source in &[crate::local_registry::SOURCE_LOCAL, crate::fleet_import::SOURCE_FLEET_IMPORT] {
        for rec in reg.list_by_source(source)? {
            let lc = reg.get_launch_context(source, &rec.id)?;
            out.push(serde_json::json!({
                "agent_id": rec.id,
                "source": rec.source,
                "mission_id": rec.mission_id,
                "runtime_kind": rec.runtime_kind,
                "supervision_mode": rec.supervision_mode,
                "vault_folder": lc.as_ref().and_then(|c| c.vault_folder.clone()),
                "zellij_session": lc.as_ref().and_then(|c| c.zellij_session.clone()),
            }));
        }
    }
    Ok(out)
}

/// Look up a single agent across known local sources. Returns `None` if no
/// matching row exists in any local source. Blocking.
fn describe_local_agent(
    registry_path: &std::path::Path,
    agent_id: &str,
) -> anyhow::Result<Option<Value>> {
    let reg = LocalRegistry::open(registry_path)?;
    for source in &[crate::local_registry::SOURCE_LOCAL, crate::fleet_import::SOURCE_FLEET_IMPORT] {
        let rows = reg.list_by_source(source)?;
        if let Some(rec) = rows.into_iter().find(|r| r.id == agent_id) {
            let lc = reg.get_launch_context(source, &rec.id)?;
            return Ok(Some(serde_json::json!({
                "agent_id": rec.id,
                "source": rec.source,
                "mission_id": rec.mission_id,
                "runtime_kind": rec.runtime_kind,
                "supervision_mode": rec.supervision_mode,
                "vault_folder": lc.as_ref().and_then(|c| c.vault_folder.clone()),
                "zellij_session": lc.as_ref().and_then(|c| c.zellij_session.clone()),
            })));
        }
    }
    Ok(None)
}

// ─── agent.cron.* handlers (Phase 4) ──────────────────────────────────────

/// `agent.cron.list` — return all jobs from cron.toml joined with their
/// runtime state from SQLite. Output:
///   { "jobs": [ { name, schedule, session, dispatch, enabled,
///                 last_fired_at?, last_status?, last_error? } ] }
async fn handle_agent_cron_list(ops: &Arc<AgentOpsHandle>, id: Value) -> Value {
    let config_path = match &ops.cron_config_path {
        Some(p) => p.clone(),
        None => return jsonrpc_error(id, -32603, "cron is not wired (no config path)"),
    };
    let registry_path = ops.registry_path.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let cfg = crate::cron_config::load(&config_path)?;
        let reg = LocalRegistry::open(&registry_path)?;
        let states = reg.cron_list_state()?;

        let state_by_name: std::collections::HashMap<String, _> =
            states.into_iter().map(|s| (s.job_name.clone(), s)).collect();

        let jobs: Vec<Value> = cfg
            .jobs
            .iter()
            .map(|j| {
                let state = state_by_name.get(&j.name);
                serde_json::json!({
                    "name": j.name,
                    "schedule": j.schedule,
                    "session": j.session,
                    "dispatch": j.dispatch,
                    "enabled": j.enabled,
                    "last_fired_at": state.and_then(|s| s.last_fired_at.clone()),
                    "last_status": state.and_then(|s| s.last_status.clone()),
                    "last_error": state.and_then(|s| s.last_error.clone()),
                })
            })
            .collect();

        Ok(serde_json::json!({
            "timezone": cfg.timezone,
            "schema_version": cfg.schema_version,
            "jobs": jobs,
        }))
    })
    .await;

    match result {
        Ok(Ok(v)) => jsonrpc_result(id, v),
        Ok(Err(e)) => jsonrpc_error(id, -32001, &format!("cron list failed: {e:#}")),
        Err(e) => jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    }
}

/// `agent.cron.describe` — return one job plus recent fire history.
/// Params: `{ "name": "<job-name>", "limit"?: <int, default 5> }`
async fn handle_agent_cron_describe(
    ops: &Arc<AgentOpsHandle>,
    id: Value,
    params: &Value,
) -> Value {
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return jsonrpc_error(id, -32602, "missing param: name"),
    };
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as u32;

    let config_path = match &ops.cron_config_path {
        Some(p) => p.clone(),
        None => return jsonrpc_error(id, -32603, "cron is not wired"),
    };
    let registry_path = ops.registry_path.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let cfg = crate::cron_config::load(&config_path)?;
        let job = cfg
            .jobs
            .iter()
            .find(|j| j.name == name)
            .ok_or_else(|| anyhow::anyhow!("no job named {:?} in {}", name, config_path.display()))?;

        let reg = LocalRegistry::open(&registry_path)?;
        let state = reg.cron_get_state(&name)?;
        let history = reg.cron_history_for_job(&name, limit)?;

        Ok(serde_json::json!({
            "name": job.name,
            "schedule": job.schedule,
            "session": job.session,
            "dispatch": job.dispatch,
            "enabled": job.enabled,
            "prompt": job.prompt,
            "last_fired_at": state.as_ref().and_then(|s| s.last_fired_at.clone()),
            "last_status": state.as_ref().and_then(|s| s.last_status.clone()),
            "last_error": state.as_ref().and_then(|s| s.last_error.clone()),
            "history": history.iter().map(|h| serde_json::json!({
                "fired_at": h.fired_at,
                "status": h.status,
                "duration_ms": h.duration_ms,
                "error_message": h.error_message,
            })).collect::<Vec<_>>(),
        }))
    })
    .await;

    match result {
        Ok(Ok(v)) => jsonrpc_result(id, v),
        Ok(Err(e)) => jsonrpc_error(id, -32004, &format!("cron describe failed: {e:#}")),
        Err(e) => jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    }
}

/// `agent.cron.reload` — poke the cron loop's reload channel. Returns
/// immediately; the actual re-parse happens on the next tick boundary.
fn handle_agent_cron_reload(ops: &Arc<AgentOpsHandle>, id: Value) -> Value {
    match &ops.cron {
        Some(handle) => {
            handle.reload();
            jsonrpc_result(id, serde_json::json!({ "queued": true }))
        }
        None => jsonrpc_error(id, -32603, "cron handle not wired"),
    }
}

/// `agent.cron.history` — recent fires across all jobs (or one job
/// when `name` is set). Params:
///   `{ "name"?: "<job-name>", "limit"?: <int, default 20> }`
async fn handle_agent_cron_history(
    ops: &Arc<AgentOpsHandle>,
    id: Value,
    params: &Value,
) -> Value {
    let name = params.get("name").and_then(|v| v.as_str()).map(String::from);
    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as u32;
    let registry_path = ops.registry_path.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let reg = LocalRegistry::open(&registry_path)?;
        let rows = match name {
            Some(n) => reg.cron_history_for_job(&n, limit)?,
            None => reg.cron_history_all(limit)?,
        };
        Ok(serde_json::json!({
            "fires": rows.iter().map(|h| serde_json::json!({
                "job_name": h.job_name,
                "fired_at": h.fired_at,
                "status": h.status,
                "duration_ms": h.duration_ms,
                "error_message": h.error_message,
            })).collect::<Vec<_>>(),
        }))
    })
    .await;

    match result {
        Ok(Ok(v)) => jsonrpc_result(id, v),
        Ok(Err(e)) => jsonrpc_error(id, -32001, &format!("cron history failed: {e:#}")),
        Err(e) => jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    }
}

/// `agent.cron.gc_now` — force a retention sweep right now (in addition
/// to the periodic GC task). Params (all optional, default from cron.toml):
///   `{ "history_days"?: <int>, "max_rows_per_job"?: <int> }`
async fn handle_agent_cron_gc_now(
    ops: &Arc<AgentOpsHandle>,
    id: Value,
    params: &Value,
) -> Value {
    let config_path = match &ops.cron_config_path {
        Some(p) => p.clone(),
        None => return jsonrpc_error(id, -32603, "cron is not wired"),
    };
    let registry_path = ops.registry_path.clone();
    let override_days = params.get("history_days").and_then(|v| v.as_u64()).map(|n| n as u32);
    let override_rows = params
        .get("max_rows_per_job")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let cfg = crate::cron_config::load(&config_path)?;
        let history_days = override_days.unwrap_or(cfg.retention.history_days);
        let max_rows = override_rows.unwrap_or(cfg.retention.max_rows_per_job);
        let reg = LocalRegistry::open(&registry_path)?;
        let deleted = reg.cron_gc(history_days, max_rows)?;
        Ok(serde_json::json!({
            "deleted": deleted,
            "history_days": history_days,
            "max_rows_per_job": max_rows,
        }))
    })
    .await;

    match result {
        Ok(Ok(v)) => jsonrpc_result(id, v),
        Ok(Err(e)) => jsonrpc_error(id, -32001, &format!("cron gc failed: {e:#}")),
        Err(e) => jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    }
}

// ─── agent.supervise.* handlers (Phase 5) ─────────────────────────────────

/// `agent.supervise.list` — every agent with a `systemd_service` set,
/// joined with the live `systemctl is-active` state. Output:
///   { "agents": [ { agent_id, source, systemd_service, supervise_paused,
///                   unit_state: "active"|"inactive"|"failed"|... } ] }
async fn handle_supervise_list(ops: &Arc<AgentOpsHandle>, id: Value) -> Value {
    let registry_path = ops.registry_path.clone();
    let result = crate::unit_health::list_supervised(registry_path).await;
    match result {
        Ok(rows) => {
            let agents: Vec<Value> = rows
                .into_iter()
                .map(|(ctx, state)| {
                    serde_json::json!({
                        "agent_id": ctx.agent_id,
                        "source": ctx.source,
                        "systemd_service": ctx.systemd_service,
                        "supervise_paused": ctx.supervise_paused,
                        "unit_state": state,
                    })
                })
                .collect();
            jsonrpc_result(id, serde_json::json!({ "agents": agents }))
        }
        Err(e) => jsonrpc_error(id, -32001, &format!("supervise list failed: {e:#}")),
    }
}

/// `agent.supervise.status` — one agent's launch context + recent restart
/// history. Params: `{ "agent_id": "<id>", "limit"?: <int, default 5> }`.
async fn handle_supervise_status(
    ops: &Arc<AgentOpsHandle>,
    id: Value,
    params: &Value,
) -> Value {
    let agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return jsonrpc_error(id, -32602, "missing param: agent_id"),
    };
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as u32;
    let registry_path = ops.registry_path.clone();
    let lookup_id = agent_id.clone();

    let info = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<Value>> {
        let reg = LocalRegistry::open(&registry_path)?;
        let all = reg.list_all_launch_contexts()?;
        let Some(ctx) = all.into_iter().find(|c| c.agent_id == lookup_id) else {
            return Ok(None);
        };
        let history = reg.unit_restart_history(&ctx.source, &ctx.agent_id, limit)?;
        Ok(Some(serde_json::json!({
            "agent_id": ctx.agent_id,
            "source": ctx.source,
            "systemd_service": ctx.systemd_service,
            "supervise_paused": ctx.supervise_paused,
            "history": history.iter().map(|h| serde_json::json!({
                "triggered_at": h.triggered_at,
                "reason": h.reason,
                "result": h.result,
                "systemctl_exit": h.systemctl_exit,
                "notes": h.notes,
            })).collect::<Vec<_>>(),
        })))
    })
    .await;

    match info {
        Ok(Ok(Some(mut v))) => {
            // Add the live unit state (calls systemctl).
            if let Some(svc) = v
                .get("systemd_service")
                .and_then(|s| s.as_str())
                .map(String::from)
            {
                let unit_state = tokio::process::Command::new("systemctl")
                    .args(["--user", "is-active", &svc])
                    .output()
                    .await
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|_| "unreachable".to_string());
                if let Value::Object(m) = &mut v {
                    m.insert("unit_state".into(), Value::String(unit_state));
                }
            }
            jsonrpc_result(id, v)
        }
        Ok(Ok(None)) => jsonrpc_error(id, -32004, &format!("agent {agent_id:?} not found")),
        Ok(Err(e)) => jsonrpc_error(id, -32001, &format!("supervise status failed: {e:#}")),
        Err(e) => jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    }
}

/// `agent.supervise.restart` — manual restart trigger. Params:
///   `{ "agent_id": "<id>" }`. Logged as reason="manual".
async fn handle_supervise_restart(
    ops: &Arc<AgentOpsHandle>,
    id: Value,
    params: &Value,
) -> Value {
    let agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return jsonrpc_error(id, -32602, "missing param: agent_id"),
    };
    let registry_path = ops.registry_path.clone();
    let events_tx = ops.supervisor_events.clone();
    let lookup_id = agent_id.clone();

    // Look up + restart on a blocking thread (subprocess + SQLite).
    let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<(String, String, String, i32)> {
        let reg = LocalRegistry::open(&registry_path)?;
        let ctx = reg
            .list_all_launch_contexts()?
            .into_iter()
            .find(|c| c.agent_id == lookup_id)
            .ok_or_else(|| anyhow::anyhow!("agent {lookup_id:?} not found"))?;
        let service = ctx
            .systemd_service
            .clone()
            .ok_or_else(|| anyhow::anyhow!("agent {lookup_id:?} has no systemd_service"))?;
        let exit = std::process::Command::new("systemctl")
            .args(["--user", "restart", &service])
            .output()?;
        let code = exit.status.code().unwrap_or(-1);
        let result = if code == 0 { "started" } else { "failed" };
        reg.log_unit_restart(
            &ctx.agent_id,
            &ctx.source,
            &chrono::Utc::now().to_rfc3339(),
            "manual",
            result,
            Some(code as i64),
            None,
        )?;
        Ok((ctx.agent_id, ctx.source, service, code))
    })
    .await;

    match outcome {
        Ok(Ok((agent_id, source, systemd_service, exit))) => {
            // Best-effort event publish.
            if let Some(tx) = events_tx {
                let _ = tx.send(mcd_core::types::SupervisorEvent::UnitRestarted {
                    agent_id: agent_id.clone(),
                    source: source.clone(),
                    systemd_service: systemd_service.clone(),
                    reason: "manual".into(),
                    result: if exit == 0 { "started".into() } else { "failed".into() },
                    exit_code: Some(exit as i64),
                    at: chrono::Utc::now().to_rfc3339(),
                });
            }
            jsonrpc_result(
                id,
                serde_json::json!({
                    "agent_id": agent_id,
                    "source": source,
                    "exit_code": exit,
                    "result": if exit == 0 { "started" } else { "failed" },
                }),
            )
        }
        Ok(Err(e)) => jsonrpc_error(id, -32004, &format!("restart failed: {e:#}")),
        Err(e) => jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    }
}

/// `agent.supervise.pause` (paused=true) or `.resume` (paused=false).
/// Params: `{ "agent_id"?: "<id>", "all"?: true }`. Exactly one must be set.
async fn handle_supervise_pause_or_resume(
    ops: &Arc<AgentOpsHandle>,
    id: Value,
    params: &Value,
    paused: bool,
) -> Value {
    let all = params.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let agent_id = params.get("agent_id").and_then(|v| v.as_str()).map(String::from);
    if !all && agent_id.is_none() {
        return jsonrpc_error(id, -32602, "must specify agent_id or all=true");
    }
    if all && agent_id.is_some() {
        return jsonrpc_error(id, -32602, "specify either agent_id or all=true, not both");
    }

    let registry_path = ops.registry_path.clone();
    let events_tx = ops.supervisor_events.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<(String, String)>> {
        let reg = LocalRegistry::open(&registry_path)?;
        let mut affected = Vec::new();
        if all {
            for ctx in reg.list_all_launch_contexts()? {
                if ctx.systemd_service.is_some() && reg.set_supervise_paused(&ctx.source, &ctx.agent_id, paused)? {
                    affected.push((ctx.source, ctx.agent_id));
                }
            }
        } else if let Some(name) = agent_id {
            for ctx in reg.list_all_launch_contexts()?.into_iter().filter(|c| c.agent_id == name) {
                if reg.set_supervise_paused(&ctx.source, &ctx.agent_id, paused)? {
                    affected.push((ctx.source, ctx.agent_id));
                }
            }
        }
        Ok(affected)
    })
    .await;

    match result {
        Ok(Ok(affected)) => {
            // Publish events for each.
            if let Some(tx) = events_tx {
                for (source, agent_id) in &affected {
                    let ev = if paused {
                        mcd_core::types::SupervisorEvent::SupervisePaused {
                            agent_id: agent_id.clone(),
                            source: source.clone(),
                            at: chrono::Utc::now().to_rfc3339(),
                        }
                    } else {
                        mcd_core::types::SupervisorEvent::SuperviseResumed {
                            agent_id: agent_id.clone(),
                            source: source.clone(),
                            at: chrono::Utc::now().to_rfc3339(),
                        }
                    };
                    let _ = tx.send(ev);
                }
            }
            jsonrpc_result(
                id,
                serde_json::json!({
                    "paused": paused,
                    "affected": affected.iter().map(|(s, a)| serde_json::json!({ "source": s, "agent_id": a })).collect::<Vec<_>>(),
                    "count": affected.len(),
                }),
            )
        }
        Ok(Err(e)) => jsonrpc_error(id, -32001, &format!("pause/resume failed: {e:#}")),
        Err(e) => jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    }
}

/// `agent.supervise.history` — recent restart events across all agents
/// (or filtered to one). Params:
///   `{ "agent_id"?: "<id>", "limit"?: <int, default 20> }`
async fn handle_supervise_history(
    ops: &Arc<AgentOpsHandle>,
    id: Value,
    params: &Value,
) -> Value {
    let agent_id = params.get("agent_id").and_then(|v| v.as_str()).map(String::from);
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
    let registry_path = ops.registry_path.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let reg = LocalRegistry::open(&registry_path)?;
        let rows = if let Some(name) = agent_id {
            // Need to resolve source first.
            let all = reg.list_all_launch_contexts()?;
            let Some(ctx) = all.into_iter().find(|c| c.agent_id == name) else {
                return Ok(serde_json::json!({ "restarts": [] }));
            };
            reg.unit_restart_history(&ctx.source, &ctx.agent_id, limit)?
        } else {
            reg.unit_restart_history_all(limit)?
        };
        Ok(serde_json::json!({
            "restarts": rows.iter().map(|r| serde_json::json!({
                "agent_id": r.agent_id,
                "source": r.source,
                "triggered_at": r.triggered_at,
                "reason": r.reason,
                "result": r.result,
                "systemctl_exit": r.systemctl_exit,
                "notes": r.notes,
            })).collect::<Vec<_>>(),
        }))
    })
    .await;

    match result {
        Ok(Ok(v)) => jsonrpc_result(id, v),
        Ok(Err(e)) => jsonrpc_error(id, -32001, &format!("supervise history failed: {e:#}")),
        Err(e) => jsonrpc_error(id, -32603, &format!("blocking task panicked: {e}")),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn jsonrpc_result(id: Value, result: Value) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: Value, code: i32, message: &str) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        },
    })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mcd_packs::{PolicyBundle, PackRegistry};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    fn make_gateway_on(socket_path: PathBuf, tcp_port: u16) -> MgmtGateway {
        let registry = Arc::new(PackRegistry::load_builtin().expect("builtin registry"));
        let dispatcher = Arc::new(CapabilityDispatcher::new(
            Arc::clone(&registry),
            PolicyBundle::allow_all(),
            None,
        ));
        MgmtGateway {
            dispatcher,
            registry,
            mc_token: None,
            socket_path,
            tcp_port,
            agent_ops: None,
        }
    }

    fn make_gateway_with_token(socket_path: PathBuf, tcp_port: u16, token: &str) -> MgmtGateway {
        let registry = Arc::new(PackRegistry::load_builtin().expect("builtin registry"));
        let dispatcher = Arc::new(CapabilityDispatcher::new(
            Arc::clone(&registry),
            PolicyBundle::allow_all(),
            None,
        ));
        MgmtGateway {
            dispatcher,
            registry,
            mc_token: Some(token.to_string()),
            socket_path,
            tcp_port,
            agent_ops: None,
        }
    }

    /// Find a free TCP port by binding to port 0 and reading the assigned port.
    fn free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port 0");
        listener.local_addr().expect("local addr").port()
    }

    #[tokio::test]
    async fn unix_socket_handles_capabilities_list() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sock = tmp.path().join("mgmt-test.sock");
        let gw = make_gateway_on(sock.clone(), free_port());
        let sock_for_client = sock.clone();

        // Start gateway in background.
        tokio::spawn(async move {
            let _ = gw.run().await;
        });

        // Give the socket a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Connect to Unix socket.
        let mut stream = tokio::net::UnixStream::connect(&sock_for_client)
            .await
            .expect("connect to unix socket");

        // Send capabilities.list request.
        let request = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"capabilities.list\",\"params\":{}}\n";
        stream.write_all(request.as_bytes()).await.expect("write request");

        // Read response line (may be large — use line-based reader).
        let mut reader = BufReader::new(stream);
        let mut response_str = String::new();
        reader.read_line(&mut response_str).await.expect("read response line");

        // Must be valid JSON containing "result".
        let response: Value = serde_json::from_str(response_str.trim())
            .expect("valid JSON response");
        assert!(
            response.get("result").is_some(),
            "response should contain 'result', got: {response_str}"
        );
        assert_eq!(response.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
    }

    #[tokio::test]
    async fn tcp_auth_rejects_bad_token() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sock = tmp.path().join("mgmt-auth-test.sock");
        let port = free_port();
        let gw = make_gateway_with_token(sock, port, "secret");

        tokio::spawn(async move {
            let _ = gw.run().await;
        });

        // Give TCP listener a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Connect and send bad token.
        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .expect("connect tcp");
        stream.write_all(b"AUTH badtoken\n").await.expect("write auth");

        // Read response — must be "ERR unauthorized\n".
        let mut buf = vec![0u8; 64];
        let n = stream.read(&mut buf).await.expect("read");
        let response = std::str::from_utf8(&buf[..n]).expect("utf8");
        assert_eq!(
            response.trim(),
            "ERR unauthorized",
            "expected ERR unauthorized, got: {response:?}"
        );

        // Connection should be closed — next read returns 0 bytes.
        let n2 = stream.read(&mut buf).await.unwrap_or(0);
        assert_eq!(n2, 0, "connection should be closed after bad auth");
    }

    #[tokio::test]
    async fn tcp_auth_accepts_good_token() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sock = tmp.path().join("mgmt-auth-ok-test.sock");
        let port = free_port();
        let gw = make_gateway_with_token(sock, port, "correcttoken");

        tokio::spawn(async move {
            let _ = gw.run().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .expect("connect tcp");
        stream.write_all(b"AUTH correcttoken\n").await.expect("write auth");

        let mut buf = vec![0u8; 64];
        let n = stream.read(&mut buf).await.expect("read ok");
        let response = std::str::from_utf8(&buf[..n]).expect("utf8");
        assert_eq!(response.trim(), "OK", "expected OK, got: {response:?}");
    }

    // ─── agent.local.* + agent.describe_local (Phase 3) ─────────────────

    /// Drive a single JSON-RPC request through `dispatch_jsonrpc` without
    /// spinning up the full gateway. Validates handler logic in isolation.
    async fn dispatch(raw: &str, agent_ops: Option<Arc<AgentOpsHandle>>) -> Value {
        let registry = Arc::new(PackRegistry::load_builtin().expect("builtin registry"));
        let dispatcher = Arc::new(CapabilityDispatcher::new(
            Arc::clone(&registry),
            PolicyBundle::allow_all(),
            None,
        ));
        dispatch_jsonrpc(&dispatcher, &registry, agent_ops.as_ref(), raw).await
    }

    #[tokio::test]
    async fn agent_local_signal_without_ops_returns_unwired() {
        let resp = dispatch(
            r#"{"jsonrpc":"2.0","id":1,"method":"agent.local.signal","params":{"agent_id":"x","kind":"user_input","text":"hi"}}"#,
            None,
        )
        .await;
        let code = resp.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_i64());
        assert_eq!(code, Some(-32603), "resp: {resp}");
    }

    #[tokio::test]
    async fn agent_local_list_without_ops_returns_unwired() {
        let resp = dispatch(
            r#"{"jsonrpc":"2.0","id":2,"method":"agent.local.list","params":{}}"#,
            None,
        )
        .await;
        assert_eq!(
            resp.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_i64()),
            Some(-32603)
        );
    }

    #[tokio::test]
    async fn agent_describe_local_without_ops_returns_unwired() {
        let resp = dispatch(
            r#"{"jsonrpc":"2.0","id":3,"method":"agent.describe_local","params":{"agent_id":"x"}}"#,
            None,
        )
        .await;
        assert_eq!(
            resp.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_i64()),
            Some(-32603)
        );
    }

    /// With agent_ops wired but an empty registry, `agent.local.list`
    /// succeeds and returns an empty array — exercising the registry-open
    /// + `spawn_blocking` path.
    #[tokio::test]
    async fn agent_local_list_empty_registry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let registry_path = tmp.path().join("registry.db");
        // Open once to create + migrate.
        let _ = LocalRegistry::open(&registry_path).expect("open registry");

        let ops = Arc::new(AgentOpsHandle {
            supervisor: Arc::new(Supervisor::new(
                tmp.path().join("work").to_path_buf(),
                "http://localhost:8008".into(),
                String::new(),
            )),
            runtime_map: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            registry_path,
            cron: None,
            cron_config_path: None,
            supervisor_events: None,
        });

        let resp = dispatch(
            r#"{"jsonrpc":"2.0","id":4,"method":"agent.local.list","params":{}}"#,
            Some(ops),
        )
        .await;

        let agents = resp
            .get("result")
            .and_then(|r| r.get("agents"))
            .and_then(|a| a.as_array())
            .expect("result.agents array");
        assert!(agents.is_empty(), "expected empty agents list, got: {resp}");
    }

    #[tokio::test]
    async fn agent_describe_local_missing_returns_not_found() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let registry_path = tmp.path().join("registry.db");
        let _ = LocalRegistry::open(&registry_path).expect("open registry");

        let ops = Arc::new(AgentOpsHandle {
            supervisor: Arc::new(Supervisor::new(
                tmp.path().join("work").to_path_buf(),
                "http://localhost:8008".into(),
                String::new(),
            )),
            runtime_map: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            registry_path,
            cron: None,
            cron_config_path: None,
            supervisor_events: None,
        });

        let resp = dispatch(
            r#"{"jsonrpc":"2.0","id":5,"method":"agent.describe_local","params":{"agent_id":"no-such"}}"#,
            Some(ops),
        )
        .await;

        let found = resp.get("result").and_then(|r| r.get("found")).and_then(|f| f.as_bool());
        assert_eq!(found, Some(false), "resp: {resp}");
    }

    #[tokio::test]
    async fn agent_local_signal_missing_agent_returns_not_supervised() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let registry_path = tmp.path().join("registry.db");
        let _ = LocalRegistry::open(&registry_path).expect("open registry");

        let ops = Arc::new(AgentOpsHandle {
            supervisor: Arc::new(Supervisor::new(
                tmp.path().join("work").to_path_buf(),
                "http://localhost:8008".into(),
                String::new(),
            )),
            runtime_map: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            registry_path,
            cron: None,
            cron_config_path: None,
            supervisor_events: None,
        });

        let resp = dispatch(
            r#"{"jsonrpc":"2.0","id":6,"method":"agent.local.signal","params":{"agent_id":"no-such","kind":"user_input","text":"hi"}}"#,
            Some(ops),
        )
        .await;

        assert_eq!(
            resp.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_i64()),
            Some(-32004),
            "resp: {resp}"
        );
    }

    #[tokio::test]
    async fn agent_local_signal_bad_kind_returns_invalid_params() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let registry_path = tmp.path().join("registry.db");
        let _ = LocalRegistry::open(&registry_path).expect("open registry");

        let ops = Arc::new(AgentOpsHandle {
            supervisor: Arc::new(Supervisor::new(
                tmp.path().join("work").to_path_buf(),
                "http://localhost:8008".into(),
                String::new(),
            )),
            runtime_map: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            registry_path,
            cron: None,
            cron_config_path: None,
            supervisor_events: None,
        });

        let resp = dispatch(
            r#"{"jsonrpc":"2.0","id":7,"method":"agent.local.signal","params":{"agent_id":"x","kind":"bogus"}}"#,
            Some(ops),
        )
        .await;

        let code = resp
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_i64());
        assert_eq!(code, Some(-32602), "resp: {resp}");
    }

    // ─── events.subscribe streaming ─────────────────────────────────────────────

    #[tokio::test]
    async fn events_subscribe_streams_event_after_ack() {
        use mcd_core::types::SupervisorEvent;

        // In-memory broadcast channel — same shape as daemon.rs:392.
        let (tx, _rx_initial) = tokio::sync::broadcast::channel::<SupervisorEvent>(16);

        // DuplexStream gives us an in-memory bidirectional pipe.
        let (server_side, mut client_side) = tokio::io::duplex(4096);
        let (server_read, server_write) = tokio::io::split(server_side);
        let reader = BufReader::new(server_read);

        // Spawn the streaming handler on the server side.
        let tx_clone = tx.clone();
        let server_task = tokio::spawn(async move {
            // Inline the loop's "is this events.subscribe?" branch — we test
            // the streaming function directly with a known method line.
            let mut writer = server_write;
            let mut reader = reader;
            let mut line = String::new();
            let _ = reader.read_line(&mut line).await;
            let trimmed = line.trim();
            let req: Value = serde_json::from_str(trimmed).expect("valid req");
            assert_eq!(req["method"], "events.subscribe");
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            stream_supervisor_events(Some(&tx_clone), id, &mut writer)
                .await
                .expect("stream ok");
        });

        // Client: send events.subscribe.
        client_side
            .write_all(br#"{"jsonrpc":"2.0","id":42,"method":"events.subscribe","params":{}}
"#)
            .await
            .expect("write subscribe");

        // Read ack frame.
        let mut client_reader = BufReader::new(client_side);
        let mut ack_line = String::new();
        client_reader.read_line(&mut ack_line).await.expect("read ack");
        let ack: Value = serde_json::from_str(ack_line.trim()).expect("ack json");
        assert_eq!(ack["id"], 42);
        assert_eq!(ack["result"]["subscribed"], true);

        // Fire a SupervisorEvent — give the subscriber a moment to register.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        tx.send(SupervisorEvent::UnitRestarted {
            agent_id: "work".to_string(),
            source: "fleet_import".to_string(),
            systemd_service: "merlinlabs.service".to_string(),
            reason: "manual".to_string(),
            result: "started".to_string(),
            exit_code: None,
            at: "2026-05-20T20:30:00Z".to_string(),
        })
        .expect("broadcast send");

        // Read event frame.
        let mut event_line = String::new();
        client_reader.read_line(&mut event_line).await.expect("read event");
        let event: Value = serde_json::from_str(event_line.trim()).expect("event json");
        assert_eq!(event["kind"], "unit_restarted");
        assert_eq!(event["agent_id"], "work");
        assert_eq!(event["result"], "started");

        // Drop the sender → receiver closes → stream task exits cleanly.
        drop(tx);
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server_task).await;
    }

    #[tokio::test]
    async fn events_subscribe_errors_when_no_sender_wired() {
        // None sender → JSON-RPC error frame, no streaming.
        let (server_side, mut client_side) = tokio::io::duplex(4096);
        let (_, mut server_write) = tokio::io::split(server_side);

        let server_task = tokio::spawn(async move {
            stream_supervisor_events(None, Value::from(7), &mut server_write)
                .await
                .expect("stream returns Ok even when erroring");
        });

        let mut client_reader = BufReader::new(&mut client_side);
        let mut err_line = String::new();
        client_reader.read_line(&mut err_line).await.expect("read err");
        let err: Value = serde_json::from_str(err_line.trim()).expect("err json");
        assert_eq!(err["id"], 7);
        assert_eq!(err["error"]["code"], -32603);
        assert!(
            err["error"]["message"].as_str().unwrap_or("").contains("supervisor events not wired"),
            "got: {err}"
        );

        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), server_task).await;
    }
}
