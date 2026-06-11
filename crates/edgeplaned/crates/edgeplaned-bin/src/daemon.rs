/// edgeplaned daemon — wires config, supervisor, runtimes, and task loops together.
use anyhow::Result;
use edgeplaned_core::agent_runtime::AgentRuntime;
use edgeplaned_core::capability_dispatcher::CapabilityDispatcher;
use edgeplaned_core::client::BackendClient;
use edgeplaned_core::machine::MachineInfo;
use edgeplaned_core::paths;
use edgeplaned_packs::{PackRegistry, PolicyBundle};
use edgeplaned_runtimes::{
    claude_agent_acp::ClaudeAgentAcpRuntime,
    claude_code::ClaudeCodeRuntime,
    codex::CodexRuntime,
    gemini::GeminiRuntime,
    goose::GooseRuntime,
    zellij_hosted::ZellijHostedRuntime,
};
use edgeplaned_receipts::ReceiptStore;
use edgeplaned_work::watchdog::{OfflinePolicy, Watchdog};
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
use crate::local_registry::{LocalRegistry, SOURCE_LOCAL, source_cp};
use crate::mgmt_gateway::MgmtGateway;
use crate::reconcile::{self, RunningAgent, RunningAgents};
use crate::secrets_gateway::SecretsGateway;
use crate::session_supervisor;
use crate::state;
use crate::supervisor::{SpawnOverrides, Supervisor};
use crate::task_loop;
use crate::task_worker;

/// Config passed from the CLI, overrides any file-based config.
pub struct CliOverrides {
    pub backend_url: String,
    pub token: String,
    pub work_dir: PathBuf,
    pub offline_grace_secs: u64,
    /// If another edgeplaned holds the singleton lock, terminate it and take over.
    /// SIGTERM, wait 5s, SIGKILL if still alive. Use only when the existing
    /// daemon is hung — otherwise prefer `systemctl --user restart edgeplaned.service`.
    pub kill_existing: bool,
    /// Permit startup to continue when a required TCP port (attach_ws 8009,
    /// mgmt 7731) is in use. Default is fatal. Use only when you knowingly
    /// need partial functionality and are willing to lose attach/mgmt.
    pub allow_degraded: bool,
}

pub async fn run(cli: CliOverrides) -> Result<()> {
    // Singleton guard: acquire the kernel flock before any port binds, before
    // touching the registry, before reaching out to the controlplane. If
    // another edgeplaned is running, this returns a structured error and we exit.
    // The lock is held for the lifetime of this function — released by the
    // kernel on any termination (clean exit, panic, SIGKILL, OOM).
    let lock_path = edgeplaned_core::paths::lock_file_path();
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

    tracing::info!("edgeplaned daemon starting");
    tracing::info!("backend: {}", cfg.backend_url);
    tracing::info!("work_dir: {}", cfg.work_dir.display());
    tracing::info!(
        "domains: {:?}",
        cfg.domains.iter().map(|m| &m.domain_id).collect::<Vec<_>>()
    );

    // Fail-fast port probe. The singleton lock already catches the dominant
    // case (another edgeplaned holding these ports), but a third party could be
    // holding them. Probe → drop → let serve() re-bind. The TOCTOU window
    // between probe and re-bind is microseconds.
    probe_required_ports(&cfg, cli.allow_degraded).await?;

    std::fs::create_dir_all(&cfg.work_dir)?;

    // Phase 5a: open (or create) the local SQLite registry. Used in both
    // standalone mode (source of truth) and federated mode (synced cache).
    // On failure: log and continue — federated still works, standalone falls
    // back to legacy yaml domains.
    let registry: Option<LocalRegistry> = LocalRegistry::default_path()
        .and_then(|p| {
            tracing::info!("local registry: {}", p.display());
            LocalRegistry::open(&p)
        })
        .map_err(|e| {
            tracing::warn!(
                "Could not open local registry: {e:#}. \
                 Standalone mode will fall back to yaml domains."
            );
            e
        })
        .ok();

    let client = Arc::new(
        BackendClient::new(&cfg.backend_url, &cfg.token)
            .with_api_prefix(
                std::env::var("EP_API_PREFIX").unwrap_or_else(|_| "/api".to_string()),
            ),
    );

    // Self-heal `attach_secret` (the browser-attach HMAC key).
    //
    // The controlplane signs short-lived attach tokens with the per-node
    // `attach_secret` minted at registration; `attach_ws` validates them with
    // the same secret. A node registered out-of-band — or before this sync
    // path existed — can have an empty `attach_secret` in its local profile,
    // which makes `attach_ws` default-deny and silently breaks browser attach.
    // When federated (node_id set) and the local secret is empty, fetch it from
    // the controlplane (owner-scoped) so the two sides match. Best-effort: on
    // failure attach stays disabled until the next start; nothing else breaks.
    let attach_secret_missing = cfg
        .attach_secret
        .as_deref()
        .map(str::is_empty)
        .unwrap_or(true);
    if let Some(node_id) = cfg.node_id.clone().filter(|_| attach_secret_missing) {
        let path = format!("/runtime/nodes/{node_id}/attach-secret");
        match client.get::<serde_json::Value>(&path).await {
            Ok(v) => match v.get("attach_secret").and_then(|s| s.as_str()) {
                Some(secret) if !secret.is_empty() => {
                    cfg.attach_secret = Some(secret.to_string());
                    tracing::info!(
                        "Fetched attach_secret from controlplane for node {node_id}; \
                         browser attach enabled."
                    );
                }
                _ => tracing::warn!(
                    "controlplane returned no attach_secret for node {node_id}; \
                     browser attach stays disabled."
                ),
            },
            Err(e) => tracing::warn!(
                "Could not fetch attach_secret for node {node_id}: {e:#}. \
                 Browser attach stays disabled until next start."
            ),
        }
    }

    // Bootstrap: idempotently provision `home-{hostname}` domain + `intake`
    // mission for per-node coordination. Runs after fleet_import (which
    // establishes the agent registry) and after the client is constructed.
    // Soft-fail: controlplane unreachable is logged as a warning; edgeplaned continues.
    match bootstrap::run(&client).await {
        Ok(summary) => {
            if summary.domain_created || summary.mission_created {
                tracing::info!(
                    "bootstrap: provisioned home domain={} intake mission={} \
                     (domain_created={}, mission_created={})",
                    summary.domain_id,
                    summary.mission_id,
                    summary.domain_created,
                    summary.mission_created,
                );
            } else {
                tracing::debug!(
                    "bootstrap: home domain and intake mission already exist \
                     (domain={}, mission={})",
                    summary.domain_id,
                    summary.mission_id,
                );
            }
        }
        Err(e) => {
            // run() itself is soft-fail and returns Ok on connectivity errors,
            // but handle the unexpected Err case for completeness.
            tracing::warn!("bootstrap: unexpected error: {e:#}. Continuing.");
        }
    }

    // Phase 2 — Ephemeral task subagent claimer loop.
    // Polls for claimable MeshTasks, spawns `claude -p` subagents, cleans up
    // on completion. Runs as an independent background tokio task; daemon
    // continues if it exits unexpectedly (which it shouldn't — the loop is
    // soft-fail internally). Gated by `task_worker_enabled` so it can be
    // disabled without restarting by toggling the config.
    {
        let tw_client = Arc::clone(&client);
        let tw_config = cfg.clone();
        tokio::spawn(async move {
            task_worker::run(tw_client, tw_config).await;
        });
    }

    // Phase 3 — Triage loop.
    // Examines unscoped ready tasks in the intake mission and either routes
    // them to a profile (via child meshtask) or surfaces them for human
    // triage in `Aria/Engineer/inbox.md`. Runs independently of P2 at a slower
    // cadence (default 60s vs 30s). Gated by `task_worker_triage_enabled`.
    if cfg.task_worker_triage_enabled {
        let triage_client = Arc::clone(&client);
        let triage_config = cfg.clone();
        tokio::spawn(async move {
            task_worker::run_triage_loop(triage_client, triage_config).await;
        });
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
    let mut agent_specs = resolve_agent_specs(&cfg, &client, registry.as_ref()).await;

    // Federated boot-time launch-override merge (Gap 2 fix).
    //
    // resolve_agent_specs returns controlplane specs with empty launch_overrides
    // and no local_alias_id. The WS/poll path runs merge_federated_overrides
    // inside persist_and_resolve_specs, but the initial spawn never goes through
    // that path — so on boot (and after every daemon restart) there is a window
    // until the first WS event / ~60 s poll where attach resolves to None.
    //
    // Fix: run the same merge here so aliases are present at boot. Gated on
    // node_id (federated mode only) so standalone behavior is byte-identical.
    if cfg.node_id.is_some() {
        if let Ok(db_path_boot) = LocalRegistry::default_path() {
            match tokio::task::spawn_blocking(move || {
                let reg = LocalRegistry::open(&db_path_boot)?;
                let ctxs = reg.list_all_launch_contexts()?;
                let overrides = reg.list_local_runtime_overrides()?;
                Ok::<_, anyhow::Error>((ctxs, overrides))
            })
            .await
            {
                Ok(Ok((boot_ctxs, local_runtime_overrides))) => {
                    merge_federated_overrides(&mut agent_specs, &boot_ctxs);
                    apply_runtime_overrides(&mut agent_specs, &local_runtime_overrides);
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        "boot merge: could not read launch contexts: {e:#}. \
                         zellij_hosted agents may not have PTY bridge until first poll."
                    );
                }
                Err(e) => {
                    tracing::warn!("boot merge: launch context query panicked: {e:#}.");
                }
            }
        }

        // Federated boot dedup (Gap 3 fix — double-attach prevention).
        //
        // After merge_federated_overrides, each controlplane spec (opaque id like
        // `aria-engineer-<hash>`) has `local_alias_id = Some("engineer")` when it
        // matched a local launch-context. The additive layer in resolve_agent_specs
        // also appended a separate spec with agent_id="engineer" — the dedup check
        // there used base_ids (controlplane ids) and didn't know the alias yet.
        //
        // Result: 12 specs → diff_specs against empty running → 12 to_spawn →
        // two PTY bridges per session (one under the opaque id, one under "engineer").
        //
        // Fix: collect all local_alias_ids set by the merge, then remove any spec
        // whose agent_id appears in that set. The controlplane spec (now carrying
        // the merged zellij_session + local_alias_id) is kept; the additive-layer
        // duplicate is dropped. 6 specs remain — one bridge per session.
        //
        // Gated on federated mode (cfg.node_id.is_some()) so standalone mode is
        // byte-identical — additive-layer specs are still needed there.
        {
            // Collect owned Strings so the borrow on agent_specs ends before
            // the mutable retain call.
            let aliased_ids: std::collections::HashSet<String> = agent_specs
                .iter()
                .filter_map(|s| s.local_alias_id.clone())
                .collect();
            if !aliased_ids.is_empty() {
                let before = agent_specs.len();
                agent_specs.retain(|s| !aliased_ids.contains(&s.agent_id));
                let dropped = before - agent_specs.len();
                if dropped > 0 {
                    tracing::info!(
                        "federated boot dedup: dropped {dropped} additive-layer spec(s) \
                         covered by controlplane aliases (aliases: {aliased_ids:?})"
                    );
                }
            }
        }
    }

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
             `edgeplane daemon agent enroll` (controlplane-driven) or add legacy \
             `domains:` entries to {} (deprecated path).",
            DaemonConfig::user_config_path().display()
        );
    }

    // Compute the registry path here so the shutdown checkpoint below (which
    // runs after the mgmt-gateway block) can use the same path. The
    // mgmt-gateway block uses a clone of this for registry_path and moves that
    // clone into AgentOpsHandle, so we need the original to remain accessible.
    let shutdown_registry: PathBuf = crate::local_registry::LocalRegistry::default_path()
        .unwrap_or_else(|_| edgeplaned_core::paths::registry_db_path());

    // If the daemon has a registered node_id, send periodic node heartbeats
    // to edgeplane-tower with current Tailscale info.
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
    let session_store = Arc::new(edgeplaned_secrets::SessionStore::new());
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
        // locally-supervised agents. Path is cloned from shutdown_registry
        // (computed before this block) so both targets resolve identically.
        let registry_path = shutdown_registry.clone();

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
            tokio::sync::broadcast::channel::<edgeplaned_core::types::SupervisorEvent>(256);
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

        // Keep the SQLite WAL files bounded: TRUNCATE-checkpoint registry + receipts
        // on a 60s cadence. journal_size_limit caps the file; this actively reclaims it.
        // Resolve the SAME paths the live stores opened (registry_path / paths::*),
        // so the checkpoint provably targets the real WAL, not a stray empty file
        // created by edgeplaned_paths::*_db_path() which points at a different dir.
        let ckpt_registry = registry_path.clone();
        let ckpt_receipts = paths::receipts_db_path();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            tick.tick().await; // consume the immediate first tick
            loop {
                tick.tick().await;
                for p in [ckpt_registry.clone(), ckpt_receipts.clone()] {
                    let _ = tokio::task::spawn_blocking(move || {
                        if let Err(e) = edgeplaned_paths::checkpoint_truncate(&p) {
                            tracing::debug!("wal checkpoint {}: {e}", p.display());
                        }
                    })
                    .await;
                }
            }
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

    // Drain the WAL on graceful shutdown so the files don't linger at high-water mark.
    // Use shutdown_registry (computed before the mgmt-gateway block, identical to
    // the registry_path used by the live store and the periodic checkpoint) and
    // paths::receipts_db_path() — not edgeplaned_paths which may point at a
    // different directory and silently create a stale empty file.
    if let Err(e) = edgeplaned_paths::checkpoint_truncate(&shutdown_registry) {
        tracing::debug!("shutdown wal checkpoint {}: {e}", shutdown_registry.display());
    }
    let receipts_path = paths::receipts_db_path();
    if let Err(e) = edgeplaned_paths::checkpoint_truncate(&receipts_path) {
        tracing::debug!("shutdown wal checkpoint {}: {e}", receipts_path.display());
    }

    // Clean up sockets on exit.
    let _ = std::fs::remove_file(attach_gateway::socket_path());
    let _ = std::fs::remove_file(paths::secrets_socket_path());
    Ok(())
}

/// Merge `~/.ep/state.json` (v2: profiles map + active_profile) into
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
                         Use `edgeplane daemon use <profile>` to select a controlplane."
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
                     Run `edgeplane daemon profile add` to register with a controlplane, \
                     or `edgeplane daemon agent enroll` to add agents in standalone mode.",
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
/// After the read-back, merge local launch overrides into any spec that lacks
/// them (federated zellij_hosted attach fix). Reads ALL sources so that
/// agents registered via `edgeplane daemon agent import --source aria` (the
/// live source the fleet actually uses) are visible — not just the defunct
/// `fleet_import` source.
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

    let mut db_specs = match tokio::task::spawn_blocking(move || {
        LocalRegistry::open(&db_path)?.list_specs_by_source(&source)
    })
    .await
    {
        Ok(Ok(db_specs)) => db_specs,
        Ok(Err(e)) => {
            tracing::warn!("persist_and_resolve: read back failed: {e:#}. Using in-memory specs.");
            return specs;
        }
        Err(e) => {
            tracing::warn!("persist_and_resolve: read task panicked: {e:#}. Using in-memory specs.");
            return specs;
        }
    };

    // Re-attach the `name` field from the original in-memory specs (it is
    // not persisted to SQLite — the registry only stores spec identity
    // fields). Without this, merge_federated_overrides can't match by name.
    {
        let name_by_id: std::collections::HashMap<&str, Option<&str>> = specs
            .iter()
            .map(|s| (s.agent_id.as_str(), s.name.as_deref()))
            .collect();
        for s in &mut db_specs {
            if let Some(name) = name_by_id.get(s.agent_id.as_str()) {
                s.name = name.map(str::to_owned);
            }
        }
    }

    // Federated launch-override merge: for each controlplane spec that is
    // zellij_hosted+Persistent with empty launch_overrides, look up the
    // matching local launch context (any source) and merge it in. Also sets
    // local_alias_id so diff_specs can recognise the logical identity.
    //
    // We use list_all_launch_contexts() (source-agnostic) rather than the
    // defunct list_launch_contexts_by_source("fleet_import") because the live
    // fleet agents are registered with source "aria" — the fleet_import source
    // is dead code and would always return an empty slice, silently disabling
    // the attach feature.
    if let Ok(db_path_for_merge) = LocalRegistry::default_path() {
        match tokio::task::spawn_blocking(move || {
            let reg = LocalRegistry::open(&db_path_for_merge)?;
            let ctxs = reg.list_all_launch_contexts()?;
            let overrides = reg.list_local_runtime_overrides()?;
            Ok::<_, anyhow::Error>((ctxs, overrides))
        })
        .await
        {
            Ok(Ok((fleet_ctxs, local_runtime_overrides))) => {
                merge_federated_overrides(&mut db_specs, &fleet_ctxs);
                // Post-merge: apply runtime_kind overrides for specs the local
                // manifest says should use a different runtime than the controlplane
                // recorded. e.g. a profile set to "claude_agent_acp" in profiles.toml
                // while the controlplane still stores "zellij_hosted".
                apply_runtime_overrides(&mut db_specs, &local_runtime_overrides);
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "persist_and_resolve: could not read launch contexts for merge: {e:#}. \
                     zellij_hosted agents will not have PTY bridge in federated mode."
                );
            }
            Err(e) => {
                tracing::warn!(
                    "persist_and_resolve: launch context query panicked: {e:#}."
                );
            }
        }
    }

    db_specs
}

/// Post-merge pass: apply local manifest runtime overrides.
///
/// Called after `merge_federated_overrides` has set `local_alias_id` on any spec
/// matched to a local context. For each matched spec whose `runtime_kind` differs
/// from the local manifest record, this function updates `runtime_kind` (and, for
/// ACP profiles, sets `profile_path` to the state_dir so claude starts with the
/// right CLAUDE.md).
///
/// `local_overrides` is a slice of `(agent_id, runtime_kind)` for all
/// non-controlplane sources in the local registry (returned by
/// `LocalRegistry::list_local_runtime_overrides`).
fn apply_runtime_overrides(specs: &mut Vec<AgentSpec>, local_overrides: &[(String, String)]) {
    use edgeplaned_core::types::StateDirSpec;

    let override_map: HashMap<&str, &str> = local_overrides
        .iter()
        .map(|(id, rt)| (id.as_str(), rt.as_str()))
        .collect();

    for spec in specs.iter_mut() {
        let alias = match spec.local_alias_id.as_deref() {
            Some(a) if !a.is_empty() => a,
            _ => continue,
        };
        let Some(&local_rt) = override_map.get(alias) else { continue };
        if local_rt == spec.runtime_kind { continue; }

        tracing::info!(
            "apply_runtime_overrides: overriding runtime_kind for '{}' (alias='{}') \
             from '{}' → '{}'",
            spec.agent_id,
            alias,
            spec.runtime_kind,
            local_rt,
        );
        spec.runtime_kind = local_rt.to_string();

        // Clear the Zellij session name — it only applies to zellij_hosted.
        spec.launch_overrides.zellij_session = None;

        // ACP supervisor uses profile_path as cwd so claude loads the right CLAUDE.md.
        if local_rt == "claude_agent_acp" && spec.profile_path.is_none() {
            if let Some(StateDirSpec::Persistent { path }) = &spec.launch_overrides.state_dir_spec {
                spec.profile_path = Some(path.clone());
            }
        }
    }
}

/// Federated launch-override merge.
///
/// For each spec in `specs` that is `runtime_kind == "zellij_hosted"`,
/// `session_mode == Persistent`, and has empty `launch_overrides`, search
/// `local_ctxs` for a matching entry and copy its overrides into the spec.
/// Also sets `local_alias_id` so `diff_specs` can treat the controlplane spec
/// as the same logical agent as the already-running local one.
///
/// `local_ctxs` is source-agnostic — the caller should pass the full result of
/// `list_all_launch_contexts()` (not a source-filtered slice) so that agents
/// registered under any source (e.g. `"aria"`, the live fleet source) are
/// visible. The `source` field of each context is not examined here.
///
/// ## Matching rule
///
/// We match on a *key* derived from the controlplane spec. Preferred: the agent
/// `name` field (e.g. `"aria-engineer"`). Live fleet agents come back from
/// `GET /runtime/nodes/{id}/agents` with `name: null`, so when `name` is absent
/// the key falls back to the profile embedded in the public_id via
/// [`profile_from_public_id`] (e.g. `"aria-research-22e6cd17"` → `"research"`).
/// The local context's `agent_id` is the short profile name (e.g. `"engineer"`).
/// We consider a match when **exactly one** context satisfies one of:
///   - The key **is** the profile name (exact), OR
///   - The key **ends with** `"-{profile_name}"` (e.g. `"aria-engineer"` →
///     suffix match against `"engineer"`) — **name-based keys only**. A profile
///     recovered from a public_id is already bare and matches by exact equality
///     only (the suffix clause is gated off for it), so a hyphenated derived
///     profile like `"foo-bar"` can't wrongly suffix-match a lone `"bar"`.
///
/// **Uniqueness is enforced.** If two contexts both match (possible if one
/// profile name is a suffix of another, e.g. `"work"` and `"a-work"` where
/// cp_name is `"aria-work"`), the spec is left unchanged — ambiguity is safer
/// than a wrong assignment. The collision will be logged as a warning.
///
/// This is intentionally conservative: we only merge when there is a single
/// unambiguous match. If no match exists (controlplane has no `name`, or no
/// context has a matching id) the spec is left unchanged — the agent simply
/// won't have a PTY bridge until the enrollment recipe is corrected.
///
/// This function is pure (no I/O) and is unit-tested independently.
pub(crate) fn merge_federated_overrides(
    specs: &mut Vec<AgentSpec>,
    local_ctxs: &[crate::local_registry::AgentLaunchContext],
) {
    use crate::config::SessionMode;

    for spec in specs.iter_mut() {
        // Only applicable to zellij_hosted Persistent agents.
        if spec.runtime_kind != "zellij_hosted" || spec.session_mode != SessionMode::Persistent {
            continue;
        }
        // Already has overrides (e.g. from a prior merge or local-source path).
        if spec.launch_overrides.zellij_session.is_some() {
            continue;
        }
        // Key to match against local context agent_ids (short profile names).
        // Prefer the controlplane `name` (e.g. "aria-engineer"). Live fleet
        // agents come back from GET /runtime/nodes/{id}/agents with `name: null`,
        // so fall back to the profile embedded in the public_id (`agent_id`),
        // shaped "<prefix>-<profile>-<hex>" (e.g. "aria-research-22e6cd17").
        // Without this fallback, federated zellij agents never get a PTY bridge
        // and web attach stays dormant (the bug Phase 7 closes).
        // `allow_suffix` gates the suffix clause below to name-based keys only.
        // A name like "aria-engineer" legitimately suffix-matches "engineer".
        // A profile recovered from a public_id is already bare, so it must match
        // by exact equality — otherwise a hyphenated derived profile ("foo-bar")
        // would wrongly suffix-match an unrelated lone context ("bar"), silently
        // merging the wrong PTY bridge.
        let (cp_key, allow_suffix) = match spec.name.as_deref() {
            Some(n) if !n.is_empty() => (n, true),
            _ => match profile_from_public_id(&spec.agent_id) {
                Some(profile) => (profile, false),
                None => continue,
            },
        };

        // Collect ALL contexts that match, then enforce uniqueness.
        // Two clauses — exact match or suffix match. The former redundant
        // rsplit_once clause has been removed: it matched on the last
        // hyphen-segment alone, which would cause `"aria-foo-bar"` to
        // match both "bar" and "foo-bar" non-deterministically.
        let matches: Vec<&crate::local_registry::AgentLaunchContext> = local_ctxs
            .iter()
            .filter(|ctx| {
                let profile_id = ctx.agent_id.as_str();
                cp_key == profile_id
                    || (allow_suffix
                        && cp_key.strip_suffix(&format!("-{profile_id}")).is_some())
            })
            .collect();

        let matched_ctx = match matches.len() {
            0 => continue, // no match — leave spec unchanged
            1 => matches[0],
            _ => {
                // Ambiguous — two or more fleet contexts match this name.
                // Safer to skip than to merge the wrong one.
                let candidates: Vec<&str> = matches.iter().map(|c| c.agent_id.as_str()).collect();
                tracing::warn!(
                    "federated merge: ambiguous match for controlplane spec '{}' (key='{}'): \
                     {} fleet contexts match ({:?}). Skipping merge — rename profiles to \
                     avoid suffix collisions.",
                    spec.agent_id,
                    cp_key,
                    matches.len(),
                    candidates,
                );
                continue;
            }
        };

        spec.launch_overrides = crate::supervisor::SpawnOverrides {
            vault_folder: matched_ctx.vault_folder.clone(),
            state_dir_spec: matched_ctx.state_dir_spec.clone(),
            zellij_session: matched_ctx.zellij_session.clone(),
        };
        spec.local_alias_id = Some(matched_ctx.agent_id.clone());
        tracing::info!(
            "federated merge: controlplane spec '{}' (key='{}') matched \
             local context '{}' source='{}' (zellij_session={:?})",
            spec.agent_id,
            cp_key,
            matched_ctx.agent_id,
            matched_ctx.source,
            matched_ctx.zellij_session,
        );
    }
}

/// Extract the profile segment from a controlplane agent `public_id`.
///
/// Federated public_ids are shaped `<prefix>-<profile>-<hex>` (e.g.
/// `"aria-research-22e6cd17"`). The profile is everything between the first and
/// last `-`, so a hyphenated profile is preserved (`"aria-foo-bar-9f"` →
/// `"foo-bar"`). Returns `None` when the id has fewer than three
/// `-`-separated segments (no embedded profile to recover).
fn profile_from_public_id(public_id: &str) -> Option<&str> {
    let (_prefix, rest) = public_id.split_once('-')?;
    let (profile, _hex) = rest.rsplit_once('-')?;
    (!profile.is_empty()).then_some(profile)
}

// ── Phase 4d: per-agent spawner + reconcile-driven apply ─────────────────────

/// Bundles the shared deps every per-agent spawn needs. Cloned/Arc-shared
/// across the start-time path, the WS subscriber, and the poll fallback so
/// they all use the same spawn flow.
pub(crate) struct Spawner {
    pub client: Arc<BackendClient>,
    pub watchdog: Arc<edgeplaned_work::watchdog::Watchdog>,
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
                    "Reconcile: restarting agent {} (domain={}, mode={:?})",
                    spec.agent_id,
                    spec.domain_id,
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
                // 3a. Register supervisor name alias so that `edgeplane agent
                //     signal <short-name>` continues to work even though the
                //     supervisor is now keyed by the controlplane opaque id.
                //
                //     Example: spec spawned as "aria-engineer-708650f1" with
                //     local_alias_id="engineer" → register "engineer" →
                //     "aria-engineer-708650f1" so signal("engineer", ...)
                //     resolves to the live supervisor entry.
                if let Some(ref alias) = spec.local_alias_id {
                    self.supervisor
                        .register_name_alias(alias.clone(), spec.agent_id.clone())
                        .await;
                    tracing::debug!(
                        "supervisor alias registered: {alias} → {} (federated name resolution)",
                        spec.agent_id
                    );
                }
            }
        }
        // 4. Register attach-registry aliases for federated specs that are
        //    already running under a local fleet-import key. This enables
        //    web attach via the controlplane public_id to reach the existing
        //    PTY bridge without re-spawning or disturbing the live session.
        //
        //    Example: bridge registered under "engineer" → alias
        //    "aria-engineer-708650f1" → "engineer" means the controlplane
        //    attach path `wss /api/runtime/nodes/{node}/agents/{public_id}/attach`
        //    resolves to the live bridge.
        for (public_id, local_id) in &plan.alias_registrations {
            self.attach_registry
                .register_alias(public_id.clone(), local_id.clone())
                .await;
            tracing::debug!(
                "attach alias registered: {public_id} → {local_id} (federated no-op bridge)"
            );
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

        let extra_caps: Vec<edgeplaned_core::types::Capability> = spec
            .capabilities
            .iter()
            .map(|s| edgeplaned_core::types::Capability::new(s.clone()))
            .collect();
        let work_dir = paths::mcd_work_dir().join(&spec.agent_id);

        let mut acp_spawn_opts: Option<edgeplaned_acp::SpawnOpts> = None;

        let rt: Arc<edgeplaned_core::agent_runtime::DynAgentRuntime> = match spec
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
                spec.domain_id.clone(),
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

        let agent_handle = Arc::new(Mutex::new(edgeplaned_core::types::AgentHandle {
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
                    spec.domain_id.clone(),
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
                // session is owned by systemd, edgeplaned doesn't need a
                // session supervisor. Signals route via mgmt_gateway
                // `agent.local.signal` → ZellijHostedRuntime::signal().
                //
                // PTY bridge: spawn a `zellij attach` child and register
                // PtyAttachEndpoints so remote viewers can connect through
                // the existing attach_ws → pump_pty pipeline.
                if spec.runtime_kind == "zellij_hosted" {
                    if let Some(zellij_session) = spec.launch_overrides.zellij_session.clone() {
                        let bridge_jh = tokio::spawn(crate::zellij_bridge::run_for_agent(
                            spec.agent_id.clone(),
                            zellij_session.clone(),
                            self.attach_registry.clone(),
                        ));
                        handles.push(bridge_jh);
                        tracing::info!(
                            "ZellijHosted agent {} registered with PTY bridge \
                             (session '{zellij_session}')",
                            spec.agent_id
                        );
                    } else {
                        tracing::info!(
                            "ZellijHosted agent {} registered without PTY bridge \
                             (no zellij_session in launch_overrides)",
                            spec.agent_id
                        );
                    }
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
            "Spawned {} loop for {} agent {} in domain {}",
            match spec.session_mode {
                SessionMode::Task => "task",
                SessionMode::Persistent => "persistent-session",
            },
            spec.runtime_kind,
            spec.agent_id,
            spec.domain_id
        );

        Some(RunningAgent::new(spec.clone(), handles))
    }
}

// ── Phase 4c: agent assignment resolution ────────────────────────────────────

/// Internal flat representation of one agent the daemon should spawn.
/// Built from either the controlplane (preferred when state.node_id is set)
/// or yaml-defined domains (legacy fallback during the deprecation window).
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub agent_id: String,
    pub domain_id: String,
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
    /// Human-readable agent name as provided by the controlplane (e.g.
    /// `"aria-engineer"`). `None` for agents built from local registry or
    /// yaml. Used by federated-mode launch-override merging to match a
    /// controlplane spec against the local `fleet_import` launch-context row
    /// (whose `agent_id` is a short profile name like `"engineer"`).
    pub name: Option<String>,
    /// When federated-mode merging finds a matching local `fleet_import` agent,
    /// this is set to that agent's `agent_id` (e.g. `"engineer"`). `diff_specs`
    /// uses this to treat the controlplane spec as the same logical agent as
    /// the already-running local agent — preventing a spurious remove+respawn
    /// of the live zellij session on every controlplane poll.
    pub local_alias_id: Option<String>,
}

/// Build the initial spawn list.
///
/// Priority order for the "base" specs (one of these three is selected):
/// 1. Controlplane GET (federated — when `cfg.node_id` is set).
/// 2. Local SQLite registry (`source = 'local'`) — standalone mode.
/// 3. Legacy yaml `domains:` — deprecated fallback for pre-Phase-4 configs.
///
/// **Additive layer (always on when a registry is present):**
/// Agents with a launch context (any source tag) are appended as an additive
/// layer on top of whatever the base path returns. They coexist with
/// controlplane assignments and yaml legacy domains; each spec gets its
/// `launch_overrides` populated from `agent_launch_context` so the runtime
/// knows which Zellij session to address. Source-agnostic: picks up agents
/// registered via `edgeplane daemon agent import` (any `--source` tag) as
/// well as legacy `fleet_import` rows.
async fn resolve_agent_specs(
    cfg: &DaemonConfig,
    client: &BackendClient,
    registry: Option<&LocalRegistry>,
) -> Vec<AgentSpec> {
    let mut specs = base_agent_specs(cfg, client, registry).await;

    if let Some(reg) = registry {
        // Build a set of agent_ids already in the base list so we can
        // deduplicate — an agent enrolled locally AND synced from the
        // controlplane should not be spawned twice. Use owned Strings so
        // this set doesn't borrow `specs` (we push into `specs` below).
        let base_ids: std::collections::HashSet<String> =
            specs.iter().map(|s| s.agent_id.clone()).collect();

        match reg.list_all_launch_contexts() {
            Ok(contexts) => {
                let mut added = 0usize;
                for ctx in contexts {
                    if base_ids.contains(&ctx.agent_id) {
                        // Already in base list — skip (don't duplicate).
                        continue;
                    }
                    // Look up the corresponding agent record to build the spec.
                    match reg.list_specs_by_source(&ctx.source) {
                        Ok(source_specs) => {
                            if let Some(mut spec) = source_specs
                                .into_iter()
                                .find(|s| s.agent_id == ctx.agent_id)
                            {
                                spec.launch_overrides = SpawnOverrides {
                                    vault_folder: ctx.vault_folder,
                                    state_dir_spec: ctx.state_dir_spec,
                                    zellij_session: ctx.zellij_session,
                                };
                                // name and local_alias_id stay None here —
                                // these are local-source specs, not
                                // controlplane-originated ones.
                                specs.push(spec);
                                added += 1;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "resolve_agent_specs: could not list specs for source '{}': {e:#}",
                                ctx.source
                            );
                        }
                    }
                }
                if added > 0 {
                    tracing::info!(
                        "launch_context: {added} agent(s) with launch context appended to spawn list"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Could not list launch contexts from registry: {e:#}. \
                     Continuing without context-based agents."
                );
            }
        }
    }

    specs
}

/// Resolve the "base" agent list per the controlplane > local > yaml priority.
/// Separated from `resolve_agent_specs` so the launch-context additive layer
/// can wrap it cleanly.
async fn base_agent_specs(
    cfg: &DaemonConfig,
    client: &BackendClient,
    registry: Option<&LocalRegistry>,
) -> Vec<AgentSpec> {
    if let Some(node_id) = cfg.node_id.as_deref() {
        match fetch_node_agents(client, node_id).await {
            Ok(specs) => {
                if !cfg.domains.is_empty() {
                    let yaml_domains: Vec<&str> =
                        cfg.domains.iter().map(|m| m.domain_id.as_str()).collect();
                    tracing::warn!(
                        "yaml carries `domains:` ({:?}) but node {} is registered with the \
                         controlplane; using controlplane assignment list. \
                         Remove `domains:` from config.yaml.",
                        yaml_domains,
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
                     (enroll via `edgeplane daemon agent enroll`).",
                    specs.len()
                );
                return specs;
            }
            Ok(_) => {
                tracing::info!(
                    "Standalone mode: local registry is empty. \
                     Checking legacy yaml domains."
                );
            }
            Err(e) => {
                tracing::warn!("Could not read local registry: {e:#}. Falling back to yaml.");
            }
        }
    } else {
        tracing::info!(
            "No node_id in state file and no local registry; falling back to legacy yaml domains. \
             Run `edgeplane daemon profile add` to register with a controlplane, \
             or `edgeplane daemon agent enroll` to add agents in standalone mode."
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
    let domain_id = v
        .get("domain_id")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow!("agent {agent_id} missing `domain_id`"))
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
    // `name` is the human-readable agent name from the controlplane (e.g.
    // "aria-engineer"). Absent on pre-name-field controlplanes; tolerated
    // gracefully — launch-override merging simply won't fire.
    let name = v
        .get("name")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    Ok(AgentSpec {
        agent_id,
        domain_id,
        runtime_kind,
        session_mode,
        capabilities,
        profile_path,
        webhook_url,
        launch_overrides: SpawnOverrides::default(),
        name,
        local_alias_id: None,
    })
}

fn yaml_specs(cfg: &DaemonConfig) -> Vec<AgentSpec> {
    let mut out = Vec::new();
    for m in &cfg.domains {
        for a in &m.agents {
            out.push(AgentSpec {
                agent_id: a.agent_id.clone(),
                domain_id: m.domain_id.clone(),
                runtime_kind: a.runtime_kind.clone(),
                session_mode: a.session_mode,
                capabilities: a.capabilities.clone(),
                profile_path: a.profile_path.clone(),
                webhook_url: None,
                launch_overrides: SpawnOverrides::default(),
                name: None,
                local_alias_id: None,
            });
        }
    }
    if !out.is_empty() {
        // Phase 6: yaml is the legacy bootstrap path. New deployments should
        // federate via `edgeplane daemon profile add` or run standalone via
        // `edgeplane daemon agent enroll-home`. The yaml path keeps working but won't
        // see future ergonomic improvements (auto-provisioning, sync loop, …).
        tracing::warn!(
            "Loaded {} agent(s) from legacy ~/.ep/config.yaml. \
             yaml-only configuration is deprecated. Migrate by running \
             `edgeplane daemon profile add` (federated) or `edgeplane daemon agent enroll-home` \
             (standalone) — see docs/plans/2026-05-10-edgeplaned-phase6-home-domain-sync.md.",
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
    let mgmt_port: u16 = std::env::var("EP_MESH_MGMT_PORT")
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
                     Another process is bound to a port edgeplaned needs. To diagnose:\n  \
                       ss -lntp | grep -E ':({mgmt_port}|8009)\\b'\n  \
                       edgeplaned doctor\n\n\
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
            "domain_id": "m-1",
            "runtime_kind": "claude_agent_acp",
        });
        let s = agent_spec_from_json(&v).unwrap();
        assert_eq!(s.agent_id, "a-1");
        assert_eq!(s.domain_id, "m-1");
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
            "domain_id": "m-1",
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
            "domain_id": "m-1",
            "runtime_kind": "claude_code",
            "supervision_mode": "future-mode",
        });
        let s = agent_spec_from_json(&v).unwrap();
        assert_eq!(s.session_mode, SessionMode::Task);
    }

    #[test]
    fn agent_spec_from_json_missing_required_fields_errors() {
        // Missing id
        let v = json!({"domain_id": "m", "runtime_kind": "claude_code"});
        assert!(agent_spec_from_json(&v).is_err());
        // Missing domain_id
        let v = json!({"id": "a", "runtime_kind": "claude_code"});
        assert!(agent_spec_from_json(&v).is_err());
        // Missing runtime_kind
        let v = json!({"id": "a", "domain_id": "m"});
        assert!(agent_spec_from_json(&v).is_err());
    }

    #[test]
    fn agent_spec_from_json_reads_name_field() {
        let v = json!({
            "id": "aria-engineer-abc12345",
            "domain_id": "m-1",
            "runtime_kind": "zellij_hosted",
            "supervision_mode": "persistent",
            "name": "aria-engineer",
        });
        let s = agent_spec_from_json(&v).unwrap();
        assert_eq!(s.name.as_deref(), Some("aria-engineer"));
        assert!(s.local_alias_id.is_none());
    }

    #[test]
    fn agent_spec_from_json_name_absent_is_none() {
        let v = json!({
            "id": "a-1",
            "domain_id": "m-1",
            "runtime_kind": "claude_agent_acp",
        });
        let s = agent_spec_from_json(&v).unwrap();
        assert!(s.name.is_none());
    }

    // ── merge_federated_overrides tests ──────────────────────────────────────

    use crate::local_registry::AgentLaunchContext;

    fn fleet_ctx(agent_id: &str, zellij_session: &str) -> AgentLaunchContext {
        AgentLaunchContext {
            source: crate::fleet_import::SOURCE_FLEET_IMPORT.to_string(),
            agent_id: agent_id.to_string(),
            vault_folder: Some(agent_id.to_string()),
            state_dir_spec: None,
            zellij_session: Some(zellij_session.to_string()),
            systemd_service: Some(format!("aria-{agent_id}.service")),
            supervise_paused: false,
        }
    }

    fn cp_zellij_spec(agent_id: &str, cp_name: &str) -> AgentSpec {
        AgentSpec {
            agent_id: agent_id.to_string(),
            domain_id: "m-1".to_string(),
            runtime_kind: "zellij_hosted".to_string(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: Default::default(),
            name: Some(cp_name.to_string()),
            local_alias_id: None,
        }
    }

    /// Federated spec with name "aria-engineer" merges fleet_import context
    /// for "engineer": launch_overrides.zellij_session is set, local_alias_id
    /// is set to "engineer".
    #[test]
    fn merge_federated_overrides_sets_zellij_session_and_alias() {
        let mut specs = vec![cp_zellij_spec("aria-engineer-abc12345", "aria-engineer")];
        let ctxs = vec![fleet_ctx("engineer", "aria-engineer")];
        merge_federated_overrides(&mut specs, &ctxs);

        let s = &specs[0];
        assert_eq!(
            s.launch_overrides.zellij_session.as_deref(),
            Some("aria-engineer"),
            "zellij_session should be merged from fleet_import context"
        );
        assert_eq!(
            s.local_alias_id.as_deref(),
            Some("engineer"),
            "local_alias_id should be set to fleet_import agent_id"
        );
    }

    /// Exact name match (profile name == cp name, no prefix).
    #[test]
    fn merge_federated_overrides_exact_name_match() {
        let mut specs = vec![cp_zellij_spec("aria-operator-00000001", "operator")];
        let ctxs = vec![fleet_ctx("operator", "operator")];
        merge_federated_overrides(&mut specs, &ctxs);

        assert_eq!(
            specs[0].launch_overrides.zellij_session.as_deref(),
            Some("operator")
        );
        assert_eq!(specs[0].local_alias_id.as_deref(), Some("operator"));
    }

    /// Non-zellij_hosted spec (claude_agent_acp) is left unchanged.
    #[test]
    fn merge_federated_overrides_skips_non_zellij_hosted() {
        let mut specs = vec![AgentSpec {
            agent_id: "aria-researcher-abc".to_string(),
            domain_id: "m-1".to_string(),
            runtime_kind: "claude_agent_acp".to_string(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: Default::default(),
            name: Some("aria-researcher".to_string()),
            local_alias_id: None,
        }];
        let ctxs = vec![fleet_ctx("researcher", "aria-researcher")];
        merge_federated_overrides(&mut specs, &ctxs);

        assert!(
            specs[0].launch_overrides.zellij_session.is_none(),
            "non-zellij_hosted spec should not be merged"
        );
        assert!(specs[0].local_alias_id.is_none());
    }

    /// Spec with no `name` field falls back to the profile embedded in the
    /// public_id ("aria-work-abc" → "work") and merges. This is the real
    /// live-fleet shape: GET /runtime/nodes/{id}/agents returns name=null.
    /// (Previously this asserted a skip — which was the Phase 7 bug.)
    #[test]
    fn merge_federated_overrides_derives_profile_from_public_id_when_name_absent() {
        let mut specs = vec![AgentSpec {
            agent_id: "aria-work-abc".to_string(),
            domain_id: "m-1".to_string(),
            runtime_kind: "zellij_hosted".to_string(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: Default::default(),
            name: None, // controlplane returned name=null
            local_alias_id: None,
        }];
        let ctxs = vec![fleet_ctx("work", "aria-work")];
        merge_federated_overrides(&mut specs, &ctxs);

        assert_eq!(
            specs[0].launch_overrides.zellij_session.as_deref(),
            Some("aria-work"),
            "name=null spec must still merge via public_id middle segment"
        );
        assert_eq!(specs[0].local_alias_id.as_deref(), Some("work"));
    }

    /// End-to-end against the real `GET /runtime/nodes/{id}/agents` shape:
    /// `agent_spec_from_json` parses a record with `name: null` (profile lives
    /// only in the public_id), then the merge wires the PTY bridge. This is the
    /// exact integration the prior `name = "aria-engineer"` fixtures masked.
    #[test]
    fn agent_spec_from_json_name_null_merges_via_public_id() {
        let row = serde_json::json!({
            "public_id": "aria-research-22e6cd17",
            "name": serde_json::Value::Null,
            "domain_id": "m-1",
            "runtime_kind": "zellij_hosted",
            "supervision_mode": "persistent",
        });
        let spec = agent_spec_from_json(&row).expect("real node-agents record parses");
        assert_eq!(spec.name, None, "controlplane returns name=null");
        assert_eq!(spec.agent_id, "aria-research-22e6cd17");
        assert_eq!(spec.session_mode, SessionMode::Persistent);

        let mut specs = vec![spec];
        let ctxs = vec![fleet_ctx("research", "aria-research")];
        merge_federated_overrides(&mut specs, &ctxs);

        assert_eq!(
            specs[0].launch_overrides.zellij_session.as_deref(),
            Some("aria-research")
        );
        assert_eq!(specs[0].local_alias_id.as_deref(), Some("research"));
    }

    /// name=null + a hyphenated profile: the middle segment ("foo-bar") is
    /// recovered whole (split on first/last `-`) and matches the "foo-bar" ctx.
    #[test]
    fn merge_federated_overrides_public_id_preserves_hyphenated_profile() {
        let mut spec = cp_zellij_spec("aria-foo-bar-9f3c", "unused");
        spec.name = None; // force the public_id fallback
        let mut specs = vec![spec];
        let ctxs = vec![fleet_ctx("foo-bar", "aria-foo-bar")];
        merge_federated_overrides(&mut specs, &ctxs);

        assert_eq!(
            specs[0].local_alias_id.as_deref(),
            Some("foo-bar"),
            "hyphenated profile recovered from public_id middle segment"
        );
    }

    /// name=null + a public_id with fewer than three `-`-segments has no
    /// recoverable profile and is skipped — and must not mis-match the prefix.
    #[test]
    fn merge_federated_overrides_no_name_unparseable_public_id_skips() {
        for id in ["aria-onlyhex", "standalone"] {
            let mut spec = cp_zellij_spec(id, "unused");
            spec.name = None;
            let mut specs = vec![spec];
            // Includes a ctx named "aria" to catch a buggy parser that would
            // return the first segment instead of None.
            let ctxs = vec![fleet_ctx("aria", "z"), fleet_ctx("onlyhex", "z2")];
            merge_federated_overrides(&mut specs, &ctxs);
            assert!(
                specs[0].launch_overrides.zellij_session.is_none(),
                "public_id {id:?} has no recoverable profile; must not merge"
            );
            assert!(specs[0].local_alias_id.is_none());
        }
    }

    /// name=null + a recoverable profile that matches no local context is
    /// skipped (no PTY bridge until enrollment is corrected).
    #[test]
    fn merge_federated_overrides_no_name_public_id_no_ctx_match_skips() {
        let mut spec = cp_zellij_spec("aria-ghost-abc", "unused");
        spec.name = None;
        let mut specs = vec![spec];
        let ctxs = vec![fleet_ctx("engineer", "aria-engineer")];
        merge_federated_overrides(&mut specs, &ctxs);

        assert!(specs[0].launch_overrides.zellij_session.is_none());
        assert!(specs[0].local_alias_id.is_none());
    }

    /// The public_id parser itself: prefix + hex stripped, middle is the profile.
    #[test]
    fn profile_from_public_id_extracts_middle_segment() {
        assert_eq!(profile_from_public_id("aria-research-22e6cd17"), Some("research"));
        assert_eq!(profile_from_public_id("aria-work-c5ff410a"), Some("work"));
        assert_eq!(profile_from_public_id("aria-foo-bar-9f3c"), Some("foo-bar"));
        assert_eq!(profile_from_public_id("aria-onlyhex"), None);
        assert_eq!(profile_from_public_id("standalone"), None);
        assert_eq!(profile_from_public_id(""), None);
    }

    /// Regression (review finding): a derived *hyphenated* profile key must not
    /// suffix-match a shorter unrelated context. Derived key "foo-bar" with only
    /// a lone "bar" context must NOT merge — the suffix clause is gated off for
    /// public_id-derived keys (otherwise it would silently bridge the wrong PTY).
    #[test]
    fn merge_federated_overrides_derived_hyphenated_key_no_suffix_false_match() {
        let mut spec = cp_zellij_spec("aria-foo-bar-9f3c", "unused");
        spec.name = None; // force the derived key "foo-bar"
        let mut specs = vec![spec];
        let ctxs = vec![fleet_ctx("bar", "aria-bar")]; // lone, unrelated
        merge_federated_overrides(&mut specs, &ctxs);

        assert!(
            specs[0].launch_overrides.zellij_session.is_none(),
            "derived 'foo-bar' must not suffix-match lone 'bar'"
        );
        assert!(specs[0].local_alias_id.is_none());
    }

    /// `name`, when present, wins over the public_id-derived profile even when
    /// the two would resolve to different contexts.
    #[test]
    fn merge_federated_overrides_name_takes_precedence_over_public_id() {
        // name → "operator"; public_id middle segment → "research". Both exist.
        let spec = cp_zellij_spec("aria-research-abc", "operator");
        let mut specs = vec![spec];
        let ctxs = vec![
            fleet_ctx("operator", "aria-operator"),
            fleet_ctx("research", "aria-research"),
        ];
        merge_federated_overrides(&mut specs, &ctxs);

        assert_eq!(
            specs[0].local_alias_id.as_deref(),
            Some("operator"),
            "controlplane name must take precedence over public_id-derived profile"
        );
    }

    /// Spec that already has a zellij_session is not overwritten.
    #[test]
    fn merge_federated_overrides_does_not_overwrite_existing_override() {
        let mut specs = vec![{
            let mut s = cp_zellij_spec("aria-operator-xyz", "aria-operator");
            s.launch_overrides.zellij_session = Some("already-set".into());
            s
        }];
        let ctxs = vec![fleet_ctx("operator", "should-not-win")];
        merge_federated_overrides(&mut specs, &ctxs);

        assert_eq!(
            specs[0].launch_overrides.zellij_session.as_deref(),
            Some("already-set"),
            "pre-existing override must not be overwritten"
        );
    }

    /// No match → spec left unchanged.
    #[test]
    fn merge_federated_overrides_no_match_leaves_spec_unchanged() {
        let mut specs = vec![cp_zellij_spec("aria-unknown-abc", "aria-unknown")];
        let ctxs = vec![fleet_ctx("engineer", "aria-engineer")];
        merge_federated_overrides(&mut specs, &ctxs);

        assert!(specs[0].launch_overrides.zellij_session.is_none());
        assert!(specs[0].local_alias_id.is_none());
    }

    /// Hyphenated profile name collision: a cp_name of "aria-foo-work" would
    /// suffix-match both "work" and "foo-work". The merge must skip on
    /// ambiguity and leave the spec unchanged — not non-deterministically
    /// pick one of the two matches.
    #[test]
    fn merge_federated_overrides_hyphenated_profile_name_no_false_match() {
        // Two fleet contexts: "work" and "foo-work".
        // cp_name "aria-foo-work" suffix-matches both (ends with "-work" AND
        // ends with "-foo-work").
        let ctxs = vec![
            fleet_ctx("work", "work-session"),
            fleet_ctx("foo-work", "foo-work-session"),
        ];
        let mut specs = vec![cp_zellij_spec("aria-foo-work-abc", "aria-foo-work")];
        merge_federated_overrides(&mut specs, &ctxs);

        // Both contexts match → ambiguous → spec must be left unchanged.
        assert!(
            specs[0].launch_overrides.zellij_session.is_none(),
            "ambiguous match must not set zellij_session (got {:?})",
            specs[0].launch_overrides.zellij_session
        );
        assert!(
            specs[0].local_alias_id.is_none(),
            "ambiguous match must not set local_alias_id"
        );
    }

    /// A profile name that is a plain hyphenated single match (e.g.
    /// "foo-bar" in fleet, cp_name "aria-foo-bar") must still match when
    /// there is no other context that would also match.
    #[test]
    fn merge_federated_overrides_hyphenated_profile_name_single_match() {
        let ctxs = vec![fleet_ctx("foo-bar", "foo-bar-session")];
        let mut specs = vec![cp_zellij_spec("aria-foo-bar-abc", "aria-foo-bar")];
        merge_federated_overrides(&mut specs, &ctxs);

        assert_eq!(
            specs[0].launch_overrides.zellij_session.as_deref(),
            Some("foo-bar-session"),
            "unambiguous hyphenated profile name must match"
        );
        assert_eq!(
            specs[0].local_alias_id.as_deref(),
            Some("foo-bar")
        );
    }

    /// Multiple fleet contexts: only the matching one is applied.
    #[test]
    fn merge_federated_overrides_multiple_fleet_contexts() {
        let mut specs = vec![
            cp_zellij_spec("aria-operator-001", "aria-operator"),
            cp_zellij_spec("aria-engineer-002", "aria-engineer"),
        ];
        let ctxs = vec![
            fleet_ctx("operator", "operator-session"),
            fleet_ctx("engineer", "engineer-session"),
        ];
        merge_federated_overrides(&mut specs, &ctxs);

        assert_eq!(
            specs[0].launch_overrides.zellij_session.as_deref(),
            Some("operator-session")
        );
        assert_eq!(specs[0].local_alias_id.as_deref(), Some("operator"));

        assert_eq!(
            specs[1].launch_overrides.zellij_session.as_deref(),
            Some("engineer-session")
        );
        assert_eq!(specs[1].local_alias_id.as_deref(), Some("engineer"));
    }

    // ── Gap 1 regression: merge works with source="aria" (the real live source) ──

    /// Contexts with source="aria" (the real source the live fleet uses —
    /// verified via `sqlite3 registry.db "select distinct source from
    /// agent_launch_context"`) must merge successfully. If this test had existed
    /// before the v2 patch, it would have caught Gap 1: the old code called
    /// `list_launch_contexts_by_source("fleet_import")` which returns an empty
    /// slice for source="aria", so fleet_ctxs was always empty and no alias was
    /// ever emitted.
    ///
    /// merge_federated_overrides is source-agnostic (takes a plain slice of
    /// AgentLaunchContext regardless of their source field). Gap 1 was in the
    /// CALLER — persist_and_resolve_specs used the wrong query. This test pins
    /// the invariant that the merge function accepts contexts regardless of
    /// source, ensuring any future change to the caller that re-introduces a
    /// source filter would need to update this test first.
    #[test]
    fn merge_federated_overrides_works_with_aria_source() {
        // Construct a context with source="aria" — the real source string found
        // in the live registry. fleet_ctx() hardcodes SOURCE_FLEET_IMPORT; build
        // manually here to use the actual live source value.
        let aria_ctx = AgentLaunchContext {
            source: "aria".to_string(), // the REAL live source — not "fleet_import"
            agent_id: "engineer".to_string(),
            vault_folder: Some("engineer".to_string()),
            state_dir_spec: None,
            zellij_session: Some("aria-engineer".to_string()),
            systemd_service: Some("aria-engineer.service".to_string()),
            supervise_paused: false,
        };
        let mut specs = vec![cp_zellij_spec("aria-engineer-abc12345", "aria-engineer")];
        merge_federated_overrides(&mut specs, &[aria_ctx]);

        assert_eq!(
            specs[0].launch_overrides.zellij_session.as_deref(),
            Some("aria-engineer"),
            "merge must work when context has source='aria' (the live fleet source)"
        );
        assert_eq!(
            specs[0].local_alias_id.as_deref(),
            Some("engineer"),
            "local_alias_id must be set from an 'aria'-sourced context"
        );
    }

    // ── Gap 2 regression: initial-boot path emits aliases in federated mode ──

    /// The merge_federated_overrides function must work on the initial boot
    /// spec list (controlplane-fetched specs with empty launch_overrides) the
    /// same way it works in persist_and_resolve_specs. This pins the invariant
    /// that calling merge_federated_overrides on a fresh controlplane spec slice
    /// produces the same alias/zellij_session outcome as the WS/poll path —
    /// ensuring the boot-time merge added in Gap 2 is functionally equivalent.
    ///
    /// The daemon boot path (daemon.rs ~228) now runs:
    ///   merge_federated_overrides(&mut agent_specs, &boot_ctxs)
    /// after resolve_agent_specs. This test verifies that a batch of
    /// controlplane specs (all with empty launch_overrides, all from a fresh
    /// fetch_node_agents call) gets aliases correctly set at boot.
    #[test]
    fn merge_federated_overrides_boot_path_sets_all_aliases() {
        // Simulates what fetch_node_agents produces in federated mode:
        // 5 specs, names set, launch_overrides empty, local_alias_id None.
        let profile_names = ["operator", "engineer", "research", "merlinlabs", "work"];
        let mut specs: Vec<AgentSpec> = profile_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                cp_zellij_spec(
                    &format!("aria-{name}-{:08x}", i),
                    &format!("aria-{name}"),
                )
            })
            .collect();

        // Contexts as they exist in the live registry (source="aria").
        let ctxs: Vec<AgentLaunchContext> = profile_names
            .iter()
            .map(|name| AgentLaunchContext {
                source: "aria".to_string(),
                agent_id: name.to_string(),
                vault_folder: Some(name.to_string()),
                state_dir_spec: None,
                zellij_session: Some(format!("aria-{name}")),
                systemd_service: Some(format!("aria-{name}.service")),
                supervise_paused: false,
            })
            .collect();

        merge_federated_overrides(&mut specs, &ctxs);

        // Every spec should now have zellij_session and local_alias_id set.
        for (spec, name) in specs.iter().zip(profile_names.iter()) {
            assert_eq!(
                spec.launch_overrides.zellij_session.as_deref(),
                Some(format!("aria-{name}").as_str()),
                "boot path: spec '{}' should have zellij_session set",
                spec.agent_id
            );
            assert_eq!(
                spec.local_alias_id.as_deref(),
                Some(*name),
                "boot path: spec '{}' should have local_alias_id set",
                spec.agent_id
            );
        }
    }

    // ── apply_runtime_overrides tests ────────────────────────────────────────

    /// apply_runtime_overrides flips runtime_kind to the local override AND
    /// clears launch_overrides.zellij_session (M2 fix: stale session name must
    /// not leak into an ACP spawn path). Also verifies that profile_path is
    /// back-filled from state_dir_spec when the spec previously had none.
    #[test]
    fn apply_runtime_overrides_flips_runtime_and_clears_session() {
        use edgeplaned_core::types::StateDirSpec;

        let mut specs = vec![AgentSpec {
            agent_id: "aria-work-deadbeef".to_string(),
            domain_id: "m-1".to_string(),
            runtime_kind: "zellij_hosted".to_string(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: SpawnOverrides {
                vault_folder: None,
                state_dir_spec: Some(StateDirSpec::Persistent {
                    path: PathBuf::from("/tmp/test-work"),
                }),
                zellij_session: Some("work-session".to_string()),
            },
            name: Some("aria-work".to_string()),
            local_alias_id: Some("work".to_string()),
        }];

        apply_runtime_overrides(&mut specs, &[("work".to_string(), "claude_agent_acp".to_string())]);

        let spec = &specs[0];
        assert_eq!(spec.runtime_kind, "claude_agent_acp", "runtime_kind must be flipped");
        assert!(
            spec.launch_overrides.zellij_session.is_none(),
            "zellij_session must be cleared when overriding to a non-zellij runtime"
        );
        assert_eq!(
            spec.profile_path.as_deref(),
            Some(PathBuf::from("/tmp/test-work").as_path()),
            "profile_path must be back-filled from state_dir_spec"
        );
    }

    /// When no override matches the spec's local_alias_id, apply_runtime_overrides
    /// must leave the spec completely unchanged.
    #[test]
    fn apply_runtime_overrides_no_match_is_noop() {
        let mut specs = vec![AgentSpec {
            agent_id: "aria-operator-cafebabe".to_string(),
            domain_id: "m-1".to_string(),
            runtime_kind: "zellij_hosted".to_string(),
            session_mode: SessionMode::Persistent,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: SpawnOverrides {
                vault_folder: None,
                state_dir_spec: None,
                zellij_session: Some("aria-operator".to_string()),
            },
            name: Some("aria-operator".to_string()),
            local_alias_id: Some("operator".to_string()),
        }];

        // Override list targets "work", not "operator" — no match expected.
        apply_runtime_overrides(&mut specs, &[("work".to_string(), "claude_agent_acp".to_string())]);

        let spec = &specs[0];
        assert_eq!(spec.runtime_kind, "zellij_hosted", "runtime_kind must be unchanged");
        assert_eq!(
            spec.launch_overrides.zellij_session.as_deref(),
            Some("aria-operator"),
            "zellij_session must be unchanged when no override matches"
        );
        assert!(spec.profile_path.is_none(), "profile_path must remain None");
    }
}
