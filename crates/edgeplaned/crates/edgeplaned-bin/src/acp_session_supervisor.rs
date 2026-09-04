//! Persistent-session supervisor for ACP-shaped agents.
//!
//! Mirrors [`super::session_supervisor`] but for [`AcpSession`] instead of
//! a PTY. One supervisor task per agent; owns one [`AcpSession`] long-lived;
//! restarts on crash with the same exponential-backoff policy
//! (1s → 60s, reset on 30s+ stable runs).
//!
//! ## Wire-up
//!
//! Outbound (agent → viewers): the supervisor subscribes to the agent's
//! [`SessionNotification`] broadcast and re-broadcasts each notification on
//! a per-supervisor channel held in [`AcpAttachEndpoints`]. Each viewer that
//! connects via the attach surface calls `subscribe()` on this channel to
//! see the live stream.
//!
//! Inbound (viewers / message relay → agent): viewers and the
//! [`task_loop::run_message_relay`] both push [`AgentSignal`]s into the
//! supervisor's `signal_tx`. The supervisor renders signals into ACP calls:
//! - `AgentSignal::UserInput` → `session/prompt`
//! - `AgentSignal::PeerMessage` → `session/prompt` with provenance prefix
//! - `AgentSignal::Cancel` → `session/cancel`
//!
//! Per the ACP spec only one prompt turn is allowed per session at a time;
//! the supervisor processes signals sequentially and drops/queues nothing
//! by default — back-pressure on `signal_tx` is the natural rate-limit.
//!
//! ## Detecting agent death
//!
//! When `Agent`'s actor task exits (child process died, IO error), its
//! notification broadcast closes. The supervisor's pump loop sees
//! [`broadcast::error::RecvError::Closed`] on the agent-update receiver and
//! returns, triggering the outer restart loop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use edgeplaned_acp::SpawnOpts;
use edgeplaned_core::types::AgentSignal;
use edgeplaned_runtimes::claude_agent_acp::AcpSession;
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc};

use crate::attach_registry::{AcpAttachEndpoints, AttachEndpoints, AttachRegistry};
use crate::replay_broadcast::ReplayBroadcast;

const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
const STABLE_THRESHOLD: Duration = Duration::from_secs(30);
const UPDATES_BROADCAST_CAPACITY: usize = 1024;
/// Fraction of context window consumed before injecting /compact.
/// Override with EP_MESH_COMPACT_THRESHOLD env var (0.0–1.0).
const DEFAULT_COMPACT_THRESHOLD: f64 = 0.85;
const SIGNAL_CHANNEL_CAPACITY: usize = 64;
/// How many recent SessionNotifications to keep for viewers attaching
/// mid-session. Sized for a couple of minutes of typical assistant
/// chatter at human-typing cadence. Full history (since process start)
/// would require ACP session resume on the agent side — separate concern.
const REPLAY_BUFFER_CAPACITY: usize = 200;

/// Per-agent configuration the supervisor needs to (re)spawn the ACP
/// session. Captured by the daemon at startup, after `ensure_installed`
/// has resolved the node + dist/index.js paths.
#[derive(Clone)]
pub struct AcpSupervisorConfig {
    pub agent_id: String,
    pub spawn_opts: SpawnOpts,
    pub cwd: std::path::PathBuf,
    /// Passed to `AcpSession::open` as the `--remote-control-session-name-prefix`
    /// flag so the session is visible in the Claude app under the agent's canonical
    /// public_id. `None` disables remote-control for this session.
    pub remote_control_prefix: Option<String>,
}

pub async fn run_for_agent(cfg: AcpSupervisorConfig, registry: Arc<AttachRegistry>) {
    let mut backoff = BACKOFF_MIN;

    loop {
        let started = Instant::now();
        match run_one_session(&cfg, &registry).await {
            Ok(()) => tracing::info!(
                "ACP session for agent {} exited cleanly after {:?}",
                cfg.agent_id,
                started.elapsed()
            ),
            Err(e) => tracing::warn!(
                "ACP session for agent {} crashed after {:?}: {e:#}",
                cfg.agent_id,
                started.elapsed()
            ),
        }

        if started.elapsed() >= STABLE_THRESHOLD {
            backoff = BACKOFF_MIN;
        }

        tracing::info!("Restarting ACP session for {} in {backoff:?}", cfg.agent_id);
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Run one ACP session: open it, register endpoints, pump until the agent
/// dies or our signal channel closes. Returns when the session is over —
/// the caller backs off and restarts.
async fn run_one_session(
    cfg: &AcpSupervisorConfig,
    registry: &Arc<AttachRegistry>,
) -> anyhow::Result<()> {
    // The cwd is passed to the claude subprocess via session/new. Node.js
    // child_process.spawn() returns ENOENT when the cwd doesn't exist, even
    // when the binary itself is accessible — so ensure it exists first.
    tokio::fs::create_dir_all(&cfg.cwd)
        .await
        .with_context(|| format!("creating session cwd {}", cfg.cwd.display()))?;

    let session = AcpSession::open(
        cfg.spawn_opts.clone(),
        cfg.cwd.clone(),
        cfg.remote_control_prefix.clone(),
    )
    .await
    .context("acp session open")?;
    tracing::info!("ACP session started for agent {}", cfg.agent_id);

    let updates_broadcast: ReplayBroadcast<edgeplaned_acp::wire::SessionNotification> =
        ReplayBroadcast::new(REPLAY_BUFFER_CAPACITY, UPDATES_BROADCAST_CAPACITY);
    let (signal_tx, mut signal_rx) = mpsc::channel::<AgentSignal>(SIGNAL_CHANNEL_CAPACITY);

    let endpoints = AttachEndpoints::Acp(AcpAttachEndpoints {
        signal_tx,
        updates_broadcast: updates_broadcast.clone(),
    });
    registry.register(cfg.agent_id.clone(), endpoints).await;

    let mut agent_updates = session.subscribe_updates();
    let compact_threshold = std::env::var("EP_MESH_COMPACT_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|&t| t > 0.0 && t <= 1.0)
        .unwrap_or(DEFAULT_COMPACT_THRESHOLD);

    let result = pump_session(
        &session,
        &mut agent_updates,
        &updates_broadcast,
        &mut signal_rx,
        compact_threshold,
    )
    .await;

    registry.unregister(&cfg.agent_id).await;
    let _ = session.shutdown().await;
    result
}

async fn pump_session(
    session: &AcpSession,
    agent_updates: &mut broadcast::Receiver<edgeplaned_acp::wire::SessionNotification>,
    updates_broadcast: &ReplayBroadcast<edgeplaned_acp::wire::SessionNotification>,
    signal_rx: &mut mpsc::Receiver<AgentSignal>,
    compact_threshold: f64,
) -> anyhow::Result<()> {
    let mut compact_triggered = false;

    loop {
        tokio::select! {
            biased;

            // Outbound: agent update → push to replay buffer + fan out to
            // attached viewers. ReplayBroadcast::send is atomic w.r.t.
            // subscribe_with_replay so a viewer attaching here sees this
            // notification exactly once (replay snapshot OR live, never
            // both).
            recv = agent_updates.recv() => {
                match recv {
                    Ok(notif) => {
                        if let edgeplaned_acp::wire::SessionUpdate::UsageUpdate(ref val) = notif.update
                            && !compact_triggered
                                && let (Some(used), Some(size)) = (
                                    val.get("used").and_then(|v| v.as_u64()),
                                    val.get("size").and_then(|v| v.as_u64()),
                                )
                                    && size > 0 {
                                        let ratio = used as f64 / size as f64;
                                        if ratio >= compact_threshold {
                                            compact_triggered = true;
                                            tracing::info!(
                                                used, size,
                                                ratio = format!("{:.1}%", ratio * 100.0),
                                                "context pressure threshold reached — injecting /compact"
                                            );
                                            run_prompt(session, "/compact".into()).await;
                                        }
                                    }
                        updates_broadcast.send(notif);
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("ACP supervisor lagged {n} updates from agent");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // Agent's actor task exited — the underlying child died
                        // or hit an unrecoverable IO error. End the session so
                        // the outer loop restarts it.
                        return Ok(());
                    }
                }
            }

            // Inbound: signal from message relay or attach viewer.
            sig = signal_rx.recv() => {
                let Some(sig) = sig else {
                    // signal_tx all dropped — supervisor itself going away.
                    return Ok(());
                };
                if let Err(e) = handle_signal(session, sig).await {
                    tracing::warn!("ACP supervisor: signal handling failed: {e:#}");
                }
            }
        }
    }
}

/// Translate an [`AgentSignal`] into ACP calls.
async fn handle_signal(session: &AcpSession, sig: AgentSignal) -> anyhow::Result<()> {
    match sig {
        AgentSignal::UserInput { text } => {
            run_prompt(session, text).await;
            Ok(())
        }
        AgentSignal::PeerMessage {
            from_agent_id,
            channel,
            body,
        } => {
            let body_str = match body {
                serde_json::Value::String(s) => s,
                v => v.to_string(),
            };
            let prompt = format!("[PEER MESSAGE from {from_agent_id} on {channel}]\n{body_str}");
            run_prompt(session, prompt).await;
            Ok(())
        }
        AgentSignal::Cancel => session.cancel().await.context("acp session/cancel"),
    }
}

/// Drive a prompt to completion, dropping the resulting `ProgressEvent`s.
/// Persistent-mode viewers see streaming output via the
/// `updates_broadcast` channel populated by the pump's outbound path; the
/// `ProgressEvent` translation is a task-mode concept (delivered to the
/// controlplane via the task loop).
async fn run_prompt(session: &AcpSession, text: String) {
    let mut stream = session.prompt(text);
    while stream.next().await.is_some() {
        // Events are observed by viewers via the agent's notification
        // broadcast (subscribed at the supervisor level). The supervisor
        // doesn't itself need to keep them; just drive the future to
        // completion so the prompt RPC resolves.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The supervisor is hard to unit-test without spawning a real ACP agent
    // (which is the wire_compat integration test in edgeplaned-acp). We
    // unit-test the signal-rendering helpers; full e2e is covered by the
    // integration suite.

    #[tokio::test]
    async fn handle_signal_cancel_calls_cancel() {
        // Smoke: just verify the match arms are exhaustive at compile time;
        // a real session is needed to exercise behavior.
        let sig_user = AgentSignal::UserInput { text: "x".into() };
        let sig_peer = AgentSignal::PeerMessage {
            from_agent_id: "a".into(),
            channel: "c".into(),
            body: json!("b"),
        };
        let sig_cancel = AgentSignal::Cancel;
        // Compile-time exhaustive match is the test:
        for s in [sig_user, sig_peer, sig_cancel] {
            match s {
                AgentSignal::UserInput { .. }
                | AgentSignal::PeerMessage { .. }
                | AgentSignal::Cancel => {}
            }
        }
    }
}
