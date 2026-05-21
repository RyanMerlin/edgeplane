/// mcd daemon — wires config, supervisor, runtimes, and task loops together.
use anyhow::Result;
use mcd_core::agent_runtime::AgentRuntime;
use mcd_core::capability_dispatcher::CapabilityDispatcher;
use mcd_core::client::BackendClient;
use mcd_core::machine::MachineInfo;
use mcd_core::paths;
use mcd_packs::{PackRegistry, PolicyBundle};
use mcd_runtimes::{
    claude_agent_acp::ClaudeAgentAcpRuntime,
    claude_code::ClaudeCodeRuntime,
    codex::CodexRuntime,
    gemini::GeminiRuntime,
    goose::GooseRuntime,
    zellij_hosted::ZellijHostedRuntime,
};
use mcd_receipts::ReceiptStore;
use mcd_work::watchdog::{OfflinePolicy, Watchdog};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::acp_session_supervisor::{self, AcpSupervisorConfig};
use crate::attach_gateway;
use crate::attach_registry::AttachRegistry;
use crate::attach_ws;
use crate::bootstrap;
use crate::config::{DaemonConfig, SessionMode};
use crate::fleet_import::SOURCE_FLEET_IMPORT;
use crate::local_registry::{LocalRegistry, SOURCE_LOCAL, source_cp};
use crate::mgmt_gateway::MgmtGateway;
use crate::reconcile::{self, RunningAgent, RunningAgents};
use crate::secrets_gateway::SecretsGateway;
use crate::session_supervisor;
use crate::state;
use crate::supervisor::{SpawnOverrides, Supervisor};
use crate::task_loop;

/// Config passed from the CLI, overrides any file-based config.
pub struct CliOverrides {
    pub backend_url: String,
    pub token: String,
    pub work_dir: PathBuf,
    pub offline_grace_secs: u64,
    /// If another mcd holds the singleton lock, terminate it and take over.
    /// SIGTERM, wait 5s, SIGKILL if still alive. Use only when the existing
    /// daemon is hung — otherwise prefer `systemctl --user restart mcd.service`.
    pub kill_existing: bool,
    /// Permit startup to continue when a required TCP port (attach_ws 8009,
    /// mgmt 7731) is in use. Default is fatal. Use only when you knowingly
    /// need partial functionality and are willing to lose attach/mgmt.
    pub allow_degraded: bool,
}

pub async fn run(cli: CliOverrides) -> Result<()> {
    // Singleton guard: acquire the kernel flock before any port binds, before
    // touching the registry, before reaching out to the controlplane. If
    // another mcd is running, this returns a structured error and we exit.
    // The lock is held for the lifetime of this function — released by the
    // kernel on any termination (clean exit, panic, SIGKILL, OOM).
    let lock_path = mcd_core::paths::lock_file_path();
    let _singleton = crate::singleton::SingletonLock::acquire(&lock_path, cli.kill_existing)?;
    tracing::info!("singleton lock acquired at {}", lock_path.display());

    let mut cfg = DaemonConfig::load_or_default();

    // Phase 5b/d: state file is the source of truth for node identity + active
    // controlplane profile. Priority: yaml → state file → CLI args (applied below).
    // Returns the active profile name for use as the SQLite source tag.
    let active_profile_name = match merge_state_file(&mut cfg).await {
        Ok(name) => name,
        Err(e) => {
            tracing::warn!("state file load failed: {e:#}. Continuing with yaml-only fields.");
            None
        }
    };

    // CLI args win over state file and yaml.
    if !cli.backend_url.is_empty() {
        cfg.backend_url = cli.backend_url;
    }
    if !cli.token.is_empty() {
        cfg.token = cli.token;
    }
    cfg.work_dir = cli.work_dir;
    cfg.offline_grace_secs = cli.offline_grace_secs;

    tracing::info!("mcd daemon starting");
    tracing::info!("backend: {}", cfg.backend_url);
    tracing::info!("work_dir: {}", cfg.work_dir.display());
    tracing::info!(
        "missions: {:?}",
        cfg.missions.iter().map(|m| &m.mission_id).collect::<Vec<_>>()
    );

    // Fail-fast port probe. The singleton lock already catches the dominant
    // case (another mcd holding these ports), but a third party could be
    // holding them. Probe → drop → let serve() re-bind. The TOCTOU window
    // between probe and re-bind is microseconds.
    probe_required_ports(&cfg, cli.allow_degraded).await?;

    std::fs::create_dir_all(&cfg.work_dir)?;

    // Phase 5a: open (or create) the local SQLite registry. Used in both
    // standalone mode (source of truth) and federated mode (synced cache).
    // On failure: log and continue — federated still works, standalone falls
    // back to legacy yaml missions.
    let registry: Option<LocalRegistry> = LocalRegistry::default_path()
        .and_then(|p| {
            tracing::info!("local registry: {}", p.display());
            LocalRegistry::open(&p)
        })
        .map_err(|e| {
            tracing::warn!(
                "Could not open local registry: {e:#}. \
                 Standalone mode will fall back to yaml missions."
            );
            e
        })
        .ok();

    // Phase 1 daemon-absorption: idempotent one-time import of Aria-style
    // fleet-profiles.toml into the local registry as ZellijHosted agents.
    // Runs on every startup so newly-added profiles get picked up. Missing
    // manifest is not an error — most nodes don't run the Aria fleet.
    if let Some(reg) = registry.as_ref() {
        let manifest_path = crate::fleet_import::resolve_manifest_path(
            cfg.fleet_profiles_file.as_deref(),
        );
        match manifest_path {
            Some(path) => match crate::fleet_import::load_profiles(&path)
                .and_then(|profiles| crate::fleet_import::import_into(reg, &profiles))
            {
                Ok(summary) => tracing::info!(
                    "fleet import from {}: {} created, {} updated, {} total",
                    path.display(),
                    summary.created,
                    summary.updated,
                    summary.total,
                ),
                Err(e) => tracing::warn!(
                    "fleet import from {} failed: {e:#}. Continuing without it.",
                    path.display(),
                ),
            },
            None => tracing::debug!("no fleet-profiles.toml to import (this is fine)"),
        }
    }

    let client = Arc::new(BackendClient::new(&cfg.backend_url, &cfg.token));

    // Bootstrap: idempotently provision `home-{hostname}` mission + `intake`
    // kluster for per-node coordination. Runs after fleet_import (which
    // establishes the agent registry) and after the client is constructed.
    // Soft-fail: controlplane unreachable is logged as a warning; mcd continues.
    match bootstrap::run(&client).await {
        Ok(summary) => {
            if summary.mission_created || summary.kluster_created {
                tracing::info!(
                    "bootstrap: provisioned fleet ops mission={} intake kluster={} \
                     (mission_created={}, kluster_created={})",
                    summary.mission_id,
                    summary.kluster_id,
                    summary.mission_created,
                    summary.kluster_created,
                );
            } else {
                tracing::debug!(
                    "bootstrap: fleet ops mission and intake kluster already exist \
                     (mission={}, kluster={})",
                    summary.mission_id,
                    summary.kluster_id,
                );
            }
        }
        Err(e) => {
            // run() itself is soft-fail and returns Ok on connectivity errors,
            // but handle the unexpected Err case for completeness.
            tracing::warn!("bootstrap: unexpected error: {e:#}. Continuing.");
        }
    }

    let policy = match cfg.offline_policy.as_str() {
        "safe_readonly" => OfflinePolicy::SafeReadonly,
        "autonomous" => OfflinePolicy::Autonomous { max_ttl_secs: 300 },
        _ => OfflinePolicy::Strict,
    };
    let watchdog = Arc::new(Watchdog::new(policy, cfg.offline_grace_secs));

    let supervisor = Arc::new(Supervisor::new(
        cfg.work_dir.clone(),
        cfg.backend_url.clone(),
        cfg.token.clone(),
    ));

    // Runtime map for the attach gateway: agent_id → runtime
    let runtime_map: attach_gateway::RuntimeMap =
        Arc::new(Mutex::new(HashMap::new()));

    // Process-wide registry of live persistent-session endpoints.
    // Populated by `session_supervisor`; consumed by attach gateway and the
    // network attach WS server (Phase 2a).
    let attach_registry = AttachRegistry::new();

    let mut task_handles = vec![];

    // Phase 4d: per-agent registry tracking running supervisors. The WS
    // subscriber and the periodic poll fall through `reconcile::diff_specs`
    // → apply against this map so individual agents can be cycled without
    // restarting the daemon.
    let running: RunningAgents = Arc::new(Mutex::new(HashMap::new()));

    let spawner = Arc::new(Spawner {
        client: Arc::clone(&client),
        watchdog: Arc::clone(&watchdog),
        supervisor: Arc::clone(&supervisor),
        runtime_map: Arc::clone(&runtime_map),
        attach_registry: Arc::clone(&attach_registry),
    });

    // Phase 4c/5a: build the flat list of agents to spawn.
    // Priority: controlplane (federated) > SQLite local registry > yaml legacy.
    let agent_specs = resolve_agent_specs(&cfg, &client, registry.as_ref()).await;

    // Initial spawn through the same reconcile path the WS subscriber will
    // use later — keeps both paths exercising one code branch.
    {
        let mut running_lock = running.lock().await;
        let plan = reconcile::diff_specs(&agent_specs, &running_lock);
        spawner.apply_plan(&plan, &mut running_lock).await;
    }

    // Phase 5d: compute the SQLite source tag for the active controlplane profile.
    // WS/poll loops write fetched specs here; the reconciler always reads from SQLite.
    let cp_source: Option<String> = active_profile_name.as_deref().map(source_cp);

    // Phase 4d/5d: live reassignment via WS + poll fallback. Only meaningful
    // when this node is registered with the controlplane; without a node_id
    // there is no /runtime/nodes/{id}/notify subscription to make.
    if let Some(node_id) = cfg.node_id.clone() {
        let ws_backend = cfg.backend_url.clone();
        let ws_token = cfg.token.clone();
        let ws_running = Arc::clone(&running);
        let ws_client = Arc::clone(&client);
        let ws_spawner = Arc::clone(&spawner);
        let ws_cp_source = cp_source.clone();
        task_handles.push(tokio::spawn(reconcile::watch_assignments_ws(
            ws_backend,
            ws_token,
            node_id.clone(),
            ws_client,
            ws_running,
            move |specs, running| {
                let spawner = Arc::clone(&ws_spawner);
                let source = ws_cp_source.clone();
                async move {
                    let resolved = persist_and_resolve_specs(specs, source.as_deref()).await;
                    let mut lock = running.lock().await;
                    let plan = reconcile::diff_specs(&resolved, &lock);
                    if !plan.is_noop() {
                        tracing::info!(
                            "WS reconcile: spawn={}, restart={}, remove={}",
                            plan.to_spawn.len(),
                            plan.to_restart.len(),
                            plan.to_remove.len()
                        );
                    }
                    spawner.apply_plan(&plan, &mut lock).await;
                }
            },
        )));

        let poll_running = Arc::clone(&running);
        let poll_client = Arc::clone(&client);
        let poll_spawner = Arc::clone(&spawner);
        let poll_cp_source = cp_source.clone();
        task_handles.push(tokio::spawn(reconcile::poll_assignments(
            poll_client,
            node_id.clone(),
            poll_running,
            move |specs, running| {
                let spawner = Arc::clone(&poll_spawner);
                let source = poll_cp_source.clone();
                async move {
                    let resolved = persist_and_resolve_specs(specs, source.as_deref()).await;
                    let mut lock = running.lock().await;
                    let plan = reconcile::diff_specs(&resolved, &lock);
                    if !plan.is_noop() {
                        tracing::info!(
                            "Poll reconcile: spawn={}, restart={}, remove={}",
                            plan.to_spawn.len(),
                            plan.to_restart.len(),
                            plan.to_remove.len()
                        );
                    }
                    spawner.apply_plan(&plan, &mut lock).await;
                }
            },
        )));
    }

    if running.lock().await.is_empty() && task_handles.is_empty() {
        tracing::warn!(
            "No agents assigned. Either enroll agents to this node via \
             `mc daemon agent enroll` (controlplane-driven) or add legacy \
             `missions:` entries to {} (deprecated path).",
            DaemonConfig::user_config_path().display()
        );
    }

    // If the daemon has a registered node_id, send periodic node heartbeats
    // to mc-controlplane with current Tailscale info.
    if let Some(node_id) = cfg.node_id.clone() {
        let heartbeat_client = Arc::clone(&client);
        let heartbeat_work_dir = cfg.work_dir.clone();
        tokio::spawn(async move {
            const NODE_HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
            loop {
                tokio::time::sleep(NODE_HEARTBEAT_INTERVAL).await;
                let info = MachineInfo::detect(&heartbeat_work_dir);
                let body = serde_json::json!({
                    "status": "online",
                    "tailscale_ip": info.tailscale_ip,
                    "tailscale_fqdn": info.tailscale_fqdn,
                });
                if let Err(e) = heartbeat_client
                    .raw_post(&format!("/runtime/nodes/{node_id}/heartbeat"), &body)
                    .await
                {
                    tracing::warn!("Node heartbeat failed for {node_id}: {e}");
                } else {
                    tracing::debug!("Node heartbeat sent for {node_id}");
                }
            }
        });
    }

    // Start the attach gateway in the background.
    let gw_map = Arc::clone(&runtime_map);
    let gw_registry = Arc::clone(&attach_registry);
    tokio::spawn(async move {
        if let Err(e) = attach_gateway::run(gw_map, gw_registry).await {
            tracing::warn!("attach gateway exited: {e}");
        }
    });

    // Network-facing attach WS server. Bound to the Tailscale interface in
    // production via `attach_bind_addr`. The controlplane proxies browser
    // attach upgrades here over Tailscale (Phase 2b).
    {
        let ws_registry = Arc::clone(&attach_registry);
        let ws_addr = cfg.attach_bind_addr.clone();
        let ws_secret = cfg.attach_secret.clone();
        tokio::spawn(async move {
            if let Err(e) = attach_ws::serve(ws_addr, ws_secret, ws_registry).await {
                tracing::warn!("attach_ws server exited: {e:#}");
            }
        });
    }

    // Create the session store shared between the secrets gateway and the dispatcher.
    let session_store = Arc::new(mcd_secrets::SessionStore::new());
    let secrets_socket = paths::secrets_socket_path();

    // Start the secrets gateway (broker for agent credential requests).
    {
        let gw = SecretsGateway::new(Arc::clone(&session_store), secrets_socket.clone());
        tokio::spawn(async move {
            if let Err(e) = gw.run().await {
                tracing::error!("secrets gateway error: {e}");
            }
        });
    }

    // Start the management gateway (Unix socket + TCP, JSON-RPC 2.0).
    {
        let registry = Arc::new(match PackRegistry::load_builtin() {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("failed to load builtin pack registry: {e}");
                return Err(anyhow::anyhow!("pack registry load failed: {e}"));
            }
        });
        let receipts_path = paths::receipts_db_path();
        let receipt_store = ReceiptStore::open(&receipts_path)
            .map_err(|e| anyhow::anyhow!("failed to open receipt store at {}: {e}", receipts_path.display()))?;
        let dispatcher = Arc::new(
            CapabilityDispatcher::new(Arc::clone(&registry), PolicyBundle::allow_all(), None)
                .with_receipt_store(Arc::new(receipt_store))
                .with_session_store(Arc::clone(&session_store), secrets_socket),
        );
        // Phase 3 daemon-absorption: wire the supervisor + runtime_map +
        // registry path so the new `agent.local.*` and
        // `agent.describe_local` JSON-RPC methods can answer queries about
        // locally-supervised agents. Registry path falls back to the
        // default location if discovery fails — the agent.* handlers
        // return a structured "registry read failed" error in that case.
        let registry_path = crate::local_registry::LocalRegistry::default_path()
            .unwrap_or_else(|_| mcd_core::paths::registry_db_path());

        // Phase 4: spawn the cron tick loop + GC task. CronHandle is
        // threaded into AgentOpsHandle so the `agent.cron.reload` JSON-RPC
        // method can poke the loop without holding any Mutex.
        let cron_config_path = crate::cron_config::resolve_path(None);
        let (cron_loop, cron_handle) = crate::cron::CronLoop::new(
            cron_config_path.clone(),
            Arc::clone(&supervisor),
            Arc::clone(&runtime_map),
            registry_path.clone(),
        );
        let cron_config_for_gc = cron_loop.config_for_gc();
        let registry_path_for_gc = registry_path.clone();
        tokio::spawn(async move { cron_loop.run().await });
        tokio::spawn(async move {
            crate::cron::gc_task(cron_config_for_gc, registry_path_for_gc).await
        });

        // Phase 5: spawn the unit-health loop + its GC task. Broadcast
        // channel for SupervisorEvents — buffer 256 so a slow subscriber
        // doesn't block the supervisor. Future TUI/WS surfaces subscribe;
        // for Phase 5 itself, persistent history lives in unit_restart_log.
        let (supervisor_events_tx, _) =
            tokio::sync::broadcast::channel::<mcd_core::types::SupervisorEvent>(256);
        let unit_health_loop = crate::unit_health::UnitHealthLoop::new(
            registry_path.clone(),
            crate::unit_health::UnitHealthConfig::default(),
            supervisor_events_tx.clone(),
        );
        let unit_gc_registry_path = registry_path.clone();
        tokio::spawn(async move { unit_health_loop.run().await });
        tokio::spawn(async move {
            crate::unit_health::gc_task(
                unit_gc_registry_path,
                30, // history_days — match cron defaults
                500, // max_rows_per_agent
                60,  // gc interval minutes
            )
            .await
        });

        let agent_ops = crate::mgmt_gateway::AgentOpsHandle {
            supervisor: Arc::clone(&supervisor),
            runtime_map: Arc::clone(&runtime_map),
            registry_path,
            cron: Some(cron_handle),
            cron_config_path: Some(cron_config_path),
            supervisor_events: Some(supervisor_events_tx),
        };
        let mgmt_gw = MgmtGateway::new(dispatcher, registry).with_agent_ops(agent_ops);
        tokio::spawn(async move {
            if let Err(e) = mgmt_gw.run().await {
                tracing::error!("mgmt gateway error: {e}");
            }
        });
    }

    // Wait for ctrl-c or all daemon-level loops to exit.
    // In standalone/yaml mode task_handles is empty (agents run as detached
    // tokio tasks in `running`), so we must not treat an empty vec as "done".
    if task_handles.is_empty() {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Received Ctrl-C, shutting down");
    } else {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received Ctrl-C, shutting down");
            }
            _ = async {
                for h in task_handles {
                    let _ = h.await;
                }
            } => {
                tracing::info!("All task loops exited");
            }
        }
    }

    // Clean up sockets on exit.
    let _ = std::fs::remove_file(attach_gateway::socket_path());
    let _ = std::fs::remove_file(paths::secrets_socket_path());
    Ok(())
}

/// Merge `~/.mc/state.json` (v2: profiles map + active_profile) into
/// `cfg`. Priority: yaml → state file. CLI args are applied by the caller
/// after this returns so they always win.
///
/// Returns the active profile name so the caller can compute the SQLite
/// source tag (`"controlplane:<name>"`).
///
/// On first run with a yaml that carries legacy `node_id`/`attach_secret`,
/// migrates those fields into a "default" profile and writes it back so
/// the next start uses the v2 state file directly.
async fn merge_state_file(cfg: &mut DaemonConfig) -> Result<Option<String>> {
    let path = state::DaemonState::default_path()?;
    let existing = state::DaemonState::read(&path)?;

    match existing {
        Some(s) => {
            match s.active() {
                Some((name, profile)) => {
                    // Active profile wins over yaml for all identity fields.
                    if let (Some(yaml_node_id), Some(profile_node_id)) =
                        (cfg.node_id.as_ref(), profile.node_id.as_ref())
                    {
                        if yaml_node_id != profile_node_id {
                            tracing::warn!(
                                "yaml has node_id={yaml_node_id} but active state profile has {}; \
                                 state wins. Remove node_id from yaml.",
                                profile_node_id
                            );
                        }
                    }
                    if cfg.attach_secret.is_some() {
                        tracing::warn!(
                            "yaml carries an `attach_secret`; state file is the source of truth — \
                             remove from yaml. (Daemon does not log secret values.)"
                        );
                    }
                    cfg.node_id = profile.node_id.clone();
                    cfg.attach_secret = Some(profile.attach_secret.clone());
                    cfg.backend_url = profile.url.clone();
                    if !profile.auth.token.is_empty() {
                        cfg.token = profile.auth.token.clone();
                    }
                    return Ok(Some(name.to_owned()));
                }
                None => {
                    // Profiles map exists but no active profile → standalone mode.
                    tracing::info!(
                        "State file has no active profile; daemon running in standalone mode. \
                         Use `mc daemon use <profile>` to select a controlplane."
                    );
                }
            }
        }
        None => {
            // No state file. If yaml carries legacy identity fields, migrate them
            // so the next start uses the v2 state file directly.
            if let (Some(node_id), Some(secret)) = (cfg.node_id.clone(), cfg.attach_secret.clone()) {
                tracing::warn!(
                    "Migrating node_id + attach_secret from yaml to state file at {}. \
                     Remove these fields from your config.yaml — a future release will hard-fail on them.",
                    path.display()
                );
                let mut profiles = std::collections::HashMap::new();
                profiles.insert(
                    "default".into(),
                    state::ProfileEntry {
                        url: cfg.backend_url.clone(),
                        auth: state::ProfileAuth::oidc(""),
                        node_id: Some(node_id),
                        attach_secret: secret,
                        registered_at: chrono::Utc::now().to_rfc3339(),
                        tailscale_fqdn: None,
                    },
                );
                let migrated = state::DaemonState {
                    schema_version: state::STATE_SCHEMA_VERSION,
                    active_profile: Some("default".into()),
                    profiles,
                };
                if let Err(e) = migrated.write_atomic(&path) {
                    tracing::warn!(
                        "Could not write migrated state file: {e:#}. Will re-migrate next start."
                    );
                }
                return Ok(Some("default".to_owned()));
            } else {
                tracing::info!(
                    "No state file at {} and no node_id in yaml; daemon running in standalone mode. \
                     Run `mc daemon profile add` to register with a controlplane, \
                     or `mc daemon agent enroll` to add agents in standalone mode.",
                    path.display()
                );
            }
        }
    }
    Ok(None)
}

// ── Phase 5d: SQLite-backed reconciler input ─────────────────────────────────

/// Write controlplane-fetched `specs` to SQLite (source = `cp_source`), then
/// read them back so the reconciler always reads from the local registry.
///
/// On any SQLite error the function falls back to `specs` directly — the WS/
/// poll loops degrade gracefully rather than stopping reconciliation entirely.
async fn persist_and_resolve_specs(
    specs: Vec<AgentSpec>,
    cp_source: Option<&str>,
) -> Vec<AgentSpec> {
    let source = match cp_source {
        Some(s) => s.to_owned(),
        None => return specs, // no active profile — pass through unchanged
    };
    let db_path = match LocalRegistry::default_path() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("persist_and_resolve: registry path unavailable: {e:#}. Using in-memory specs.");
            return specs;
        }
    };

    let source_w = source.clone();
    let specs_w = specs.clone();
    let db_path_w = db_path.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        LocalRegistry::replace_source(&db_path_w, &source_w, &specs_w)
    })
    .await
    .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking panicked: {e}")))
    {
        tracing::warn!("persist_and_resolve: write failed: {e:#}. Using in-memory specs.");
        return specs;
    }

    match tokio::task::spawn_blocking(move || {
        LocalRegistry::open(&db_path)?.list_specs_by_source(&source)
    })
    .await
    {
        Ok(Ok(db_specs)) => db_specs,
        Ok(Err(e)) => {
            tracing::warn!("persist_and_resolve: read back failed: {e:#}. Using in-memory specs.");
            specs
        }
        Err(e) => {
            tracing::warn!("persist_and_resolve: read task panicked: {e:#}. Using in-memory specs.");
            specs
        }
    }
}

// ── Phase 4d: per-agent spawner + reconcile-driven apply ─────────────────────

/// Bundles the shared deps every per-agent spawn needs. Cloned/Arc-shared
/// across the start-time path, the WS subscriber, and the poll fallback so
/// they all use the same spawn flow.
pub(crate) struct Spawner {
    pub client: Arc<BackendClient>,
    pub watchdog: Arc<mcd_work::watchdog::Watchdog>,
    pub supervisor: Arc<Supervisor>,
    pub runtime_map: attach_gateway::RuntimeMap,
    pub attach_registry: Arc<AttachRegistry>,
}

impl Spawner {
    /// Apply the diff plan against the running map: shut down removed
    /// agents, restart changed ones, spawn new ones. Holds the running
    /// lock for the duration so concurrent reconciles don't race; the
    /// alternative (per-agent locking) wasn't worth the complexity for
    /// the small fleet sizes we expect.
    pub async fn apply_plan(
        self: &Arc<Self>,
        plan: &reconcile::ReconcilePlan,
        running: &mut HashMap<String, RunningAgent>,
    ) {
        // 1. Shut down removed agents first (frees up runtime_map slots,
        //    SIGKILLs child processes via supervisor's kill_on_drop).
        for id in &plan.to_remove {
            if let Some(ra) = running.remove(id) {
                tracing::info!("Reconcile: shutting down agent {id}");
                ra.shutdown().await;
                self.runtime_map.lock().await.remove(id);
            }
        }
        // 2. Restart changed agents (shutdown old, spawn new).
        for spec in &plan.to_restart {
            if let Some(ra) = running.remove(&spec.agent_id) {
                tracing::info!(
                    "Reconcile: restarting agent {} (mission={}, mode={:?})",
                    spec.agent_id,
                    spec.mission_id,
                    spec.session_mode
                );
                ra.shutdown().await;
                self.runtime_map.lock().await.remove(&spec.agent_id);
            }
            if let Some(new) = self.spawn_one(spec).await {
                running.insert(spec.agent_id.clone(), new);
            }
        }
        // 3. Spawn newly-assigned agents.
        for spec in &plan.to_spawn {
            if let Some(new) = self.spawn_one(spec).await {
                running.insert(spec.agent_id.clone(), new);
            }
        }
    }

    /// Spawn one agent according to its spec. Returns `None` if any
    /// pre-flight step fails (work_dir create, ensure_installed, etc.) —
    /// the caller logs and continues. Mirrors the inline behavior the
    /// legacy spawn loop had, just refactored for callability.
    pub async fn spawn_one(self: &Arc<Self>, spec: &AgentSpec) -> Option<RunningAgent> {
        // Persistent agents with a webhook_url are already running as
        // systemd/tmux sessions. We relay messages to them via HTTP webhook
        // instead of spawning a competing ACP process.
        if spec.session_mode == SessionMode::Persistent {
            if let Some(ref webhook_url) = spec.webhook_url {
                tracing::info!(
                    "Agent {} is persistent with webhook_url={webhook_url}; \
                     using webhook relay (no ACP spawn)",
                    spec.agent_id
                );
                let relay_jh = tokio::spawn(task_loop::run_webhook_relay(
                    self.client.clone(),
                    spec.agent_id.clone(),
                    webhook_url.clone(),
                ));
                return Some(RunningAgent::new(spec.clone(), vec![relay_jh]));
            }
        }

        let extra_caps: Vec<mcd_core::types::Capability> = spec
            .capabilities
            .iter()
            .map(|s| mcd_core::types::Capability::new(s.clone()))
            .collect();
        let work_dir = paths::mcd_work_dir().join(&spec.agent_id);

        let mut acp_spawn_opts: Option<mcd_acp::SpawnOpts> = None;

        let rt: Arc<mcd_core::agent_runtime::DynAgentRuntime> = match spec
            .runtime_kind
            .as_str()
        {
            "claude_code" => Arc::new(Box::new(ClaudeCodeRuntime::with_extra_capabilities(
                extra_caps,
            ))),
            "claude_agent_acp" => {
                let concrete = ClaudeAgentAcpRuntime::with_extra_capabilities(extra_caps);
                if let Err(e) = std::fs::create_dir_all(&work_dir) {
                    tracing::error!("failed to create work dir for {}: {e}", spec.agent_id);
                    return None;
                }
                if let Err(e) = concrete.ensure_installed().await {
                    tracing::error!(
                        "ensure_installed failed for ACP agent {}: {e:#}. Skipping.",
                        spec.agent_id
                    );
                    return None;
                }
                match concrete.spawn_opts(&work_dir) {
                    Ok(opts) => acp_spawn_opts = Some(opts),
                    Err(e) => {
                        tracing::error!(
                            "could not resolve ACP spawn opts for {}: {e:#}. Skipping.",
                            spec.agent_id
                        );
                        return None;
                    }
                }
                Arc::new(Box::new(concrete))
            }
            "codex" => Arc::new(Box::new(CodexRuntime::with_extra_capabilities(extra_caps))),
            "gemini" => Arc::new(Box::new(GeminiRuntime::with_extra_capabilities(extra_caps))),
            "goose" => Arc::new(Box::new(GooseRuntime::with_extra_capabilities(extra_caps))),
            "zellij_hosted" => Arc::new(Box::new(
                ZellijHostedRuntime::with_extra_capabilities(extra_caps),
            )),
            other => {
                tracing::warn!(
                    "Unknown runtime kind '{other}', skipping agent {}",
                    spec.agent_id
                );
                return None;
            }
        };

        if let Err(e) = rt.ensure_installed().await {
            tracing::error!(
                "ensure_installed failed for agent {} (runtime {}): {e:#}. Skipping.",
                spec.agent_id,
                spec.runtime_kind
            );
            return None;
        }

        // Register in the attach gateway's runtime map.
        self.runtime_map
            .lock()
            .await
            .insert(spec.agent_id.clone(), rt.clone());

        if let Err(e) = self
            .supervisor
            .spawn(
                spec.agent_id.clone(),
                spec.mission_id.clone(),
                rt.clone(),
                vec![],
                spec.launch_overrides.clone(),
            )
            .await
        {
            tracing::error!(
                "supervisor.spawn failed for {}: {e:#}. Skipping.",
                spec.agent_id
            );
            return None;
        }

        let agent_handle = Arc::new(Mutex::new(mcd_core::types::AgentHandle {
            agent_id: spec.agent_id.clone(),
            runtime_kind: rt.kind(),
            pid: 0,
        }));

        let mut handles: Vec<tokio::task::JoinHandle<()>> = vec![];

        match spec.session_mode {
            SessionMode::Task => {
                let h = tokio::spawn(task_loop::run_for_agent(
                    agent_handle,
                    rt.clone(),
                    self.client.clone(),
                    spec.mission_id.clone(),
                    spec.agent_id.clone(),
                    self.watchdog.clone(),
                ));
                // task_loop returns Result<(), _>; wrap so JoinHandle<()> matches.
                handles.push(tokio::spawn(async move {
                    let _ = h.await;
                }));
            }
            SessionMode::Persistent => {
                // ZellijHosted agents are externally managed: their Zellij
                // session is owned by systemd + aria-watchdog, mcd doesn't
                // need a session supervisor or message-relay loop. Signals
                // route via the mgmt_gateway `agent.local.signal` method
                // (Phase 3), which looks the agent up in the supervisor's
                // map (populated above by `supervisor.spawn`). No loops
                // needed; the agent record is the entire surface.
                if spec.runtime_kind == "zellij_hosted" {
                    tracing::info!(
                        "ZellijHosted agent {} registered; no session supervisor needed \
                         (signals route via mgmt_gateway agent.local.signal)",
                        spec.agent_id
                    );
                    return Some(RunningAgent::new(spec.clone(), handles));
                }

                let supervisor_jh = if spec.runtime_kind == "claude_agent_acp" {
                    let opts = acp_spawn_opts
                        .clone()
                        .expect("acp_spawn_opts populated when runtime_kind == claude_agent_acp");
                    let session_cwd = spec.profile_path.clone().unwrap_or(work_dir.clone());
                    let scfg = AcpSupervisorConfig {
                        agent_id: spec.agent_id.clone(),
                        spawn_opts: opts,
                        cwd: session_cwd,
                        // agent_id is the public_id (e.g. "aria-work-708650f1");
                        // using it as the remote-control prefix makes the ACP
                        // session visible in the Claude app under that name.
                        remote_control_prefix: Some(spec.agent_id.clone()),
                    };
                    tokio::spawn(acp_session_supervisor::run_for_agent(
                        scfg,
                        self.attach_registry.clone(),
                    ))
                } else {
                    tokio::spawn(session_supervisor::run_for_agent(
                        spec.agent_id.clone(),
                        rt.clone(),
                        self.attach_registry.clone(),
                    ))
                };
                handles.push(supervisor_jh);

                let relay_agent = agent_handle.clone();
                let relay_runtime = rt.clone();
                let relay_client = self.client.clone();
                let relay_agent_id = spec.agent_id.clone();
                let relay_registry = self.attach_registry.clone();
                let relay_jh = tokio::spawn(async move {
                    task_loop::run_message_relay(
                        relay_agent,
                        relay_runtime,
                        relay_client,
                        relay_agent_id,
                        Some(relay_registry),
                        None,
                    )
                    .await;
                });
                handles.push(relay_jh);
            }
        }

        tracing::info!(
            "Spawned {} loop for {} agent {} in mission {}",
            match spec.session_mode {
                SessionMode::Task => "task",
                SessionMode::Persistent => "persistent-session",
            },
            spec.runtime_kind,
            spec.agent_id,
            spec.mission_id
        );

        Some(RunningAgent::new(spec.clone(), handles))
    }
}

// ── Phase 4c: agent assignment resolution ────────────────────────────────────

/// Internal flat representation of one agent the daemon should spawn.
/// Built from either the controlplane (preferred when state.node_id is set)
/// or yaml-defined missions (legacy fallback during the deprecation window).
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub agent_id: String,
    pub mission_id: String,
    pub runtime_kind: String,
    pub session_mode: SessionMode,
    pub capabilities: Vec<String>,
    pub profile_path: Option<PathBuf>,
    /// HTTP endpoint to POST messages to (instead of spawning an ACP process).
    /// Set when the agent's `machine.webhook_url` is populated in the controlplane.
    pub webhook_url: Option<String>,
    /// Per-agent launch-context overrides resolved from the local registry's
    /// `agent_launch_context` table (Phase 1+). Empty default for agents
    /// without a row; populated for fleet-imported ZellijHosted agents and
    /// any future runtime that needs declarative launch parameters.
    pub launch_overrides: SpawnOverrides,
}

/// Build the initial spawn list.
///
/// Priority order for the "base" specs (one of these three is selected):
/// 1. Controlplane GET (federated — when `cfg.node_id` is set).
/// 2. Local SQLite registry (`source = 'local'`) — standalone mode.
/// 3. Legacy yaml `missions:` — deprecated fallback for pre-Phase-4 configs.
///
/// **Additive layer (always on when a registry is present):**
/// fleet-imported agents (`source = 'fleet_import'`) are appended to whatever
/// the base path returns. These represent the local Aria fleet (operator,
/// work, etc.) — they coexist with controlplane assignments and yaml legacy
/// missions, not replace them. Each fleet_import spec gets its
/// `launch_overrides` populated from the `agent_launch_context` table so
/// the runtime knows which Zellij session to address.
async fn resolve_agent_specs(
    cfg: &DaemonConfig,
    client: &BackendClient,
    registry: Option<&LocalRegistry>,
) -> Vec<AgentSpec> {
    let mut specs = base_agent_specs(cfg, client, registry).await;

    if let Some(reg) = registry {
        match reg.list_specs_by_source(SOURCE_FLEET_IMPORT) {
            Ok(mut fleet_specs) => {
                let n = fleet_specs.len();
                for spec in &mut fleet_specs {
                    if let Ok(Some(ctx)) =
                        reg.get_launch_context(SOURCE_FLEET_IMPORT, &spec.agent_id)
                    {
                        spec.launch_overrides = SpawnOverrides {
                            vault_folder: ctx.vault_folder,
                            state_dir_spec: ctx.state_dir_spec,
                            zellij_session: ctx.zellij_session,
                        };
                    }
                }
                if n > 0 {
                    tracing::info!("fleet_import: {n} agent(s) appended to spawn list");
                }
                specs.extend(fleet_specs);
            }
            Err(e) => {
                tracing::warn!(
                    "Could not list fleet_import agents from registry: {e:#}. \
                     Continuing without them."
                );
            }
        }
    }

    specs
}

/// Resolve the "base" agent list per the controlplane > local > yaml priority.
/// Separated from `resolve_agent_specs` so the fleet_import additive layer
/// can wrap it cleanly.
async fn base_agent_specs(
    cfg: &DaemonConfig,
    client: &BackendClient,
    registry: Option<&LocalRegistry>,
) -> Vec<AgentSpec> {
    if let Some(node_id) = cfg.node_id.as_deref() {
        match fetch_node_agents(client, node_id).await {
            Ok(specs) => {
                if !cfg.missions.is_empty() {
                    let yaml_missions: Vec<&str> =
                        cfg.missions.iter().map(|m| m.mission_id.as_str()).collect();
                    tracing::warn!(
                        "yaml carries `missions:` ({:?}) but node {} is registered with the \
                         controlplane; using controlplane assignment list. \
                         Remove `missions:` from config.yaml.",
                        yaml_missions,
                        node_id
                    );
                }
                tracing::info!(
                    "Resolved {} agent(s) from controlplane for node {}",
                    specs.len(),
                    node_id
                );
                return specs;
            }
            Err(e) => {
                tracing::error!(
                    "GET /runtime/nodes/{node_id}/agents failed: {e:#}. \
                     Falling back to local registry / yaml for this start."
                );
            }
        }
    }

    // Standalone mode — read from local SQLite registry first.
    if let Some(reg) = registry {
        match reg.list_specs_by_source(SOURCE_LOCAL) {
            Ok(specs) if !specs.is_empty() => {
                tracing::info!(
                    "Standalone mode: {} agent(s) from local registry \
                     (enroll via `mc daemon agent enroll`).",
                    specs.len()
                );
                return specs;
            }
            Ok(_) => {
                tracing::info!(
                    "Standalone mode: local registry is empty. \
                     Checking legacy yaml missions."
                );
            }
            Err(e) => {
                tracing::warn!("Could not read local registry: {e:#}. Falling back to yaml.");
            }
        }
    } else {
        tracing::info!(
            "No node_id in state file and no local registry; falling back to legacy yaml missions. \
             Run `mc daemon profile add` to register with a controlplane, \
             or `mc daemon agent enroll` to add agents in standalone mode."
        );
    }

    yaml_specs(cfg)
}

pub async fn fetch_node_agents(
    client: &BackendClient,
    node_id: &str,
) -> Result<Vec<AgentSpec>> {
    let path = format!("/runtime/nodes/{node_id}/agents");
    let rows: Vec<serde_json::Value> = client.get(&path).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        match agent_spec_from_json(row) {
            Ok(spec) => out.push(spec),
            Err(e) => tracing::warn!(
                "skipping malformed agent record from controlplane: {e:#} (raw={row})"
            ),
        }
    }
    Ok(out)
}

fn agent_spec_from_json(v: &serde_json::Value) -> Result<AgentSpec> {
    use anyhow::{Context, anyhow};
    // Wire identifier precedence:
    // 1. `public_id` (preferred — new shape, set by controlplane after the
    //    agent-public-id migration; matches the format used by the persistent
    //    agent registry and the `/agents/{public_id}/messages` route).
    // 2. `id` (fallback — pre-public_id controlplanes still emit only `id`).
    // Storing the resolved value as `AgentSpec.agent_id` keeps the local
    // registry, attach gateway, and message-poll URLs aligned.
    let agent_id = v
        .get("public_id")
        .and_then(|s| s.as_str())
        .or_else(|| v.get("id").and_then(|s| s.as_str()))
        .ok_or_else(|| anyhow!("agent record missing `public_id` (and `id` fallback)"))?
        .to_string();
    let mission_id = v
        .get("mission_id")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow!("agent {agent_id} missing `mission_id`"))
        .with_context(|| format!("agent_id={agent_id}"))?
        .to_string();
    let runtime_kind = v
        .get("runtime_kind")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow!("agent {agent_id} missing `runtime_kind`"))?
        .to_string();
    // supervision_mode is nullable in the schema; default to Task when
    // missing/unknown (the safer of the two — agents that need persistent
    // mode must be explicitly enrolled with it).
    let session_mode = match v.get("supervision_mode").and_then(|s| s.as_str()) {
        Some("persistent") => SessionMode::Persistent,
        Some("task") | None => SessionMode::Task,
        Some(other) => {
            tracing::warn!(
                "agent {} has unknown supervision_mode={other:?}; defaulting to task",
                agent_id
            );
            SessionMode::Task
        }
    };
    let capabilities = v
        .get("capabilities")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let profile_path = v
        .get("profile")
        .and_then(|p| p.get("path"))
        .and_then(|s| s.as_str())
        .map(PathBuf::from);
    let webhook_url = v
        .get("machine")
        .and_then(|m| m.get("webhook_url"))
        .and_then(|u| u.as_str())
        .filter(|u| !u.is_empty())
        .map(String::from);
    Ok(AgentSpec {
        agent_id,
        mission_id,
        runtime_kind,
        session_mode,
        capabilities,
        profile_path,
        webhook_url,
        launch_overrides: SpawnOverrides::default(),
    })
}

fn yaml_specs(cfg: &DaemonConfig) -> Vec<AgentSpec> {
    let mut out = Vec::new();
    for m in &cfg.missions {
        for a in &m.agents {
            out.push(AgentSpec {
                agent_id: a.agent_id.clone(),
                mission_id: m.mission_id.clone(),
                runtime_kind: a.runtime_kind.clone(),
                session_mode: a.session_mode,
                capabilities: a.capabilities.clone(),
                profile_path: a.profile_path.clone(),
                webhook_url: None,
                launch_overrides: SpawnOverrides::default(),
            });
        }
    }
    if !out.is_empty() {
        // Phase 6: yaml is the legacy bootstrap path. New deployments should
        // federate via `mc daemon profile add` or run standalone via
        // `mc daemon agent enroll-home`. The yaml path keeps working but won't
        // see future ergonomic improvements (auto-provisioning, sync loop, …).
        tracing::warn!(
            "Loaded {} agent(s) from legacy ~/.mc/config.yaml. \
             yaml-only configuration is deprecated. Migrate by running \
             `mc daemon profile add` (federated) or `mc daemon agent enroll-home` \
             (standalone) — see docs/plans/2026-05-10-mcd-phase6-home-mission-sync.md.",
            out.len()
        );
    }
    out
}

/// Try to bind every TCP port the daemon will use later, then immediately
/// drop the listener. If any required bind fails:
/// - default: return an error → daemon refuses to start
/// - `allow_degraded`: log a warning and continue (operator opt-in)
///
/// This is fail-fast belt to the singleton lock's suspenders. The singleton
/// lock blocks the dominant "two mcds" case, but a port conflict can still
/// arise if some unrelated process is on 8009 or 7731 — we want that loud,
/// not silently degraded.
async fn probe_required_ports(cfg: &DaemonConfig, allow_degraded: bool) -> Result<()> {
    let mgmt_port: u16 = std::env::var("MC_MESH_MGMT_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7731);
    let mgmt_addr = format!("0.0.0.0:{mgmt_port}");

    let probes: [(&str, &str); 2] = [
        ("attach_ws", cfg.attach_bind_addr.as_str()),
        ("mgmt_tcp", mgmt_addr.as_str()),
    ];

    for (name, addr) in probes {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                drop(listener);
                tracing::debug!("port probe ok: {name} ({addr})");
            }
            Err(e) if allow_degraded => {
                tracing::warn!(
                    "port probe failed for {name} at {addr}: {e}. \
                     Continuing because --allow-degraded was set."
                );
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "{name} port {addr} is already in use: {e}\n\n\
                     Another process is bound to a port mcd needs. To diagnose:\n  \
                       ss -lntp | grep -E ':({mgmt_port}|8009)\\b'\n  \
                       mcd doctor\n\n\
                     If you intentionally want partial startup, re-run with --allow-degraded."
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_spec_from_json_minimal() {
        let v = json!({
            "id": "a-1",
            "mission_id": "m-1",
            "runtime_kind": "claude_agent_acp",
        });
        let s = agent_spec_from_json(&v).unwrap();
        assert_eq!(s.agent_id, "a-1");
        assert_eq!(s.mission_id, "m-1");
        assert_eq!(s.runtime_kind, "claude_agent_acp");
        // Default mode when supervision_mode is absent.
        assert_eq!(s.session_mode, SessionMode::Task);
        assert!(s.capabilities.is_empty());
        assert!(s.profile_path.is_none());
    }

    #[test]
    fn agent_spec_from_json_full() {
        let v = json!({
            "id": "a-2",
            "mission_id": "m-1",
            "runtime_kind": "claude_agent_acp",
            "supervision_mode": "persistent",
            "capabilities": ["code.read", "code.edit"],
            "profile": { "path": "/home/x/profile" },
        });
        let s = agent_spec_from_json(&v).unwrap();
        assert_eq!(s.session_mode, SessionMode::Persistent);
        assert_eq!(s.capabilities, vec!["code.read", "code.edit"]);
        assert_eq!(s.profile_path.as_deref().unwrap().to_str().unwrap(), "/home/x/profile");
    }

    #[test]
    fn agent_spec_from_json_unknown_supervision_mode_defaults_task() {
        let v = json!({
            "id": "a-3",
            "mission_id": "m-1",
            "runtime_kind": "claude_code",
            "supervision_mode": "future-mode",
        });
        let s = agent_spec_from_json(&v).unwrap();
        assert_eq!(s.session_mode, SessionMode::Task);
    }

    #[test]
    fn agent_spec_from_json_missing_required_fields_errors() {
        // Missing id
        let v = json!({"mission_id": "m", "runtime_kind": "claude_code"});
        assert!(agent_spec_from_json(&v).is_err());
        // Missing mission_id
        let v = json!({"id": "a", "runtime_kind": "claude_code"});
        assert!(agent_spec_from_json(&v).is_err());
        // Missing runtime_kind
        let v = json!({"id": "a", "mission_id": "m"});
        assert!(agent_spec_from_json(&v).is_err());
    }
}
