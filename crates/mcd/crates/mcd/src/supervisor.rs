/// Process supervisor for agent runtimes.
///
/// Owns spawned AgentHandles and their associated runtimes.
/// Spawn once, track PID; restart policy is handled by the task loop.
use anyhow::Result;
use mcd_core::agent_runtime::DynAgentRuntime;
use mcd_core::types::{AgentHandle, LaunchContext, StateDirSpec};
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
    work_dir: PathBuf,
    backend_url: String,
    token: String,
}

impl Supervisor {
    pub fn new(work_dir: PathBuf, backend_url: String, token: String) -> Self {
        Supervisor {
            agents: Mutex::new(HashMap::new()),
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

    /// Borrow a supervised agent by id (clones the metadata for use outside the lock).
    pub async fn with_agent<F, T>(&self, agent_id: &str, f: F) -> Option<T>
    where
        F: FnOnce(&SupervisedAgent) -> T,
    {
        self.agents.lock().await.get(agent_id).map(f)
    }
}
