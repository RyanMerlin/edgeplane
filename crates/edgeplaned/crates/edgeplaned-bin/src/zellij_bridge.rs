/// PTY bridge for ZellijHosted agents.
///
/// Spawns `zellij attach <session_name>` as a PTY child and registers
/// `PtyAttachEndpoints` in the `AttachRegistry`, enabling remote terminal
/// viewing through the existing `attach_ws` → `pump_pty` pipeline.
///
/// The bridge does NOT own the Zellij session — systemd services manage the
/// session lifecycle. This module only provides a PTY view into it.
///
/// Modeled after `session_supervisor.rs` — restart-on-exit with exponential
/// backoff, signal rendering to PTY stdin, stdout broadcast fan-out.
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use edgeplaned_core::types::AgentSignal;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::{broadcast, mpsc};

use crate::attach_registry::{AttachEndpoints, AttachRegistry, PtyAttachEndpoints};

const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
const STABLE_THRESHOLD: Duration = Duration::from_secs(30);
const STDOUT_BROADCAST_CAPACITY: usize = 1024;
const DEFAULT_ROWS: u16 = 50;
const DEFAULT_COLS: u16 = 220;

pub async fn run_for_agent(
    agent_id: String,
    zellij_session: String,
    registry: Arc<AttachRegistry>,
) {
    let mut backoff = BACKOFF_MIN;

    loop {
        let started = Instant::now();
        match run_one_bridge(&agent_id, &zellij_session, &registry).await {
            Ok(()) => {
                tracing::info!(
                    "Zellij bridge for agent {agent_id} (session {zellij_session}) \
                     exited cleanly after {:?}",
                    started.elapsed()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Zellij bridge for agent {agent_id} (session {zellij_session}) \
                     failed after {:?}: {e:#}",
                    started.elapsed()
                );
            }
        }

        if started.elapsed() >= STABLE_THRESHOLD {
            backoff = BACKOFF_MIN;
        }

        tracing::info!("Restarting Zellij bridge for {agent_id} in {backoff:?}");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

async fn run_one_bridge(
    agent_id: &str,
    zellij_session: &str,
    registry: &Arc<AttachRegistry>,
) -> Result<()> {
    if !session_is_alive(zellij_session) {
        return Err(anyhow!("Zellij session '{zellij_session}' is not running"));
    }

    let zellij_bin = edgeplaned_runtimes::zellij_session::zellij_binary();
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("openpty for zellij bridge")?;

    let mut cmd = CommandBuilder::new(zellij_bin);
    cmd.arg("attach");
    cmd.arg(zellij_session);
    cmd.env_remove("ZELLIJ");
    cmd.env_remove("ZELLIJ_SESSION_NAME");
    cmd.env("TERM", "xterm-256color");

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .context("spawn zellij attach")?;
    drop(pair.slave);

    let mut master_reader = pair.master.try_clone_reader()?;
    let mut master_writer = pair.master.take_writer()?;

    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(256);
    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(256);
    let signal_in_tx = in_tx.clone();
    let (resize_tx, mut resize_rx) = mpsc::channel::<(u16, u16)>(8);
    let (stdout_broadcast, _) = broadcast::channel::<Vec<u8>>(STDOUT_BROADCAST_CAPACITY);
    let (signal_tx, mut signal_rx) = mpsc::channel::<AgentSignal>(64);

    let endpoints = AttachEndpoints::Pty(PtyAttachEndpoints {
        stdin_tx: in_tx,
        stdout_broadcast: stdout_broadcast.clone(),
        resize_tx,
        signal_tx,
    });
    registry.register(agent_id.to_string(), endpoints).await;
    tracing::info!(
        "Zellij bridge registered PTY endpoints for agent {agent_id} \
         (session {zellij_session})"
    );

    // PTY output → channel (blocking thread).
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut buf = [0u8; 4096];
        loop {
            match master_reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Channel → PTY input (blocking thread).
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        loop {
            match in_rx.blocking_recv() {
                None => break,
                Some(bytes) => {
                    if master_writer.write_all(&bytes).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // Resize loop — holds the master alive for the session lifetime.
    let master = pair.master;
    tokio::task::spawn_blocking(move || {
        while let Some((rows, cols)) = resize_rx.blocking_recv() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        drop(master);
    });

    // Pump: fan-out PTY output to broadcast + render signals to PTY stdin.
    let result = pump(
        &mut out_rx,
        &stdout_broadcast,
        &mut signal_rx,
        &signal_in_tx,
    )
    .await;

    registry.unregister(agent_id).await;

    // Reap the `zellij attach` child so it doesn't linger as a <defunct> zombie.
    // `std::process::Child` (which `portable_pty::Child` wraps) does not kill or
    // reap on drop, so without this every bridge exit leaks one zombie and the
    // supervise loop accumulates them. `kill`/`wait` are blocking → run off the
    // async runtime. `kill` first in case `pump` returned for a non-EOF reason
    // (e.g. signal channel closed) and the attach process is still alive.
    let _ = tokio::task::spawn_blocking(move || {
        let _ = child.kill();
        let _ = child.wait();
    })
    .await;

    tracing::info!("Zellij bridge unregistered for agent {agent_id}");
    result
}

async fn pump(
    out_rx: &mut mpsc::Receiver<Vec<u8>>,
    stdout_broadcast: &broadcast::Sender<Vec<u8>>,
    signal_rx: &mut mpsc::Receiver<AgentSignal>,
    stdin_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<()> {
    loop {
        tokio::select! {
            biased;

            chunk = out_rx.recv() => {
                let Some(bytes) = chunk else {
                    return Ok(());
                };
                let _ = stdout_broadcast.send(bytes);
            }

            sig = signal_rx.recv() => {
                let Some(sig) = sig else {
                    return Ok(());
                };
                if let Some(rendered) = render_signal(sig)
                    && stdin_tx.send(rendered.into_bytes()).await.is_err() {
                        return Ok(());
                    }
            }
        }
    }
}

/// Render an `AgentSignal` into PTY-injectable text. Mirrors
/// `session_supervisor::render_signal` — same format so behaviour is
/// identical regardless of which path delivers the signal.
fn render_signal(sig: AgentSignal) -> Option<String> {
    match sig {
        AgentSignal::UserInput { text } => {
            let mut t = text;
            t.push('\r');
            Some(t)
        }
        AgentSignal::PeerMessage {
            from_agent_id,
            channel,
            body,
        } => {
            let body_str = match &body {
                serde_json::Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            Some(format!(
                "[PEER MESSAGE from {from_agent_id} on {channel}]\n{body_str}\r"
            ))
        }
        AgentSignal::Cancel => Some("\u{0003}".to_string()),
    }
}

/// Check if the named Zellij session is running. Mirrors
/// `ZellijSession::is_alive()` including env-var stripping.
fn session_is_alive(name: &str) -> bool {
    let binary = edgeplaned_runtimes::zellij_session::zellij_binary();
    match std::process::Command::new(binary)
        .args(["list-sessions", "--short"])
        .env_remove("ZELLIJ")
        .env_remove("ZELLIJ_SESSION_NAME")
        .output()
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|line| line.trim() == name),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_is_alive_returns_false_for_nonexistent() {
        assert!(!session_is_alive("nonexistent-session-name-12345"));
    }
}
