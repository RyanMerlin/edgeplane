/// Process supervisor for agent runtimes.
///
/// Owns spawned AgentHandles and their associated runtimes.
/// Spawn once, track PID; restart policy is handled by the task loop.
use anyhow::Result;
use edgeplaned_core::agent_runtime::DynAgentRuntime;
use edgeplaned_core::types::{AgentHandle, LaunchContext, StateDirSpec};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Per-agent fields the daemon resolves from the local registry's
/// `AgentLaunchContext` row and forwards to the runtime via `LaunchContext`.
/// For agents that don't have a launch-context row (most controlplane-synced
/// task agents) all fields stay `None` and the runtime falls back to its
/// own defaults.
#[derive(Debug, Default, Clone)]
pub struct SpawnOverrides {
    pub vault_folder: Option<String>,
    pub state_dir_spec: Option<StateDirSpec>,
    pub zellij_session: Option<String>,
}

#[allow(dead_code)]
pub struct SupervisedAgent {
    pub agent_id: String,
    pub runtime: Arc<DynAgentRuntime>,
    pub handle: AgentHandle,
    pub domain_id: String,
}

pub struct Supervisor {
    agents: Mutex<HashMap<String, SupervisedAgent>>,
    /// Alias map: short name (e.g. `"engineer"`) → canonical opaque id
    /// (e.g. `"aria-engineer-708650f1"`). Used in federated mode so that
    /// `edgeplane agent signal engineer` still reaches the agent that was
    /// spawned under its controlplane public_id. Registered by the daemon
    /// after spawning a spec that carries a `local_alias_id`.
    name_aliases: Mutex<HashMap<String, String>>,
    work_dir: PathBuf,
    backend_url: String,
    token: String,
}

impl Supervisor {
    pub fn new(work_dir: PathBuf, backend_url: String, token: String) -> Self {
        Supervisor {
            agents: Mutex::new(HashMap::new()),
            name_aliases: Mutex::new(HashMap::new()),
            work_dir,
            backend_url,
            token,
        }
    }

    /// Launch an agent runtime and register it.
    pub async fn spawn(
        &self,
        agent_id: String,
        domain_id: String,
        runtime: Arc<DynAgentRuntime>,
        env: Vec<(String, String)>,
        overrides: SpawnOverrides,
    ) -> Result<()> {
        let work_dir = self.work_dir.join(&agent_id);
        std::fs::create_dir_all(&work_dir)?;

        let ctx = LaunchContext {
            agent_id: agent_id.clone(),
            domain_id: domain_id.clone(),
            work_dir,
            backend_url: self.backend_url.clone(),
            backend_token: self.token.clone(),
            env,
            // Profile and roster are injected per-task in the task loop, not at launch time.
            profile: None,
            roster: vec![],
            with_rtk: false,
            // Per-agent overrides resolved by the daemon from the local
            // registry's AgentLaunchContext row (when one exists).
            // Populated for fleet-imported ZellijHosted agents; default
            // `None` for everything else.
            vault_folder: overrides.vault_folder,
            state_dir_spec: overrides.state_dir_spec,
            zellij_session: overrides.zellij_session,
        };

        let handle = runtime.launch(ctx).await?;
        tracing::info!(
            "Spawned {} agent {} (pid {})",
            runtime.kind(),
            agent_id,
            handle.pid
        );

        let supervised = SupervisedAgent {
            agent_id: agent_id.clone(),
            runtime,
            handle,
            domain_id,
        };

        self.agents.lock().await.insert(agent_id, supervised);
        Ok(())
    }

    /// Return all agent ids currently supervised.
    #[allow(dead_code)]
    pub async fn agent_ids(&self) -> Vec<String> {
        self.agents.lock().await.keys().cloned().collect()
    }

    /// Register `alias` as an alternate lookup key resolving to `canonical`.
    ///
    /// Used in federated mode when an agent is spawned under its controlplane
    /// `public_id` (e.g. `"aria-engineer-708650f1"`) but callers (CLI, cron
    /// dispatcher) reference it by its short local profile name (e.g.
    /// `"engineer"`). After registration, `with_agent("engineer")` resolves
    /// the same as `with_agent("aria-engineer-708650f1")`.
    pub async fn register_name_alias(&self, alias: String, canonical: String) {
        self.name_aliases.lock().await.insert(alias, canonical);
    }

    /// Borrow a supervised agent by id, with name-alias fallback.
    ///
    /// Resolution order:
    /// 1. Direct lookup by `agent_id`.
    /// 2. Alias lookup: if `agent_id` matches a registered alias, resolve to
    ///    the canonical id and look that up.
    ///
    /// Returns `None` when neither lookup succeeds. The closure `f` receives
    /// a reference to the `SupervisedAgent` and its return value is
    /// propagated, so callers clone only what they need outside the lock.
    pub async fn with_agent<F, T>(&self, agent_id: &str, f: F) -> Option<T>
    where
        F: FnOnce(&SupervisedAgent) -> T,
    {
        // Resolve alias first (two separate lock acquisitions to avoid holding
        // name_aliases lock while acquiring agents lock — deadlock-safe since
        // no other code holds both simultaneously).
        let resolved_id: String = {
            let aliases = self.name_aliases.lock().await;
            match aliases.get(agent_id) {
                Some(canonical) => canonical.clone(),
                None => agent_id.to_owned(),
            }
        };
        self.agents.lock().await.get(&resolved_id).map(f)
    }
}
