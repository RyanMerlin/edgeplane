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
//!    (domain_id, runtime_kind, supervision_mode). Capability-only
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
use edgeplaned_core::client::BackendClient;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::daemon::AgentSpec;

/// Periodic poll interval: WS is primary, this catches WS drops + verifies
/// state every minute. Override via `EP_MESH_POLL_SECS` for tests.
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
/// - `to_restart`: specs whose stable fields changed (domain_id,
///   runtime_kind, supervision_mode). Returned in (old_id, new_spec) form.
///
/// ## Federated alias handling (`local_alias_id`)
///
/// When a federated (controlplane) spec carries a `local_alias_id`, the
/// controlplane agent_id differs from the local fleet_import agent_id that
/// is already running. For example: the running map has key `"engineer"` (the
/// fleet_import agent_id) but the desired spec has `agent_id =
/// "my-agent-engineer-708650f1"` and `local_alias_id = Some("engineer")`.
///
/// Without alias handling, diff_specs would see `"my-agent-engineer-708650f1"` as
/// new (→ `to_spawn`) and `"engineer"` as orphaned (→ `to_remove`), tearing
/// down the live PTY bridge and spawning a duplicate.
///
/// With alias handling:
/// - When looking up a desired spec in `running`, also try the alias key.
/// - When computing `to_remove`, exclude any running key that appears as a
///   `local_alias_id` in the desired list.
pub fn diff_specs(desired: &[AgentSpec], running: &HashMap<String, RunningAgent>) -> ReconcilePlan {
    // Canonical id → spec lookup.
    let desired_by_id: HashMap<&str, &AgentSpec> =
        desired.iter().map(|s| (s.agent_id.as_str(), s)).collect();

    // Build the set of local alias ids covered by the desired list so we can
    // exclude them from to_remove.
    let desired_alias_ids: HashSet<&str> = desired
        .iter()
        .filter_map(|s| s.local_alias_id.as_deref())
        .collect();

    let running_ids: HashSet<&str> = running.keys().map(|s| s.as_str()).collect();

    // A running agent should be removed when:
    //   1. Its id is not in the desired spec list (by canonical id), AND
    //   2. Its id is not an alias of any desired spec (i.e. not covered by a
    //      federated spec that has local_alias_id == this running key).
    let to_remove: Vec<String> = running_ids
        .iter()
        .filter(|id| !desired_by_id.contains_key(*id) && !desired_alias_ids.contains(*id))
        .map(|s| s.to_string())
        .collect();

    let mut to_spawn: Vec<AgentSpec> = Vec::new();
    let mut to_restart: Vec<AgentSpec> = Vec::new();
    let mut alias_registrations: Vec<(String, String)> = Vec::new();
    for spec in desired {
        // Look up by canonical id first, then by local_alias_id (for federated
        // specs whose controlplane id differs from the already-running local id).
        let matched_by_alias =
            running.get(&spec.agent_id).is_none() && spec.local_alias_id.is_some();
        let running_agent = running.get(&spec.agent_id).or_else(|| {
            spec.local_alias_id
                .as_deref()
                .and_then(|alias| running.get(alias))
        });

        match running_agent {
            None => to_spawn.push(spec.clone()),
            Some(existing) => {
                // When matched via alias, skip the agent_id comparison
                // (the controlplane id and the local fleet id are different
                // by design — they refer to the same logical agent).
                let materially_changed = if matched_by_alias {
                    !specs_match_ignoring_id(&existing.spec, spec)
                } else {
                    !specs_match(&existing.spec, spec)
                };
                if materially_changed {
                    to_restart.push(spec.clone());
                } else if matched_by_alias {
                    // The spec is a no-op (already running under the alias key)
                    // but the controlplane public_id differs from the local
                    // fleet key. Record the pair so the reconcile caller can
                    // register an attach-registry alias, enabling web attach
                    // for the public_id to reach the existing PTY bridge.
                    if let Some(ref local_id) = spec.local_alias_id {
                        alias_registrations.push((spec.agent_id.clone(), local_id.clone()));
                    }
                }
            }
        }
    }

    ReconcilePlan {
        to_remove,
        to_spawn,
        to_restart,
        alias_registrations,
    }
}

/// Materially-equal-for-supervisor purposes. Capability differences alone
/// don't require restart — they take effect on the next task claim.
pub fn specs_match(a: &AgentSpec, b: &AgentSpec) -> bool {
    a.agent_id == b.agent_id
        && a.domain_id == b.domain_id
        && a.runtime_kind == b.runtime_kind
        && a.session_mode == b.session_mode
        && a.profile_path == b.profile_path
}

/// Same as `specs_match` but skips the `agent_id` comparison. Used when a
/// federated spec was located in the running map via its `local_alias_id` —
/// the two specs will always differ in `agent_id` (controlplane vs. local
/// fleet key) but that alone is not a reason to restart.
pub fn specs_match_ignoring_id(a: &AgentSpec, b: &AgentSpec) -> bool {
    a.domain_id == b.domain_id
        && a.runtime_kind == b.runtime_kind
        && a.session_mode == b.session_mode
        && a.profile_path == b.profile_path
}

#[derive(Debug, Default)]
pub struct ReconcilePlan {
    pub to_remove: Vec<String>,
    pub to_spawn: Vec<AgentSpec>,
    pub to_restart: Vec<AgentSpec>,
    /// Pairs of `(public_id, local_alias_id)` for federated specs that were
    /// recognised as already-running (matched via `local_alias_id`) and do
    /// not need to be re-spawned. The reconcile caller uses these to register
    /// attach-registry aliases so an incoming attach for `public_id` reaches
    /// the PTY bridge registered under `local_alias_id`.
    pub alias_registrations: Vec<(String, String)>,
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
        std::env::var("EP_MESH_POLL_SECS")
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
        let url = ws_url(&backend_url, &client.api_prefix, &node_id);
        // Read the live token at each (re)connect so a rotation between
        // attempts uses the current credential rather than the one captured
        // at daemon start.
        let token = client.current_token();
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

fn ws_url(backend_url: &str, api_prefix: &str, node_id: &str) -> String {
    let scheme = if backend_url.starts_with("https") {
        "wss"
    } else {
        "ws"
    };
    let host_and_path = backend_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    // The notify route is mounted under the same `api_prefix` as every other
    // controlplane route (default `/api`). The GET/POST `BackendClient` already
    // prepends this prefix; the WS URL must too or the daemon dials the wrong
    // path and the subscription 404s (falling back to ~60s polling).
    let prefix = api_prefix.trim_end_matches('/');
    format!("{scheme}://{host_and_path}{prefix}/runtime/nodes/{node_id}/notify")
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

    fn spec(id: &str, domain: &str, mode: SessionMode) -> AgentSpec {
        AgentSpec {
            agent_id: id.into(),
            domain_id: domain.into(),
            runtime_kind: "claude_agent_acp".into(),
            session_mode: mode,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: Default::default(),
            name: None,
            local_alias_id: None,
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
        let plan = diff_specs(std::slice::from_ref(&s), &HashMap::new());
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
    fn diff_domain_change_goes_to_restart() {
        let old = spec("a-1", "m-1", SessionMode::Persistent);
        let new = spec("a-1", "m-2", SessionMode::Persistent);
        let running = running_with(old);
        let plan = diff_specs(&[new], &running);
        assert_eq!(plan.to_restart.len(), 1);
        assert_eq!(plan.to_restart[0].domain_id, "m-2");
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

    // ── federated alias tests ────────────────────────────────────────────────

    /// Federated spec with local_alias_id set + matching running entry under
    /// the alias key → should be a no-op (no spawn, no remove).
    #[test]
    fn diff_federated_alias_running_is_noop() {
        // The local agent "engineer" is already running as a zellij_hosted agent.
        let local_running = AgentSpec {
            agent_id: "engineer".into(),
            domain_id: "m-1".into(),
            runtime_kind: "zellij_hosted".into(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: crate::supervisor::SpawnOverrides {
                zellij_session: Some("engineer".into()),
                ..Default::default()
            },
            name: None,
            local_alias_id: None,
        };
        let running = running_with(local_running);

        // The controlplane wants "my-agent-engineer-abc12345" (same logical agent,
        // but a different id) with local_alias_id pointing at the running key.
        let mut cp_spec = AgentSpec {
            agent_id: "my-agent-engineer-abc12345".into(),
            domain_id: "m-1".into(),
            runtime_kind: "zellij_hosted".into(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: Default::default(),
            name: Some("my-agent-engineer".into()),
            local_alias_id: Some("engineer".into()),
        };
        cp_spec.launch_overrides.zellij_session = Some("my-agent-engineer".into());

        let plan = diff_specs(&[cp_spec], &running);
        assert!(
            plan.is_noop(),
            "federated spec aliased to a running local agent should be a no-op; \
             got to_remove={:?} to_spawn={:?} to_restart={:?}",
            plan.to_remove,
            plan.to_spawn
                .iter()
                .map(|s| &s.agent_id)
                .collect::<Vec<_>>(),
            plan.to_restart
                .iter()
                .map(|s| &s.agent_id)
                .collect::<Vec<_>>(),
        );
    }

    /// Federated spec aliases a running local agent; the local key must NOT
    /// appear in to_remove.
    #[test]
    fn diff_federated_alias_does_not_remove_local_key() {
        // Local fleet-import "operator" running as zellij_hosted.
        let local_running = AgentSpec {
            agent_id: "operator".into(),
            domain_id: "m-1".into(),
            runtime_kind: "zellij_hosted".into(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: crate::supervisor::SpawnOverrides {
                zellij_session: Some("operator".into()),
                ..Default::default()
            },
            name: None,
            local_alias_id: None,
        };
        let running = running_with(local_running);

        let cp_spec = AgentSpec {
            agent_id: "my-agent-operator-deadbeef".into(),
            domain_id: "m-1".into(),
            runtime_kind: "zellij_hosted".into(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: crate::supervisor::SpawnOverrides {
                zellij_session: Some("my-agent-operator".into()),
                ..Default::default()
            },
            name: Some("my-agent-operator".into()),
            local_alias_id: Some("operator".into()),
        };

        let plan = diff_specs(&[cp_spec], &running);
        assert!(
            !plan.to_remove.contains(&"operator".to_string()),
            "local alias key 'operator' must not appear in to_remove; got {:?}",
            plan.to_remove
        );
        assert!(plan.to_spawn.is_empty(), "should not spawn a duplicate");
    }

    /// A federated spec that is a no-op (running under the alias key) should
    /// record an alias_registration so the reconcile caller can wire the
    /// attach registry alias — this is the attach-reachability fix.
    #[test]
    fn diff_federated_noop_emits_alias_registration() {
        // Running: "engineer" (local fleet-import key).
        let local_running = AgentSpec {
            agent_id: "engineer".into(),
            domain_id: "m-1".into(),
            runtime_kind: "zellij_hosted".into(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: crate::supervisor::SpawnOverrides {
                zellij_session: Some("engineer".into()),
                ..Default::default()
            },
            name: None,
            local_alias_id: None,
        };
        let running = running_with(local_running);

        // Desired: controlplane public_id with local_alias_id pointing at "engineer".
        let cp_spec = AgentSpec {
            agent_id: "my-agent-engineer-708650f1".into(),
            domain_id: "m-1".into(),
            runtime_kind: "zellij_hosted".into(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: crate::supervisor::SpawnOverrides {
                zellij_session: Some("engineer".into()),
                ..Default::default()
            },
            name: Some("my-agent-engineer".into()),
            local_alias_id: Some("engineer".into()),
        };

        let plan = diff_specs(&[cp_spec], &running);
        assert!(plan.is_noop(), "should be a no-op for the running map");
        assert_eq!(
            plan.alias_registrations,
            vec![(
                "my-agent-engineer-708650f1".to_string(),
                "engineer".to_string()
            )],
            "must emit alias registration for attach-registry wiring"
        );
    }

    /// Without a local_alias_id, a new controlplane id with a running local
    /// id gets the old (correct pre-fix) behaviour: the local id is removed
    /// and the new controlplane spec is spawned.
    #[test]
    fn diff_no_alias_still_removes_local_and_spawns_cp() {
        let local_running = AgentSpec {
            agent_id: "engineer".into(),
            domain_id: "m-1".into(),
            runtime_kind: "zellij_hosted".into(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: crate::supervisor::SpawnOverrides {
                zellij_session: Some("engineer".into()),
                ..Default::default()
            },
            name: None,
            local_alias_id: None,
        };
        let running = running_with(local_running);

        // Controlplane spec has a different id but NO alias.
        let cp_spec = AgentSpec {
            agent_id: "my-agent-engineer-abc12345".into(),
            domain_id: "m-1".into(),
            runtime_kind: "zellij_hosted".into(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: Default::default(),
            name: Some("my-agent-engineer".into()),
            local_alias_id: None, // no alias
        };

        let plan = diff_specs(&[cp_spec], &running);
        assert!(
            plan.to_remove.contains(&"engineer".to_string()),
            "without alias, local 'engineer' should be in to_remove"
        );
        assert_eq!(plan.to_spawn.len(), 1);
        assert_eq!(plan.to_spawn[0].agent_id, "my-agent-engineer-abc12345");
    }

    // ── URL helper tests ─────────────────────────────────────────────────────

    #[test]
    fn ws_url_http_to_ws() {
        assert_eq!(
            ws_url("http://localhost:8008", "/api", "node-x"),
            "ws://localhost:8008/api/runtime/nodes/node-x/notify"
        );
    }

    #[test]
    fn ws_url_https_to_wss() {
        assert_eq!(
            ws_url("https://edgeplane.example.com", "/api", "node-x"),
            "wss://edgeplane.example.com/api/runtime/nodes/node-x/notify"
        );
    }

    #[test]
    fn ws_url_strips_trailing_slash() {
        assert_eq!(
            ws_url("http://localhost:8008/", "/api", "node-x"),
            "ws://localhost:8008/api/runtime/nodes/node-x/notify"
        );
    }

    #[test]
    fn ws_url_empty_prefix_has_no_api_segment() {
        assert_eq!(
            ws_url("http://localhost:8008", "", "node-x"),
            "ws://localhost:8008/runtime/nodes/node-x/notify"
        );
    }

    #[test]
    fn ws_url_prefix_trailing_slash_normalized() {
        assert_eq!(
            ws_url("http://localhost:8008", "/api/", "node-x"),
            "ws://localhost:8008/api/runtime/nodes/node-x/notify"
        );
    }

    // ── Boot-flow dedup test ─────────────────────────────────────────────────

    /// Simulates the full federated boot sequence that produces the double-attach
    /// bug and verifies the Gap 3 dedup logic:
    ///
    ///   1. 6 controlplane specs (opaque ids, empty launch_overrides, no alias).
    ///   2. 6 additive-layer specs (short ids, with zellij_session set).
    ///      Total = 12 specs (the pre-fix input to diff_specs).
    ///   3. merge_federated_overrides sets local_alias_id on the 6 CP specs.
    ///   4. Boot dedup removes specs whose agent_id is in the aliased set.
    ///      Total = 6 specs (the post-fix input).
    ///   5. diff_specs against empty running → exactly 6 to_spawn.
    ///   6. All to_spawn are the controlplane opaque ids (the attach ids).
    ///   7. No duplicate agent_ids in to_spawn.
    ///   8. Each to_spawn spec has local_alias_id set (for supervisor alias reg).
    #[test]
    fn federated_boot_dedup_yields_exactly_6_to_spawn_under_attach_ids() {
        use crate::daemon::{AgentSpec, merge_federated_overrides};
        use crate::local_registry::AgentLaunchContext;

        let profile_names = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"];

        // Step 1: 6 controlplane specs (what fetch_node_agents returns).
        let cp_specs: Vec<AgentSpec> = profile_names
            .iter()
            .enumerate()
            .map(|(i, name)| AgentSpec {
                agent_id: format!("my-agent-{name}-{i:08x}"),
                domain_id: "d-1".to_string(),
                runtime_kind: "zellij_hosted".to_string(),
                session_mode: SessionMode::Persistent,
                capabilities: vec![],
                profile_path: None,
                webhook_url: None,
                launch_overrides: Default::default(),
                name: Some(format!("my-agent-{name}")),
                local_alias_id: None,
            })
            .collect();

        // Step 2: 6 additive-layer specs (what resolve_agent_specs appends).
        // These have the short profile name as agent_id and zellij_session set.
        let additive_specs: Vec<AgentSpec> = profile_names
            .iter()
            .map(|name| AgentSpec {
                agent_id: name.to_string(),
                domain_id: "d-1".to_string(),
                runtime_kind: "zellij_hosted".to_string(),
                session_mode: SessionMode::Persistent,
                capabilities: vec![],
                profile_path: None,
                webhook_url: None,
                launch_overrides: crate::supervisor::SpawnOverrides {
                    zellij_session: Some(format!("my-agent-{name}")),
                    ..Default::default()
                },
                name: None,
                local_alias_id: None,
            })
            .collect();

        // Combine: 12 specs, as produced by resolve_agent_specs in federated mode.
        let mut agent_specs: Vec<AgentSpec> = cp_specs;
        agent_specs.extend(additive_specs);
        assert_eq!(agent_specs.len(), 12, "pre-fix: 12 specs before dedup");

        // Step 3: run merge_federated_overrides (boot-time merge, Gap 2 fix).
        let local_ctxs: Vec<AgentLaunchContext> = profile_names
            .iter()
            .map(|name| AgentLaunchContext {
                source: "fleet".to_string(),
                agent_id: name.to_string(),
                vault_folder: Some(name.to_string()),
                state_dir_spec: None,
                zellij_session: Some(format!("my-agent-{name}")),
                herdr_session: None,
                systemd_service: Some(format!("my-agent-{name}.service")),
                supervise_paused: false,
            })
            .collect();
        merge_federated_overrides(&mut agent_specs, &local_ctxs);

        // Step 4: boot dedup (Gap 3 fix) — collect aliased ids (owned Strings
        // so the immutable borrow on agent_specs ends before the mutable
        // retain call) and retain only specs whose agent_id is NOT aliased.
        let aliased_ids: std::collections::HashSet<String> = agent_specs
            .iter()
            .filter_map(|s| s.local_alias_id.clone())
            .collect();
        agent_specs.retain(|s| !aliased_ids.contains(&s.agent_id));
        assert_eq!(
            agent_specs.len(),
            6,
            "post-dedup: exactly 6 specs (one per agent)"
        );

        // Step 5 + 6: diff against empty running → exactly 6 to_spawn.
        let plan = diff_specs(&agent_specs, &HashMap::new());
        assert_eq!(
            plan.to_spawn.len(),
            6,
            "exactly 6 to_spawn after dedup; got {:?}",
            plan.to_spawn
                .iter()
                .map(|s| &s.agent_id)
                .collect::<Vec<_>>()
        );
        assert!(plan.to_remove.is_empty(), "no agents to remove at boot");
        assert!(plan.to_restart.is_empty(), "no agents to restart at boot");

        // Step 6: all to_spawn are the controlplane opaque ids (attach ids).
        for spec in &plan.to_spawn {
            assert!(
                spec.agent_id.starts_with("my-agent-"),
                "to_spawn must be opaque controlplane id, got '{}'",
                spec.agent_id
            );
        }

        // Step 7: no duplicate agent_ids.
        let ids: std::collections::HashSet<&str> =
            plan.to_spawn.iter().map(|s| s.agent_id.as_str()).collect();
        assert_eq!(
            ids.len(),
            plan.to_spawn.len(),
            "no duplicate agent_ids in to_spawn"
        );

        // Step 8: every to_spawn spec has local_alias_id set (needed for
        // supervisor alias registration so signal-by-name still works).
        for spec in &plan.to_spawn {
            assert!(
                spec.local_alias_id.is_some(),
                "to_spawn spec '{}' must carry local_alias_id for supervisor alias",
                spec.agent_id
            );
        }
    }

    /// Steady-state no-op: after federated boot spawns 6 agents under opaque
    /// ids, the next WS/poll cycle (also 6 opaque-id specs with aliases) must
    /// yield a no-op diff — no re-spawn, no remove.
    #[test]
    fn federated_steady_state_is_noop_after_boot_spawn() {
        use crate::daemon::AgentSpec;

        let profile_names = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"];

        // Running map: 6 agents keyed by opaque id (the result of the boot spawn).
        let mut running: HashMap<String, RunningAgent> = HashMap::new();
        for (i, name) in profile_names.iter().enumerate() {
            let opaque_id = format!("my-agent-{name}-{i:08x}");
            let spec = AgentSpec {
                agent_id: opaque_id.clone(),
                domain_id: "d-1".to_string(),
                runtime_kind: "zellij_hosted".to_string(),
                session_mode: SessionMode::Persistent,
                capabilities: vec![],
                profile_path: None,
                webhook_url: None,
                launch_overrides: crate::supervisor::SpawnOverrides {
                    zellij_session: Some(format!("my-agent-{name}")),
                    ..Default::default()
                },
                name: Some(format!("my-agent-{name}")),
                local_alias_id: Some(name.to_string()),
            };
            running.insert(
                opaque_id,
                RunningAgent {
                    spec,
                    handles: vec![],
                },
            );
        }

        // Desired: same 6 opaque-id specs (what persist_and_resolve_specs returns
        // on the WS/poll path after merge). Alias key == running key → no-op.
        let desired: Vec<AgentSpec> = running.values().map(|ra| ra.spec.clone()).collect();

        let plan = diff_specs(&desired, &running);
        assert!(
            plan.is_noop(),
            "steady-state poll must be a no-op; got: remove={:?} spawn={:?} restart={:?}",
            plan.to_remove,
            plan.to_spawn
                .iter()
                .map(|s| &s.agent_id)
                .collect::<Vec<_>>(),
            plan.to_restart
                .iter()
                .map(|s| &s.agent_id)
                .collect::<Vec<_>>(),
        );
    }
}
