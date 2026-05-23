/// `McDispatch` — routes capability commands to the right backend.
///
/// Route priority (highest to lowest):
///   1. `--host <node>` → Remote
///   2. `--route <mode>` → explicit RouteMode
///   3. `MC_ROUTE` env var
///   4. `~/.mc/config.json` `capability_route` field
///   5. Default: Auto
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum RouteMode {
    Auto,
    Local,
    Remote { host: String, port: u16 },
    Backend,
}

pub struct McDispatch {
    pub mode: RouteMode,
    mc_token: Option<String>,
}

// ---------------------------------------------------------------------------
// Config file shape (only the fields we need)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct FileConfig {
    capability_route: Option<String>,
    default_host: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse `"hostname"` or `"hostname:port"` into `(host, port)`.
fn parse_host_port(s: &str, default_port: u16) -> (String, u16) {
    if let Some(idx) = s.rfind(':') {
        let maybe_port = &s[idx + 1..];
        if let Ok(p) = maybe_port.parse::<u16>() {
            return (s[..idx].to_string(), p);
        }
    }
    (s.to_string(), default_port)
}

/// Convert a route string (+ optional host hint) to a `RouteMode`.
fn route_mode_from_str(s: &str, host: Option<&str>) -> RouteMode {
    match s.trim().to_lowercase().as_str() {
        "local" => RouteMode::Local,
        "backend" => RouteMode::Backend,
        "remote" => {
            if let Some(h) = host {
                let (hostname, port) = parse_host_port(h, 7731);
                RouteMode::Remote {
                    host: hostname,
                    port,
                }
            } else {
                // Can't be Remote without a host — fall back to Auto.
                RouteMode::Auto
            }
        }
        _ => RouteMode::Auto,
    }
}

/// Load `~/.mc/config.json` if it exists.
fn load_file_config() -> FileConfig {
    let path = crate::config::config_file_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return FileConfig::default(),
    };
    serde_json::from_str::<FileConfig>(&raw).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// McDispatch
// ---------------------------------------------------------------------------

impl McDispatch {
    /// Build from environment + config file with optional CLI overrides.
    pub fn from_env(host: Option<String>, route_override: Option<String>) -> Self {
        let mc_token = crate::config::load_session_token("");

        // 1. --host flag → Remote immediately.
        if let Some(ref h) = host {
            let (hostname, port) = parse_host_port(h, 7731);
            return Self {
                mode: RouteMode::Remote {
                    host: hostname,
                    port,
                },
                mc_token,
            };
        }

        // 2. --route override
        if let Some(ref r) = route_override {
            return Self {
                mode: route_mode_from_str(r, None),
                mc_token,
            };
        }

        // 3. MC_ROUTE env var
        if let Ok(r) = std::env::var("MC_ROUTE") {
            return Self {
                mode: route_mode_from_str(&r, None),
                mc_token,
            };
        }

        // 4. Config file
        let cfg = load_file_config();
        if let Some(ref r) = cfg.capability_route {
            let host_hint = cfg.default_host.as_deref();
            return Self {
                mode: route_mode_from_str(r, host_hint),
                mc_token,
            };
        }

        // 5. Default: Auto
        Self {
            mode: RouteMode::Auto,
            mc_token,
        }
    }

    /// List capabilities from the daemon, optionally filtered by tag.
    pub async fn list_capabilities(
        &self,
        tag: Option<&str>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let params = serde_json::json!({ "tag": tag });

        let effective_mode = match &self.mode {
            RouteMode::Auto => self.resolve_auto(),
            other => other.clone(),
        };

        let result = match &effective_mode {
            RouteMode::Local => {
                let socket_path = std::env::var("MC_MESH_SOCKET")
                    .context("MC_MESH_SOCKET not set for Local route")?;
                send_jsonrpc_unix(&socket_path, "capabilities.list", params).await?
            }
            RouteMode::Remote { host, port } => {
                send_jsonrpc_tcp(host, *port, self.mc_token.as_deref(), "capabilities.list", params).await?
            }
            RouteMode::Backend | RouteMode::Auto => {
                anyhow::bail!("daemon not reachable: no socket or remote host configured")
            }
        };

        // Result should be an array of capability objects.
        result
            .as_array()
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| anyhow::bail!("capabilities.list returned non-array"))
    }

    /// Dispatch a capability call and return the JSON result.
    pub async fn dispatch(
        &self,
        full_name: &str,
        args: Value,
        dry_run: bool,
        timeout_secs: Option<u32>,
        domain_id: Option<String>,
        agent_id: Option<String>,
    ) -> Result<Value> {
        let params = serde_json::json!({
            "full_name": full_name,
            "args": args,
            "dry_run": dry_run,
            "timeout_secs": timeout_secs,
            "domain_id": domain_id,
            "agent_id": agent_id,
        });

        let effective_mode = match &self.mode {
            RouteMode::Auto => self.resolve_auto(),
            other => other.clone(),
        };

        match &effective_mode {
            RouteMode::Local => {
                let socket_path = std::env::var("MC_MESH_SOCKET")
                    .context("MC_MESH_SOCKET not set for Local route")?;
                send_jsonrpc_unix(&socket_path, "dispatch", params).await
            }
            RouteMode::Remote { host, port } => {
                send_jsonrpc_tcp(host, *port, self.mc_token.as_deref(), "dispatch", params).await
            }
            RouteMode::Backend => {
                anyhow::bail!(
                    "backend route not yet implemented; set MC_MESH_SOCKET or use --host"
                )
            }
            RouteMode::Auto => {
                // Auto resolution failed to narrow down — default to backend stub.
                anyhow::bail!(
                    "backend route not yet implemented; set MC_MESH_SOCKET or use --host"
                )
            }
        }
    }

    /// Resolve Auto: check MC_MESH_SOCKET → Local; otherwise Backend.
    fn resolve_auto(&self) -> RouteMode {
        if let Ok(sock) = std::env::var("MC_MESH_SOCKET") {
            if std::path::Path::new(&sock).exists() {
                return RouteMode::Local;
            }
        }
        RouteMode::Backend
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC transport helpers
// ---------------------------------------------------------------------------

#[cfg(unix)]
async fn send_jsonrpc_unix(
    socket_path: &str,
    method: &str,
    params: Value,
) -> Result<Value> {
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connect to unix socket {socket_path}"))?;

    send_jsonrpc_over_stream(stream, method, params).await
}

#[cfg(not(unix))]
async fn send_jsonrpc_unix(
    _socket_path: &str,
    _method: &str,
    _params: Value,
) -> Result<Value> {
    anyhow::bail!("Unix socket routing is not supported on Windows")
}

async fn send_jsonrpc_tcp(
    host: &str,
    port: u16,
    token: Option<&str>,
    method: &str,
    params: Value,
) -> Result<Value> {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("connect to {host}:{port}"))?;

    // AUTH handshake: send "AUTH <token>\n", expect "OK\n".
    if let Some(tok) = token {
        stream
            .write_all(format!("AUTH {tok}\n").as_bytes())
            .await
            .context("send AUTH")?;
    } else {
        stream
            .write_all(b"AUTH \n")
            .await
            .context("send AUTH (empty)")?;
    }

    // Read the OK line.
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut resp_line = String::new();
    reader
        .read_line(&mut resp_line)
        .await
        .context("read AUTH response")?;
    if !resp_line.trim().eq_ignore_ascii_case("ok") {
        anyhow::bail!("TCP AUTH rejected: {}", resp_line.trim());
    }

    // Reassemble a combined stream using the reunite helper.
    let stream = reader
        .into_inner()
        .reunite(write_half)
        .map_err(|_| anyhow::anyhow!("failed to reunite TCP stream halves"))?;

    send_jsonrpc_over_stream(stream, method, params).await
}

/// Send a JSON-RPC 2.0 request over any newline-delimited async stream and
/// return the `result` field (or error if the response contains `error`).
async fn send_jsonrpc_over_stream<S>(stream: S, method: &str, params: Value) -> Result<Value>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (read_half, mut write_half) = tokio::io::split(stream);

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let mut request_bytes = serde_json::to_vec(&request).context("serialize JSON-RPC request")?;
    request_bytes.push(b'\n');

    write_half
        .write_all(&request_bytes)
        .await
        .context("send JSON-RPC request")?;
    write_half.flush().await.context("flush JSON-RPC request")?;

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("read JSON-RPC response")?;

    let response: Value =
        serde_json::from_str(line.trim()).context("parse JSON-RPC response")?;

    if let Some(err) = response.get("error") {
        anyhow::bail!("JSON-RPC error: {err}");
    }

    response
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("JSON-RPC response missing 'result' field"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Guard: run env-sensitive tests serially so they don't clobber each other.
    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn host_flag_forces_remote() {
        let dispatch = McDispatch::from_env(Some("optiplex".to_string()), None);
        assert!(
            matches!(dispatch.mode, RouteMode::Remote { .. }),
            "expected Remote, got {:?}",
            dispatch.mode
        );
    }

    #[test]
    fn host_with_port_parsed() {
        let dispatch = McDispatch::from_env(Some("optiplex:7732".to_string()), None);
        match &dispatch.mode {
            RouteMode::Remote { host, port } => {
                assert_eq!(host, "optiplex");
                assert_eq!(*port, 7732);
            }
            other => panic!("expected Remote, got {other:?}"),
        }
    }

    #[test]
    fn auto_resolves_to_local_when_socket_set() {
        let _guard = ENV_LOCK.lock().unwrap();

        // Create a temp file to act as the socket placeholder (just needs to exist).
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let sock_path = tmp.path().to_str().unwrap().to_string();

        // Capture and clear any existing env so test is deterministic.
        let prev_sock = std::env::var("MC_MESH_SOCKET").ok();
        let prev_route = std::env::var("MC_ROUTE").ok();
        // SAFETY: test is serialised behind ENV_LOCK; no other threads touch these vars.
        unsafe {
            std::env::remove_var("MC_ROUTE");
            std::env::set_var("MC_MESH_SOCKET", &sock_path);
        }

        let dispatch = McDispatch::from_env(None, None);
        // In Auto mode, resolve_auto() should pick Local because the socket file exists.
        let resolved = dispatch.resolve_auto();
        assert_eq!(resolved, RouteMode::Local, "expected Local route");

        // Restore env.
        unsafe {
            match prev_sock {
                Some(v) => std::env::set_var("MC_MESH_SOCKET", v),
                None => std::env::remove_var("MC_MESH_SOCKET"),
            }
            match prev_route {
                Some(v) => std::env::set_var("MC_ROUTE", v),
                None => std::env::remove_var("MC_ROUTE"),
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn send_jsonrpc_unix_round_trip() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixListener;

        // Create a unique socket path in the temp dir.
        let dir = tempfile::tempdir().expect("tempdir");
        let sock_path = dir.path().join("test.sock");
        let sock_str = sock_path.to_str().unwrap().to_string();

        let listener = UnixListener::bind(&sock_path).expect("bind unix socket");

        // Background task: accept one connection, echo back a fixed JSON-RPC result.
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                stream.read_exact(&mut byte).await.expect("read byte");
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            // Verify it's valid JSON-RPC.
            let req: Value = serde_json::from_slice(&buf).expect("parse request");
            assert_eq!(req["method"].as_str().unwrap(), "dispatch");

            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": { "status": "ok", "output": "hello" }
            });
            let mut resp_bytes = serde_json::to_vec(&response).unwrap();
            resp_bytes.push(b'\n');
            stream.write_all(&resp_bytes).await.expect("write response");
        });

        // Give the listener task a moment to be ready.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let params = serde_json::json!({ "full_name": "test.ping", "args": {} });
        let result = send_jsonrpc_unix(&sock_str, "dispatch", params)
            .await
            .expect("round trip");

        assert_eq!(result["status"].as_str().unwrap(), "ok");
        assert_eq!(result["output"].as_str().unwrap(), "hello");
    }
}
