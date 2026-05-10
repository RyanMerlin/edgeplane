/// mc-mesh daemon — wires config, supervisor, runtimes, and task loops together.
use anyhow::Result;
use mc_mesh_core::capability_dispatcher::CapabilityDispatcher;
use mc_mesh_core::client::BackendClient;
use mc_mesh_core::machine::MachineInfo;
use mc_mesh_core::paths;
use mc_mesh_packs::{PackRegistry, PolicyBundle};
use mc_mesh_runtimes::{
    claude_code::ClaudeCodeRuntime,
    codex::CodexRuntime,
    gemini::GeminiRuntime,
    goose::GooseRuntime,
};
use mc_mesh_receipts::ReceiptStore;
use mc_mesh_work::watchdog::{OfflinePolicy, Watchdog};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::attach_gateway;
use crate::attach_registry::AttachRegistry;
use crate::attach_ws;
use crate::config::{DaemonConfig, SessionMode};
use crate::mgmt_gateway::MgmtGateway;
use crate::secrets_gateway::SecretsGateway;
use crate::session_supervisor;
use crate::supervisor::Supervisor;
use crate::task_loop;

/// Config passed from the CLI, overrides any file-based config.
pub struct CliOverrides {
    pub backend_url: String,
    pub token: String,
    pub work_dir: PathBuf,
    pub offline_grace_secs: u64,
}

pub async fn run(cli: CliOverrides) -> Result<()> {
    let mut cfg = DaemonConfig::load_or_default();

    // CLI args win over file.
    if !cli.backend_url.is_empty() {
        cfg.backend_url = cli.backend_url;
    }
    if !cli.token.is_empty() {
        cfg.token = cli.token;
    }
    cfg.work_dir = cli.work_dir;
    cfg.offline_grace_secs = cli.offline_grace_secs;

    tracing::info!("mc-mesh daemon starting");
    tracing::info!("backend: {}", cfg.backend_url);
    tracing::info!("work_dir: {}", cfg.work_dir.display());
    tracing::info!(
        "missions: {:?}",
        cfg.missions.iter().map(|m| &m.mission_id).collect::<Vec<_>>()
    );

    std::fs::create_dir_all(&cfg.work_dir)?;

    let client = Arc::new(BackendClient::new(&cfg.backend_url, &cfg.token));

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

    // For each mission → each enrolled agent, spawn the runtime and start a task loop.
    for mission in &cfg.missions {
        for agent_entry in &mission.agents {
            let extra_caps: Vec<mc_mesh_core::types::Capability> = agent_entry
                .capabilities
                .iter()
                .map(|s| mc_mesh_core::types::Capability::new(s.clone()))
                .collect();
            let rt: Arc<mc_mesh_core::agent_runtime::DynAgentRuntime> =
                match agent_entry.runtime_kind.as_str() {
                    "claude_code" => Arc::new(Box::new(ClaudeCodeRuntime::with_extra_capabilities(
                        extra_caps,
                    ))),
                    "codex" => Arc::new(Box::new(CodexRuntime::with_extra_capabilities(extra_caps))),
                    "gemini" => Arc::new(Box::new(GeminiRuntime::with_extra_capabilities(
                        extra_caps,
                    ))),
                    "goose" => Arc::new(Box::new(GooseRuntime::with_extra_capabilities(extra_caps))),
                    other => {
                        tracing::warn!("Unknown runtime kind '{other}', skipping agent {}", agent_entry.agent_id);
                        continue;
                    }
                };

            // Ensure the agent CLI is installed and harness is rendered before spawning.
            if let Err(e) = rt.ensure_installed().await {
                tracing::error!(
                    "ensure_installed failed for agent {} (runtime {}): {e:#}. Skipping.",
                    agent_entry.agent_id,
                    agent_entry.runtime_kind
                );
                continue;
            }

            // Register in runtime map for attach gateway.
            {
                let mut map = runtime_map.lock().await;
                map.insert(agent_entry.agent_id.clone(), rt.clone());
            }

            supervisor
                .spawn(
                    agent_entry.agent_id.clone(),
                    mission.mission_id.clone(),
                    rt.clone(),
                    vec![],
                )
                .await?;

            // Fetch the handle from supervisor to share with the task loop.
            // We use a lightweight Arc<Mutex<AgentHandle>> so the task loop can
            // lock it before calling inject_task.
            let handle = supervisor
                .with_agent(&agent_entry.agent_id, |a| {
                    // We can't move the handle out; create a placeholder.
                    // The real handle lives inside supervisor; we pass the agent_id.
                    a.agent_id.clone()
                })
                .await;

            if handle.is_none() {
                continue;
            }

            // Build a synthetic AgentHandle for the task loop.
            // Each task loop owns its handle; the supervisor tracks them by agent_id.
            let agent_handle = Arc::new(Mutex::new(mc_mesh_core::types::AgentHandle {
                agent_id: agent_entry.agent_id.clone(),
                runtime_kind: rt.kind(),
                pid: 0,
            }));

            match agent_entry.session_mode {
                SessionMode::Task => {
                    let jh = tokio::spawn(task_loop::run_for_agent(
                        agent_handle,
                        rt.clone(),
                        client.clone(),
                        mission.mission_id.clone(),
                        agent_entry.agent_id.clone(),
                        watchdog.clone(),
                    ));
                    task_handles.push(jh);
                }
                SessionMode::Persistent => {
                    // Persistent agents: a session supervisor owns the PTY
                    // and registers itself in the attach registry. A message
                    // relay still runs so peer messages reach the live
                    // session via signal_tx.
                    let supervisor_jh = tokio::spawn(session_supervisor::run_for_agent(
                        agent_entry.agent_id.clone(),
                        rt.clone(),
                        attach_registry.clone(),
                    ));
                    task_handles.push(supervisor_jh);

                    let relay_agent = agent_handle.clone();
                    let relay_runtime = rt.clone();
                    let relay_client = client.clone();
                    let relay_agent_id = agent_entry.agent_id.clone();
                    let relay_registry = attach_registry.clone();
                    let relay_jh = tokio::spawn(async move {
                        task_loop::run_message_relay(
                            relay_agent,
                            relay_runtime,
                            relay_client,
                            relay_agent_id,
                            Some(relay_registry),
                        )
                        .await;
                    });
                    task_handles.push(relay_jh);
                }
            }

            tracing::info!(
                "Started {} loop for {} agent {} in mission {}",
                match agent_entry.session_mode {
                    SessionMode::Task => "task",
                    SessionMode::Persistent => "persistent-session",
                },
                agent_entry.runtime_kind,
                agent_entry.agent_id,
                mission.mission_id
            );
        }
    }

    if task_handles.is_empty() {
        tracing::warn!(
            "No agents configured. Add missions/agents to {} and restart.",
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
    let session_store = Arc::new(mc_mesh_secrets::SessionStore::new());
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
        let mgmt_gw = MgmtGateway::new(dispatcher, registry);
        tokio::spawn(async move {
            if let Err(e) = mgmt_gw.run().await {
                tracing::error!("mgmt gateway error: {e}");
            }
        });
    }

    // Wait for ctrl-c or all loops to exit.
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

    // Clean up sockets on exit.
    let _ = std::fs::remove_file(attach_gateway::socket_path());
    let _ = std::fs::remove_file(paths::secrets_socket_path());
    Ok(())
}
