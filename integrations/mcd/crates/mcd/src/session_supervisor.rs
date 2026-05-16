/// Persistent-session supervisor for mcd.
///
/// Replaces `task_loop::run_for_agent` for agents configured with
/// `session_mode: persistent`. Owns one interactive PTY per agent, fans
/// stdout out to N attached viewers (web UI, local socket), accepts stdin
/// + resize from any single viewer, and processes `AgentSignal`s from the
/// peer-message relay by injecting the rendered text into the PTY.
///
/// On PTY exit it relaunches with exponential backoff (1s → 60s, reset on
/// 30s+ stable runs). The plan is the kubelet-equivalent for agents.
use std::sync::Arc;
use std::time::{Duration, Instant};

use mcd_core::agent_runtime::DynAgentRuntime;
use mcd_core::types::{AgentHandle, AgentSignal};
use tokio::sync::{broadcast, mpsc};

use crate::attach_registry::{AttachEndpoints, AttachRegistry, PtyAttachEndpoints};

const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// A session is considered "stable" (worth resetting backoff for) if it ran
/// at least this long before exiting.
const STABLE_THRESHOLD: Duration = Duration::from_secs(30);

/// Fan-out channel size for stdout — each chunk is held until all subscribers
/// consume it. 1024 chunks at 4KB each = ~4MB worst-case buffer per agent;
/// slow viewers will lag but won't apply back-pressure to the PTY.
const STDOUT_BROADCAST_CAPACITY: usize = 1024;

pub async fn run_for_agent(
    agent_id: String,
    runtime: Arc<DynAgentRuntime>,
    registry: Arc<AttachRegistry>,
) {
    let mut backoff = BACKOFF_MIN;

    loop {
        let started = Instant::now();
        match run_one_session(&agent_id, &runtime, &registry).await {
            Ok(()) => {
                tracing::info!(
                    "Persistent session for agent {agent_id} exited cleanly after {:?}",
                    started.elapsed()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Persistent session for agent {agent_id} crashed after {:?}: {e:#}",
                    started.elapsed()
                );
            }
        }

        // Reset backoff if the session ran long enough to call "stable".
        if started.elapsed() >= STABLE_THRESHOLD {
            backoff = BACKOFF_MIN;
        }

        tracing::info!("Restarting persistent session for {agent_id} in {backoff:?}");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Run one PTY session. Returns when the PTY closes (process exit) or an
/// I/O error occurs. The caller decides how to back off and restart.
async fn run_one_session(
    agent_id: &str,
    runtime: &Arc<DynAgentRuntime>,
    registry: &Arc<AttachRegistry>,
) -> anyhow::Result<()> {
    let handle = AgentHandle {
        agent_id: agent_id.to_string(),
        runtime_kind: runtime.kind(),
        pid: 0,
    };

    let mut session = runtime.attach_pty(&handle).await?;
    tracing::info!("Persistent session started for agent {agent_id}");

    let (stdout_broadcast, _) = broadcast::channel::<Vec<u8>>(STDOUT_BROADCAST_CAPACITY);
    let (signal_tx, mut signal_rx) = mpsc::channel::<AgentSignal>(64);

    let endpoints = AttachEndpoints::Pty(PtyAttachEndpoints {
        stdin_tx: session.input.clone(),
        stdout_broadcast: stdout_broadcast.clone(),
        resize_tx: session.resize.clone(),
        signal_tx,
    });
    registry.register(agent_id.to_string(), endpoints).await;

    let result = pump_session(&mut session, &stdout_broadcast, &mut signal_rx).await;

    registry.unregister(agent_id).await;
    // Dropping `session` here closes input/resize channels, which lets the
    // PTY threads in `spawn_interactive_pty` exit and the master drop, which
    // reaps the child.
    drop(session);
    result
}

async fn pump_session(
    session: &mut mcd_core::types::PtySession,
    stdout_broadcast: &broadcast::Sender<Vec<u8>>,
    signal_rx: &mut mpsc::Receiver<AgentSignal>,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            biased;

            // PTY died (reader thread exited). End the session.
            chunk = session.output.recv() => {
                let Some(bytes) = chunk else {
                    return Ok(());
                };
                // `send` only errors when there are zero subscribers — that's
                // expected when nothing is attached, so silently swallow.
                let _ = stdout_broadcast.send(bytes);
            }

            // Peer message / user input → render to stdin.
            sig = signal_rx.recv() => {
                let Some(sig) = sig else {
                    // signal_tx dropped — supervisor itself going away.
                    return Ok(());
                };
                if let Some(rendered) = render_signal(sig) {
                    if session.input.send(rendered.into_bytes()).await.is_err() {
                        // PTY input task exited — session is dead.
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// Render an `AgentSignal` into PTY-injectable text, or return `None` to
/// drop. Persistent sessions are interactive — every injection ends with a
/// CR so the agent processes it as an entered prompt.
fn render_signal(sig: AgentSignal) -> Option<String> {
    match sig {
        AgentSignal::UserInput { text } => {
            let mut t = text;
            t.push('\r');
            Some(t)
        }
        AgentSignal::PeerMessage { from_agent_id, channel, body } => {
            let body_str = match &body {
                serde_json::Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            Some(format!(
                "[PEER MESSAGE from {from_agent_id} on {channel}]\n{body_str}\r"
            ))
        }
        AgentSignal::Cancel => {
            // ETX — Ctrl-C; lets the agent abort whatever it's doing without
            // tearing the session down. Supervisor only relaunches on PTY exit.
            Some("\u{0003}".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_user_input_appends_cr() {
        let s = render_signal(AgentSignal::UserInput { text: "hello".into() }).unwrap();
        assert_eq!(s, "hello\r");
    }

    #[test]
    fn render_peer_message_includes_provenance_and_cr() {
        let s = render_signal(AgentSignal::PeerMessage {
            from_agent_id: "research-1".into(),
            channel: "coordination".into(),
            body: json!("done"),
        })
        .unwrap();
        assert!(s.starts_with("[PEER MESSAGE from research-1 on coordination]"));
        assert!(s.ends_with("done\r"));
    }

    #[test]
    fn render_cancel_emits_etx() {
        let s = render_signal(AgentSignal::Cancel).unwrap();
        assert_eq!(s, "\u{0003}");
    }
}
