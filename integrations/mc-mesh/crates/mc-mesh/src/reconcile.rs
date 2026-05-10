//! Live agent reconciliation: spawn/restart/shutdown supervisors based on
//! the controlplane's authoritative agent list.
//!
//! The daemon's start-time path runs `reconcile(initial_specs, ...)` once
//! to spawn whatever's currently assigned. After that, two long-lived
//! tasks keep the local supervisor set converged with the controlplane:
//!
//! - **WS subscriber** (`watch_assignments_ws`) listens on
//!   `/runtime/nodes/{id}/notify` and fires reconciles on each event.
//! - **Poll fallback** (`poll_assignments`) re-runs the GET every ~60s
//!   to catch anything the WS dropped (network blips, server restart).
//!
//! Both ultimately call [`Spawner::reconcile`] which diffs `desired`
//! against `running` and:
//!
//! 1. Shuts down agents present in `running` but absent from `desired`.
//! 2. Spawns agents present in `desired` but absent from `running`.
//! 3. For agents in both, restarts iff the spec materially changed
//!    (mission_id, runtime_kind, supervision_mode). Capability-only
//!    changes don't require restart.
//!
//! ## Shutdown policy
//!
//! Per-agent shutdown is `JoinHandle::abort()` with a 5s grace window.
//! Both supervisor types have child processes registered with
//! `kill_on_drop(true)`, so aborting the task drops the supervisor's
//! state and reaps the child — clean enough for the persistence model
//! (state lives in the controlplane / vault, not in agent memory).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use mc_mesh_core::client::BackendClient;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::daemon::AgentSpec;

/// Periodic poll interval: WS is primary, this catches WS drops + verifies
/// state every minute. Override via `MC_MESH_POLL_SECS` for tests.
pub const DEFAULT_POLL_SECS: u64 = 60;

/// Per-agent abort grace before we move on. The supervisor's `Drop`
/// (via `kill_on_drop`) already handles the child process; this is
/// just task-cleanup time.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Reconnect backoff bounds for the WS subscriber.
pub const WS_RECONNECT_MIN: Duration = Duration::from_secs(1);
pub const WS_RECONNECT_MAX: Duration = Duration::from_secs(60);

/// One running agent — the spec it was spawned from + the task handles
/// owning its supervisor / message-relay tasks.
pub struct RunningAgent {
    pub spec: AgentSpec,
    pub handles: Vec<JoinHandle<()>>,
}

impl RunningAgent {
    pub fn new(spec: AgentSpec, handles: Vec<JoinHandle<()>>) -> Self {
        Self { spec, handles }
    }

    /// Abort all task handles, then await each with a 5s grace so the
    /// supervisor's `Drop` runs.
    pub async fn shutdown(self) {
        for h in &self.handles {
            h.abort();
        }
        for h in self.handles {
            let _ = tokio::time::timeout(SHUTDOWN_GRACE, h).await;
        }
    }
}

/// `running` map shared across the daemon. Wrapped in `Arc<Mutex<>>` so the
/// WS subscriber, poll loop, and start-time path can all reconcile against
/// the same state.
pub type RunningAgents = Arc<Mutex<HashMap<String, RunningAgent>>>;

/// Diff `desired` against `running`. Returns three lists:
/// - `to_remove`: agent_ids to shut down (in running, not in desired)
/// - `to_spawn`: specs to spawn (in desired, not in running)
/// - `to_restart`: specs whose stable fields changed (mission_id,
///   runtime_kind, supervision_mode). Returned in (old_id, new_spec) form.
pub fn diff_specs(
    desired: &[AgentSpec],
    running: &HashMap<String, RunningAgent>,
) -> ReconcilePlan {
    let desired_by_id: HashMap<&str, &AgentSpec> =
        desired.iter().map(|s| (s.agent_id.as_str(), s)).collect();
    let running_ids: HashSet<&str> = running.keys().map(|s| s.as_str()).collect();

    let to_remove: Vec<String> = running_ids
        .iter()
        .filter(|id| !desired_by_id.contains_key(*id))
        .map(|s| s.to_string())
        .collect();

    let mut to_spawn: Vec<AgentSpec> = Vec::new();
    let mut to_restart: Vec<AgentSpec> = Vec::new();
    for spec in desired {
        match running.get(&spec.agent_id) {
            None => to_spawn.push(spec.clone()),
            Some(existing) => {
                if !specs_match(&existing.spec, spec) {
                    to_restart.push(spec.clone());
                }
            }
        }
    }

    ReconcilePlan {
        to_remove,
        to_spawn,
        to_restart,
    }
}

/// Materially-equal-for-supervisor purposes. Capability differences alone
/// don't require restart — they take effect on the next task claim.
pub fn specs_match(a: &AgentSpec, b: &AgentSpec) -> bool {
    a.agent_id == b.agent_id
        && a.mission_id == b.mission_id
        && a.runtime_kind == b.runtime_kind
        && a.session_mode == b.session_mode
        && a.profile_path == b.profile_path
}

#[derive(Debug, Default)]
pub struct ReconcilePlan {
    pub to_remove: Vec<String>,
    pub to_spawn: Vec<AgentSpec>,
    pub to_restart: Vec<AgentSpec>,
}

impl ReconcilePlan {
    pub fn is_noop(&self) -> bool {
        self.to_remove.is_empty() && self.to_spawn.is_empty() && self.to_restart.is_empty()
    }
}

// ── poll fallback ────────────────────────────────────────────────────────────

/// Periodic poll loop: every ~60s, fetch the agent list and reconcile.
/// Catches anything the WS subscriber missed (drops, server restarts, etc).
pub async fn poll_assignments<F, Fut>(
    client: Arc<BackendClient>,
    node_id: String,
    running: RunningAgents,
    mut reconcile_fn: F,
) where
    F: FnMut(Vec<AgentSpec>, RunningAgents) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let interval = Duration::from_secs(
        std::env::var("MC_MESH_POLL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_POLL_SECS),
    );
    loop {
        tokio::time::sleep(interval).await;
        match crate::daemon::fetch_node_agents(&client, &node_id).await {
            Ok(specs) => reconcile_fn(specs, Arc::clone(&running)).await,
            Err(e) => tracing::warn!("poll fetch failed for node {node_id}: {e:#}"),
        }
    }
}

// ── WS subscriber ────────────────────────────────────────────────────────────

/// WebSocket subscription to `/runtime/nodes/{id}/notify`. Reconnects on
/// disconnect with exponential backoff (1s → 60s). On every successful
/// connection AND on every received `agent.*` event, fires a full reconcile
/// against the current controlplane state — events are hints, the GET is
/// truth.
pub async fn watch_assignments_ws<F, Fut>(
    backend_url: String,
    token: String,
    node_id: String,
    client: Arc<BackendClient>,
    running: RunningAgents,
    mut reconcile_fn: F,
) where
    F: FnMut(Vec<AgentSpec>, RunningAgents) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let mut backoff = WS_RECONNECT_MIN;

    loop {
        let url = ws_url(&backend_url, &node_id);
        match connect_and_pump(&url, &token).await {
            Ok(()) => {
                // Clean disconnect — reset backoff, reconnect immediately.
                backoff = WS_RECONNECT_MIN;
            }
            Err(e) => {
                tracing::warn!(
                    "WS subscription to {url} failed: {e:#}. Reconnecting in {backoff:?}."
                );
            }
        }

        // After every connect or disconnect: reconcile via GET so we're
        // not relying solely on the event stream.
        match crate::daemon::fetch_node_agents(&client, &node_id).await {
            Ok(specs) => reconcile_fn(specs, Arc::clone(&running)).await,
            Err(e) => tracing::warn!("post-WS reconcile fetch failed: {e:#}"),
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(WS_RECONNECT_MAX);
    }
}

fn ws_url(backend_url: &str, node_id: &str) -> String {
    let scheme = if backend_url.starts_with("https") {
        "wss"
    } else {
        "ws"
    };
    let host_and_path = backend_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    format!("{scheme}://{host_and_path}/runtime/nodes/{node_id}/notify")
}

/// Connect to the WS endpoint, pump messages until the server closes or
/// we hit an error. Each `agent.*` event is logged; the loop above does
/// the actual reconciliation by re-fetching the GET, so we don't try to
/// be clever about applying single events incrementally — events are
/// purely a wake-up signal.
async fn connect_and_pump(url: &str, token: &str) -> Result<()> {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;

    let mut req = url.into_client_request()?;
    req.headers_mut()
        .insert(AUTHORIZATION, format!("Bearer {token}").parse()?);
    let (mut ws, _resp) = tokio_tungstenite::connect_async(req).await?;

    while let Some(msg) = ws.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("WS read error: {e}");
                break;
            }
        };
        if msg.is_close() {
            break;
        }
        if let Ok(text) = msg.to_text() {
            if text.is_empty() {
                continue;
            }
            tracing::debug!("WS event: {text}");
            // Heuristic: if it's an `agent.*` event, return so the outer
            // loop re-fetches and reconciles. Pings (`{"type":"ping"}`)
            // are ignored — the connection just stays open.
            if text.contains("\"agent.") {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SessionMode;

    fn spec(id: &str, mission: &str, mode: SessionMode) -> AgentSpec {
        AgentSpec {
            agent_id: id.into(),
            mission_id: mission.into(),
            runtime_kind: "claude_agent_acp".into(),
            session_mode: mode,
            capabilities: vec![],
            profile_path: None,
        }
    }

    fn running_with(spec: AgentSpec) -> HashMap<String, RunningAgent> {
        let mut m = HashMap::new();
        m.insert(
            spec.agent_id.clone(),
            RunningAgent {
                spec,
                handles: vec![],
            },
        );
        m
    }

    #[test]
    fn diff_empty_returns_noop() {
        let plan = diff_specs(&[], &HashMap::new());
        assert!(plan.is_noop());
    }

    #[test]
    fn diff_new_agent_goes_to_spawn() {
        let s = spec("a-1", "m-1", SessionMode::Persistent);
        let plan = diff_specs(&[s.clone()], &HashMap::new());
        assert_eq!(plan.to_spawn.len(), 1);
        assert_eq!(plan.to_spawn[0].agent_id, "a-1");
        assert!(plan.to_remove.is_empty());
        assert!(plan.to_restart.is_empty());
    }

    #[test]
    fn diff_removed_agent_goes_to_remove() {
        let running = running_with(spec("a-1", "m-1", SessionMode::Persistent));
        let plan = diff_specs(&[], &running);
        assert_eq!(plan.to_remove, vec!["a-1".to_string()]);
    }

    #[test]
    fn diff_unchanged_agent_is_noop() {
        let s = spec("a-1", "m-1", SessionMode::Persistent);
        let running = running_with(s.clone());
        let plan = diff_specs(&[s], &running);
        assert!(plan.is_noop());
    }

    #[test]
    fn diff_mission_change_goes_to_restart() {
        let old = spec("a-1", "m-1", SessionMode::Persistent);
        let new = spec("a-1", "m-2", SessionMode::Persistent);
        let running = running_with(old);
        let plan = diff_specs(&[new], &running);
        assert_eq!(plan.to_restart.len(), 1);
        assert_eq!(plan.to_restart[0].mission_id, "m-2");
        assert!(plan.to_spawn.is_empty());
        assert!(plan.to_remove.is_empty());
    }

    #[test]
    fn diff_session_mode_change_goes_to_restart() {
        let old = spec("a-1", "m-1", SessionMode::Task);
        let new = spec("a-1", "m-1", SessionMode::Persistent);
        let running = running_with(old);
        let plan = diff_specs(&[new], &running);
        assert_eq!(plan.to_restart.len(), 1);
    }

    #[test]
    fn diff_capability_only_change_does_not_restart() {
        let old = spec("a-1", "m-1", SessionMode::Persistent);
        let mut new = old.clone();
        new.capabilities = vec!["code.review".into()];
        let running = running_with(old);
        let plan = diff_specs(&[new], &running);
        assert!(plan.is_noop(), "cap-only changes should not restart");
    }

    #[test]
    fn diff_multi_agent_mixed_actions() {
        // Running: a-1 (m-1, persistent), a-2 (m-1, task)
        // Desired: a-1 (m-2, persistent — restart), a-3 (m-1, task — spawn)
        // Should result in: remove a-2, spawn a-3, restart a-1
        let mut running = HashMap::new();
        running.insert(
            "a-1".into(),
            RunningAgent {
                spec: spec("a-1", "m-1", SessionMode::Persistent),
                handles: vec![],
            },
        );
        running.insert(
            "a-2".into(),
            RunningAgent {
                spec: spec("a-2", "m-1", SessionMode::Task),
                handles: vec![],
            },
        );
        let desired = vec![
            spec("a-1", "m-2", SessionMode::Persistent),
            spec("a-3", "m-1", SessionMode::Task),
        ];
        let plan = diff_specs(&desired, &running);
        assert_eq!(plan.to_remove, vec!["a-2".to_string()]);
        assert_eq!(plan.to_spawn.len(), 1);
        assert_eq!(plan.to_spawn[0].agent_id, "a-3");
        assert_eq!(plan.to_restart.len(), 1);
        assert_eq!(plan.to_restart[0].agent_id, "a-1");
    }

    #[test]
    fn ws_url_http_to_ws() {
        assert_eq!(
            ws_url("http://localhost:8008", "node-x"),
            "ws://localhost:8008/runtime/nodes/node-x/notify"
        );
    }

    #[test]
    fn ws_url_https_to_wss() {
        assert_eq!(
            ws_url("https://mc.example.com", "node-x"),
            "wss://mc.example.com/runtime/nodes/node-x/notify"
        );
    }

    #[test]
    fn ws_url_strips_trailing_slash() {
        assert_eq!(
            ws_url("http://localhost:8008/", "node-x"),
            "ws://localhost:8008/runtime/nodes/node-x/notify"
        );
    }
}
