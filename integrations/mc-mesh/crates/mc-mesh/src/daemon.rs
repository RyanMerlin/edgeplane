/// mc-mesh daemon — wires config, supervisor, runtimes, and task loops together.
use anyhow::Result;
use mc_mesh_core::agent_runtime::AgentRuntime;
use mc_mesh_core::capability_dispatcher::CapabilityDispatcher;
use mc_mesh_core::client::BackendClient;
use mc_mesh_core::machine::MachineInfo;
use mc_mesh_core::paths;
use mc_mesh_packs::{PackRegistry, PolicyBundle};
use mc_mesh_runtimes::{
    claude_agent_acp::ClaudeAgentAcpRuntime,
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

use crate::acp_session_supervisor::{self, AcpSupervisorConfig};
use crate::attach_gateway;
use crate::attach_registry::AttachRegistry;
use crate::attach_ws;
use crate::config::{DaemonConfig, SessionMode};
use crate::mgmt_gateway::MgmtGateway;
use crate::secrets_gateway::SecretsGateway;
use crate::session_supervisor;
use crate::state;
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

    // Phase 4b: state file is the source of truth for node identity. yaml
    // fields are accepted for one release as a deprecation path; if they're
    // present and state is empty we migrate, then warn-and-strip on next
    // save. The plan: docs/plans/2026-05-10-mc-mesh-controlplane-driven-enrollment.md
    if let Err(e) = merge_state_file(&mut cfg).await {
        tracing::warn!("state file load failed: {e:#}. Continuing with yaml-only fields.");
    }

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

    // Phase 4c: build the flat list of agents to spawn from the controlplane
    // first (state.node_id present → GET /runtime/nodes/{id}/agents), falling
    // back to legacy yaml-defined missions only when no node identity is
    // registered. The yaml path is logged with a deprecation warning when
    // both sources are populated.
    let agent_specs = resolve_agent_specs(&cfg, &client).await;

    // Spawn each agent. mission_id now travels per-agent (controlplane is the
    // source of truth) instead of being grouped under a yaml mission.
    for spec in &agent_specs {
        let extra_caps: Vec<mc_mesh_core::types::Capability> = spec
            .capabilities
            .iter()
            .map(|s| mc_mesh_core::types::Capability::new(s.clone()))
            .collect();
        let work_dir = paths::mc_mesh_work_dir().join(&spec.agent_id);

        // For ACP+persistent we need SpawnOpts to feed the ACP supervisor
        // directly. Capture them here while the runtime is still
        // concrete-typed; once it's behind `Box<dyn AgentRuntime>` we
        // can't call `spawn_opts` on it.
        let mut acp_spawn_opts: Option<mc_mesh_acp::SpawnOpts> = None;

        let rt: Arc<mc_mesh_core::agent_runtime::DynAgentRuntime> =
            match spec.runtime_kind.as_str() {
                "claude_code" => Arc::new(Box::new(ClaudeCodeRuntime::with_extra_capabilities(
                    extra_caps,
                ))),
                "claude_agent_acp" => {
                    let concrete =
                        ClaudeAgentAcpRuntime::with_extra_capabilities(extra_caps);
                    if let Err(e) = std::fs::create_dir_all(&work_dir) {
                        tracing::error!(
                            "failed to create work dir for {}: {e}",
                            spec.agent_id
                        );
                        continue;
                    }
                    if let Err(e) = concrete.ensure_installed().await {
                        tracing::error!(
                            "ensure_installed failed for ACP agent {}: {e:#}. Skipping.",
                            spec.agent_id
                        );
                        continue;
                    }
                    match concrete.spawn_opts(&work_dir) {
                        Ok(opts) => acp_spawn_opts = Some(opts),
                        Err(e) => {
                            tracing::error!(
                                "could not resolve ACP spawn opts for {}: {e:#}. Skipping.",
                                spec.agent_id
                            );
                            continue;
                        }
                    }
                    Arc::new(Box::new(concrete))
                }
                "codex" => Arc::new(Box::new(CodexRuntime::with_extra_capabilities(extra_caps))),
                "gemini" => Arc::new(Box::new(GeminiRuntime::with_extra_capabilities(
                    extra_caps,
                ))),
                "goose" => Arc::new(Box::new(GooseRuntime::with_extra_capabilities(extra_caps))),
                other => {
                    tracing::warn!("Unknown runtime kind '{other}', skipping agent {}", spec.agent_id);
                    continue;
                }
            };

        // Ensure the agent CLI is installed and harness is rendered before spawning.
        if let Err(e) = rt.ensure_installed().await {
            tracing::error!(
                "ensure_installed failed for agent {} (runtime {}): {e:#}. Skipping.",
                spec.agent_id,
                spec.runtime_kind
            );
            continue;
        }

        // Register in runtime map for attach gateway.
        {
            let mut map = runtime_map.lock().await;
            map.insert(spec.agent_id.clone(), rt.clone());
        }

        supervisor
            .spawn(
                spec.agent_id.clone(),
                spec.mission_id.clone(),
                rt.clone(),
                vec![],
            )
            .await?;

        // Fetch the handle from supervisor to share with the task loop.
        let handle = supervisor
            .with_agent(&spec.agent_id, |a| a.agent_id.clone())
            .await;

        if handle.is_none() {
            continue;
        }

        // Build a synthetic AgentHandle for the task loop.
        let agent_handle = Arc::new(Mutex::new(mc_mesh_core::types::AgentHandle {
            agent_id: spec.agent_id.clone(),
            runtime_kind: rt.kind(),
            pid: 0,
        }));

        match spec.session_mode {
            SessionMode::Task => {
                let jh = tokio::spawn(task_loop::run_for_agent(
                    agent_handle,
                    rt.clone(),
                    client.clone(),
                    spec.mission_id.clone(),
                    spec.agent_id.clone(),
                    watchdog.clone(),
                ));
                task_handles.push(jh);
            }
            SessionMode::Persistent => {
                // Persistent agents: a session supervisor owns the live
                // session and registers itself in the attach registry.
                // ACP runtimes use a JSON-RPC-aware supervisor; everything
                // else uses the byte-stream PTY supervisor. A message relay
                // still runs so peer messages reach the session via
                // signal_tx (registered by either supervisor).
                let supervisor_jh = if spec.runtime_kind == "claude_agent_acp" {
                    let opts = acp_spawn_opts.clone().expect(
                        "acp_spawn_opts populated when runtime_kind == claude_agent_acp",
                    );
                    let scfg = AcpSupervisorConfig {
                        agent_id: spec.agent_id.clone(),
                        spawn_opts: opts,
                        cwd: work_dir.clone(),
                    };
                    tokio::spawn(acp_session_supervisor::run_for_agent(
                        scfg,
                        attach_registry.clone(),
                    ))
                } else {
                    tokio::spawn(session_supervisor::run_for_agent(
                        spec.agent_id.clone(),
                        rt.clone(),
                        attach_registry.clone(),
                    ))
                };
                task_handles.push(supervisor_jh);

                let relay_agent = agent_handle.clone();
                let relay_runtime = rt.clone();
                let relay_client = client.clone();
                let relay_agent_id = spec.agent_id.clone();
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
            match spec.session_mode {
                SessionMode::Task => "task",
                SessionMode::Persistent => "persistent-session",
            },
            spec.runtime_kind,
            spec.agent_id,
            spec.mission_id
        );
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

/// Merge `~/.mc/mc-mesh.state.json` into `cfg`. State wins over yaml (the
/// new model: state is daemon-managed, yaml is config). If yaml had values
/// and state did not, migrate them and warn loudly so the operator removes
/// them from the yaml.
async fn merge_state_file(cfg: &mut DaemonConfig) -> Result<()> {
    let path = state::NodeState::default_path()?;
    let existing = state::NodeState::read(&path)?;

    match existing {
        Some(s) => {
            // Source of truth — state always wins. If yaml carries stale
            // duplicates, warn so they get cleaned up.
            if let Some(yaml_node_id) = cfg.node_id.as_ref() {
                if yaml_node_id != &s.node_id {
                    tracing::warn!(
                        "yaml has node_id={yaml_node_id} but state file has {}; state wins. Remove node_id from yaml.",
                        s.node_id
                    );
                }
            }
            if cfg.attach_secret.is_some() {
                tracing::warn!(
                    "yaml carries an `attach_secret`; state file is the source of truth — remove from yaml. \
                     (Daemon does not log secret values.)"
                );
            }
            cfg.node_id = Some(s.node_id);
            cfg.attach_secret = Some(s.attach_secret);
        }
        None => {
            // No state file yet. If yaml carries the legacy values, migrate
            // them so the next start uses the new path. The yaml fields are
            // left in place this release; a future release will hard-fail
            // on them.
            if let (Some(node_id), Some(secret)) = (cfg.node_id.clone(), cfg.attach_secret.clone()) {
                tracing::warn!(
                    "Migrating node_id + attach_secret from yaml to state file at {}. \
                     Remove these fields from your mc-mesh.yaml — a future release will hard-fail on them.",
                    path.display()
                );
                let migrated = state::NodeState {
                    schema_version: state::STATE_SCHEMA_VERSION,
                    node_id,
                    attach_secret: secret,
                    registered_at: chrono::Utc::now().to_rfc3339(),
                    controlplane_url: cfg.backend_url.clone(),
                };
                migrated.write_atomic(&path)?;
            } else {
                // No state, no yaml values. Daemon runs without registered
                // identity — heartbeat + attach_ws are no-ops. The operator
                // is expected to run `mc-mesh node-register` first; we
                // surface that explicitly in Phase 4c when GET /agents
                // becomes the only source of agent assignment.
                tracing::info!(
                    "No state file at {} and no node_id in yaml; daemon will run without a registered identity. \
                     Run `mc-mesh node-register --bootstrap-token <jt_…>` to register this node.",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

// ── Phase 4c: agent assignment resolution ────────────────────────────────────

/// Internal flat representation of one agent the daemon should spawn.
/// Built from either the controlplane (preferred when state.node_id is set)
/// or yaml-defined missions (legacy fallback during the deprecation window).
#[derive(Debug, Clone)]
struct AgentSpec {
    agent_id: String,
    mission_id: String,
    runtime_kind: String,
    session_mode: SessionMode,
    capabilities: Vec<String>,
    #[allow(dead_code)] // consumed by future ACP supervisor profile-loading work
    profile_path: Option<PathBuf>,
}

/// Build the spawn list. Controlplane wins when state.node_id is set; yaml
/// is the fallback for unregistered nodes. If both populate (legacy yaml
/// during migration), warn and prefer the controlplane.
async fn resolve_agent_specs(cfg: &DaemonConfig, client: &BackendClient) -> Vec<AgentSpec> {
    if let Some(node_id) = cfg.node_id.as_deref() {
        match fetch_node_agents(client, node_id).await {
            Ok(specs) => {
                if !cfg.missions.is_empty() {
                    let yaml_missions: Vec<&str> =
                        cfg.missions.iter().map(|m| m.mission_id.as_str()).collect();
                    tracing::warn!(
                        "yaml carries `missions:` ({:?}) but node {} is registered with the controlplane; \
                         using controlplane assignment list and ignoring yaml. \
                         Remove `missions:` from your mc-mesh.yaml — a future release will hard-fail on it.",
                        yaml_missions,
                        node_id
                    );
                }
                tracing::info!(
                    "Resolved {} agent assignment(s) from controlplane for node {}",
                    specs.len(),
                    node_id
                );
                return specs;
            }
            Err(e) => {
                tracing::error!(
                    "GET /runtime/nodes/{node_id}/agents failed: {e:#}. \
                     Falling back to yaml-defined missions for this start; \
                     rebalance will retry once the controlplane is reachable."
                );
            }
        }
    } else {
        tracing::info!(
            "No node_id in state file; using yaml-defined missions. \
             Run `mc-mesh node-register --bootstrap-token <jt_…>` to switch to controlplane-driven assignment."
        );
    }
    yaml_specs(cfg)
}

async fn fetch_node_agents(
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
    let agent_id = v
        .get("id")
        .and_then(|s| s.as_str())
        .ok_or_else(|| anyhow!("agent record missing `id`"))?
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
    Ok(AgentSpec {
        agent_id,
        mission_id,
        runtime_kind,
        session_mode,
        capabilities,
        profile_path,
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
            });
        }
    }
    out
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
