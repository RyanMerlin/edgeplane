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
use mcd_packs::PackRegistry;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ─── MgmtGateway ─────────────────────────────────────────────────────────────

pub struct MgmtGateway {
    dispatcher: Arc<CapabilityDispatcher>,
    registry: Arc<PackRegistry>,
    mc_token: Option<String>,
    socket_path: PathBuf,
    tcp_port: u16,
}

impl MgmtGateway {
    pub fn new(dispatcher: Arc<CapabilityDispatcher>, registry: Arc<PackRegistry>) -> Self {
        let mc_token = std::env::var("MC_TOKEN").ok().filter(|s| !s.is_empty());
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
        }
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
        handle_jsonrpc_loop(&self.dispatcher, &self.registry, reader, write_half).await
    }

    /// Handle a Unix socket connection — always authenticated.
    async fn handle_connection<S>(&self, stream: S) -> Result<()>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        let reader = BufReader::new(read_half);
        handle_jsonrpc_loop(&self.dispatcher, &self.registry, reader, write_half).await
    }
}

// ─── JSON-RPC loop ────────────────────────────────────────────────────────────

async fn handle_jsonrpc_loop<R, W>(
    dispatcher: &CapabilityDispatcher,
    registry: &PackRegistry,
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

        let response = dispatch_jsonrpc(dispatcher, registry, trimmed).await;
        let mut response_bytes = serde_json::to_vec(&response)
            .unwrap_or_else(|_| br#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"serialization error"}}"#.to_vec());
        response_bytes.push(b'\n');
        writer.write_all(&response_bytes).await?;
    }
    Ok(())
}

async fn dispatch_jsonrpc(
    dispatcher: &CapabilityDispatcher,
    registry: &PackRegistry,
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
}
