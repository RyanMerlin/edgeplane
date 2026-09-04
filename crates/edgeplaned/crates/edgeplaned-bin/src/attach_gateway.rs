/// Local unix socket gateway for `edgeplane daemon attach`.
///
/// The daemon binds a unix socket at `~/.edgeplane/run/edgeplaned.sock`.
/// The `edgeplane` CLI connects, sends a single line with the target agent ID,
/// receives `OK\n` (or `ERR <reason>\n`), then I/O becomes raw PTY proxy:
///
///   client → socket → PTY master input
///   PTY master output → socket → client
///
/// For persistent-mode agents, attach connects to the live session via the
/// `AttachRegistry` (multiple viewers share one PTY). For task-mode agents
/// or when no live session is registered, falls back to spawning a fresh
/// `runtime.attach_pty()` — the original behavior.
///
/// This keeps the attachment path entirely local — no backend round-trip.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use edgeplaned_core::agent_runtime::DynAgentRuntime;
use edgeplaned_core::paths;
#[cfg(unix)]
use edgeplaned_core::types::AgentHandle;
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::Mutex;

#[cfg(unix)]
use crate::attach_registry::AttachRegistry;

/// Return the path to the local control socket.
pub fn socket_path() -> PathBuf {
    paths::attach_socket_path()
}

/// Shared map from agent_id → runtime, built by the daemon.
pub type RuntimeMap = Arc<Mutex<HashMap<String, Arc<DynAgentRuntime>>>>;

/// Start the attach gateway.  Runs until the process is killed.
#[cfg(unix)]
pub async fn run(runtimes: RuntimeMap, registry: Arc<AttachRegistry>) -> Result<()> {
    let path = socket_path();
    // Create parent dir if needed.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Remove stale socket from a previous run.
    let _ = std::fs::remove_file(&path);

    let listener = UnixListener::bind(&path)
        .map_err(|e| anyhow::anyhow!("attach gateway bind {}: {e}", path.display()))?;

    tracing::info!("attach gateway listening on {}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let rt_map = Arc::clone(&runtimes);
                let reg = Arc::clone(&registry);
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, rt_map, reg).await {
                        tracing::debug!("attach session ended: {e}");
                    }
                });
            }
            Err(e) => {
                tracing::warn!("attach gateway accept error: {e}");
            }
        }
    }
}

#[cfg(not(unix))]
pub async fn run(
    _runtimes: RuntimeMap,
    _registry: Arc<crate::attach_registry::AttachRegistry>,
) -> Result<()> {
    tracing::warn!("attach gateway is only supported on Unix-like hosts");
    futures::future::pending::<()>().await;
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(unix)]
async fn handle_connection(
    stream: tokio::net::UnixStream,
    runtimes: RuntimeMap,
    registry: Arc<AttachRegistry>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    // First line: <agent-id>\n
    let mut agent_id = String::new();
    reader.read_line(&mut agent_id).await?;
    let agent_id = agent_id.trim().to_string();

    if agent_id.is_empty() {
        write_half.write_all(b"ERR empty agent id\n").await?;
        return Ok(());
    }

    // Persistent-session fast path: a session supervisor already owns a live
    // PTY for this agent. Subscribe to its broadcast and route input through
    // the registered stdin sender. ACP-shaped endpoints don't speak the
    // byte-stream Unix socket protocol — surface a clean error instead of
    // wiring half a channel.
    if let Some(endpoints) = registry.get(&agent_id).await {
        let pty = match endpoints {
            crate::attach_registry::AttachEndpoints::Pty(p) => p,
            crate::attach_registry::AttachEndpoints::Acp(_) => {
                write_half
                    .write_all(
                        format!(
                            "ERR agent {agent_id} is an ACP session; \
                             byte-stream attach not supported on this transport\n"
                        )
                        .as_bytes(),
                    )
                    .await?;
                return Ok(());
            }
        };
        write_half.write_all(b"OK\n").await?;
        tracing::info!("attach session started for persistent agent {agent_id}");

        let mut stdout_rx = pty.stdout_broadcast.subscribe();
        let stdin_tx = pty.stdin_tx.clone();
        let agent_id_for_log = agent_id.clone();

        tokio::spawn(async move {
            loop {
                match stdout_rx.recv().await {
                    Ok(bytes) => {
                        if write_half.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("attach viewer for {agent_id_for_log} lagged {n} chunks");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let mut read_raw = reader.into_inner();
        let mut buf = vec![0u8; 4096];
        loop {
            match read_raw.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if stdin_tx.send(buf[..n].to_vec()).await.is_err() {
                        break;
                    }
                }
            }
        }

        tracing::info!("attach session ended for persistent agent {agent_id}");
        return Ok(());
    }

    // Fallback: spawn a fresh PTY via the runtime. Used for task-mode agents
    // and as a debug aid when no live session is registered.
    let runtime = {
        let map = runtimes.lock().await;
        map.get(&agent_id).cloned()
    };
    let Some(runtime) = runtime else {
        write_half
            .write_all(format!("ERR agent {agent_id} not found\n").as_bytes())
            .await?;
        return Ok(());
    };

    let handle = AgentHandle {
        agent_id: agent_id.clone(),
        runtime_kind: runtime.kind(),
        pid: 0,
    };
    let session = match runtime.attach_pty(&handle).await {
        Ok(s) => s,
        Err(e) => {
            write_half
                .write_all(format!("ERR {e}\n").as_bytes())
                .await?;
            return Ok(());
        }
    };

    write_half.write_all(b"OK\n").await?;
    tracing::info!("attach session started for agent {agent_id} (fresh PTY)");

    let mut pty_output = session.output;
    let pty_input = session.input;

    tokio::spawn(async move {
        while let Some(bytes) = pty_output.recv().await {
            if write_half.write_all(&bytes).await.is_err() {
                break;
            }
        }
    });

    let mut read_raw = reader.into_inner();
    let mut buf = vec![0u8; 4096];
    loop {
        match read_raw.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if pty_input.send(buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
        }
    }

    tracing::info!("attach session ended for agent {agent_id}");
    Ok(())
}
