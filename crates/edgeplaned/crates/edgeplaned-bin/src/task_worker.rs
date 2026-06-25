//! Ephemeral task subagent claimer loop (Phase 2).
//!
//! # Phase 2 — claimer loop
//!
//! Polls the Edgeplane controlplane for `MeshTask` rows whose
//! `claim_policy` contains a `target_profile` matching one of the profiles this
//! edgeplaned instance supervises. For each match (up to `max_concurrent_subagents`),
//! it:
//!
//!   1. Enrolls an ephemeral `MeshAgent` under the fleet-ops domain.
//!   2. Claims the task via the agent.
//!   3. Allocates a per-task working directory (`~/.ep/worktrees/<task_id>/`).
//!   4. Starts a durable `AgentRun` audit record.
//!   5. Spawns `claude -p "<prompt>"` as a child process in the worktree.
//!   6. On subprocess exit: completes the AgentRun + MeshTask, deletes the
//!      ephemeral MeshAgent, removes the worktree.
//!
//! # Soft-fail philosophy
//!
//! Individual task failures log a warning and continue. The loop does not crash
//! the daemon — controlplane unreachable is logged as a warning and retried on
//! the next poll interval. The daemon stays alive through transient API errors.
//!
//! # Concurrency
//!
//! P2: A `tokio::sync::Semaphore` caps active subagent processes at
//! `config.task_worker_max_concurrent`. Tasks beyond the cap stay `ready` in
//! the queue and are picked up when a slot frees.
//!
//! # HTTP endpoint surface used (P2)
//!
//! - `GET /domains` — discover all domains for mission scanning.
//! - `GET /domains/{id}/k` — list missions per domain.
//! - `GET /work/missions/{id}/tasks?status=ready` — poll ready tasks per mission.
//! - `POST /work/domains/{domain_id}/agents/enroll` — enroll ephemeral MeshAgent.
//! - `POST /work/tasks/{task_id}/claim` — claim task with enrolled agent.
//! - `POST /runs` — start durable AgentRun.
//! - `POST /runs/{run_id}/complete` — complete AgentRun on subprocess exit.
//! - `POST /work/tasks/{task_id}/complete` — complete MeshTask.
//! - `DELETE /work/agents/{agent_id}` — delete ephemeral MeshAgent (Decision #1).
//!
//! # Cross-mission scan trade-off
//!
//! The controlplane exposes `GET /work/missions/{id}/tasks` per-mission but has
//! no cross-mission scan endpoint. This module scans domains → missions → tasks
//! (three round-trips per poll). For the typical single-node fleet this adds
//! < 100ms per poll cycle and is acceptable. If mission count grows beyond ~50,
//! a dedicated `GET /work/tasks?status=ready&target_profile=X` index endpoint
//! should be added to the controlplane (tracked as a tech-debt item).
//!
//! See `docs/design/ephemeral-task-subagents.md` for full design rationale.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use edgeplaned_core::client::BackendClient;
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::capabilities::{parse_required_capabilities, resolve_capabilities};
use crate::config::DaemonConfig;
use crate::local_registry::LocalRegistry;

/// Entry point spawned by `daemon.rs`. Loops forever, polling for claimable
/// tasks and spawning subagents up to the concurrency cap.
///
/// Returns `()` only on graceful shutdown (which is not yet implemented —
/// the daemon exits via Ctrl-C / signal, so this task is simply abandoned).
pub async fn run(client: Arc<BackendClient>, config: DaemonConfig) {
    if !config.task_worker_enabled {
        tracing::info!("task_worker: disabled by config, not starting poll loop");
        return;
    }

    let poll_interval = std::time::Duration::from_secs(config.task_worker_poll_interval_secs);
    let semaphore = Arc::new(Semaphore::new(config.task_worker_max_concurrent));

    tracing::info!(
        "task_worker: starting poll loop (interval={}s, max_concurrent={})",
        config.task_worker_poll_interval_secs,
        config.task_worker_max_concurrent,
    );

    // Track in-flight task IDs so the poll loop doesn't try to claim a task
    // that's already being processed in a concurrent slot.
    let in_flight: Arc<tokio::sync::Mutex<HashSet<String>>> =
        Arc::new(tokio::sync::Mutex::new(HashSet::new()));

    loop {
        // Discover which profiles this edgeplaned instance supervises.
        let supervised = discover_supervised_profiles();
        if supervised.is_empty() {
            tracing::debug!(
                "task_worker: no supervised profiles in local registry, \
                 skipping poll (will retry)"
            );
        } else {
            poll_and_claim(&client, &config, &semaphore, &in_flight, &supervised).await;
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Read the local registry and return the set of agent IDs this edgeplaned
/// node supervises. Returns agents from any source that have a launch context
/// (i.e. zellij_hosted / persistent agents registered via
/// `edgeplane daemon agent import` or the legacy `fleet_import` path).
/// Falls back to an empty set on any registry error so the daemon continues
/// without crashing.
fn discover_supervised_profiles() -> HashSet<String> {
    let db_path = match LocalRegistry::default_path() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("task_worker: could not resolve registry path: {e:#}");
            return HashSet::new();
        }
    };

    if !db_path.exists() {
        // Registry not yet initialised — not an error on first startup.
        tracing::debug!("task_worker: local registry not found, no supervised profiles");
        return HashSet::new();
    }

    let reg = match LocalRegistry::open(&db_path) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("task_worker: could not open local registry: {e:#}");
            return HashSet::new();
        }
    };

    match reg.list_all_launch_contexts() {
        Ok(contexts) => contexts.into_iter().map(|c| c.agent_id).collect(),
        Err(e) => {
            tracing::warn!("task_worker: could not list launch contexts: {e:#}");
            HashSet::new()
        }
    }
}

/// One poll iteration: discover ready claimable tasks and spawn subagents for
/// each match that fits under the concurrency cap.
async fn poll_and_claim(
    client: &Arc<BackendClient>,
    config: &DaemonConfig,
    semaphore: &Arc<Semaphore>,
    in_flight: &Arc<tokio::sync::Mutex<HashSet<String>>>,
    supervised: &HashSet<String>,
) {
    let tasks = match scan_ready_tasks(client, supervised).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("task_worker: scan_ready_tasks failed: {e:#}");
            return;
        }
    };

    if tasks.is_empty() {
        tracing::debug!("task_worker: no claimable tasks found");
        return;
    }

    tracing::info!("task_worker: {} claimable task(s) found", tasks.len());

    for task in tasks {
        let task_id = match task.get("id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => {
                tracing::warn!("task_worker: task missing id field, skipping: {task}");
                continue;
            }
        };

        // Skip if already being processed by a concurrent slot.
        {
            let lock = in_flight.lock().await;
            if lock.contains(&task_id) {
                tracing::debug!("task_worker: task {task_id} already in flight, skipping");
                continue;
            }
        }

        // Try to acquire a concurrency slot without blocking the poll loop.
        // If all slots are busy, leave remaining tasks for the next poll.
        let permit = match semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                tracing::debug!("task_worker: concurrency cap reached, deferring remaining tasks");
                break;
            }
        };

        // Mark as in-flight.
        {
            let mut lock = in_flight.lock().await;
            lock.insert(task_id.clone());
        }

        // Spawn the task lifecycle as an independent tokio task.
        let client_c = Arc::clone(client);
        let config_c = config.clone();
        let in_flight_c = Arc::clone(in_flight);
        let task_c = task.clone();

        tokio::spawn(async move {
            // `permit` is held for the lifetime of this task. Dropped on return,
            // freeing the semaphore slot.
            let _permit = permit;

            run_task_lifecycle(&client_c, &config_c, &task_c).await;

            // Remove from in-flight set regardless of success/failure.
            let mut lock = in_flight_c.lock().await;
            let id = task_c
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            lock.remove(&id);
        });
    }
}

/// Scan all domains → missions → tasks, returning ready tasks whose
/// `claim_policy.target_profile` matches one of the supervised profiles.
///
/// # Cross-mission scan
///
/// There is no single endpoint for cross-mission task listing. This function
/// traverses domains → missions → tasks (3 round-trips). For the current
/// fleet scale (< 20 missions) this is fine. A dedicated index endpoint
/// would be the right optimisation if mission count grows significantly.
async fn scan_ready_tasks(
    client: &BackendClient,
    supervised: &HashSet<String>,
) -> anyhow::Result<Vec<Value>> {
    let domains: Vec<Value> = client
        .get("/domains")
        .await
        .map_err(|e| anyhow::anyhow!("GET /domains failed: {e:#}"))?;

    let mut claimable = Vec::new();

    for domain in &domains {
        let domain_id = match domain.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => continue,
        };

        let missions: Vec<Value> = match client.get(&format!("/domains/{domain_id}/m")).await {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("task_worker: GET /domains/{domain_id}/m failed: {e:#}, skipping");
                continue;
            }
        };

        for mission in &missions {
            let mission_id = match mission.get("id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None => continue,
            };

            let tasks: Vec<Value> = match client
                .get(&format!("/work/missions/{mission_id}/tasks?status=ready"))
                .await
            {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        "task_worker: GET /work/missions/{mission_id}/tasks failed: {e:#}, \
                         skipping"
                    );
                    continue;
                }
            };

            for task in tasks {
                if should_claim(&task, supervised) {
                    claimable.push(task);
                }
            }
        }
    }

    Ok(claimable)
}

/// The full lifecycle for one task: enroll → claim → worktree → AgentRun →
/// spawn claude → complete/cleanup.
///
/// Logs a warning on any step failure and returns — the concurrency slot is
/// released so the next poll can retry (the task will be back to `ready`
/// once any partial claim lease expires).
async fn run_task_lifecycle(client: &BackendClient, config: &DaemonConfig, task: &Value) {
    let task_id = match task.get("id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            tracing::warn!("task_worker: lifecycle called with task missing id");
            return;
        }
    };
    let domain_id = match task.get("domain_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            tracing::warn!("task_worker: task {task_id} missing domain_id, skipping");
            return;
        }
    };

    // Resolve the target profile name from claim_policy so we can label the
    // ephemeral agent correctly.
    let target_profile = parse_target_profile_from_claim_policy(
        task.get("claim_policy")
            .and_then(|v| v.as_str())
            .unwrap_or("{}"),
    )
    .unwrap_or_else(|| "unknown".to_string());

    tracing::info!(
        "task_worker: starting lifecycle for task={task_id} profile={target_profile} \
         domain={domain_id}"
    );

    // ── Step 1: Enroll ephemeral MeshAgent ────────────────────────────────

    let enroll_body = serde_json::json!({
        "agent_name": target_profile,
        "runtime_kind": "claude_headless",
        "runtime_version": concat!("task-worker-v", env!("CARGO_PKG_VERSION")),
        "capabilities": ["shell", "fs:read", "fs:write"],
        "labels": {
            "role": "task-subagent",
            "ephemeral": true,
            "task_id": task_id,
            "target_profile": target_profile,
        }
    });

    let enroll_resp: Value = match client
        .post(
            &format!("/work/domains/{domain_id}/agents/enroll"),
            &enroll_body,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("task_worker: enroll_mesh_agent failed for task {task_id}: {e:#}");
            return;
        }
    };

    let agent_id = match enroll_resp.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            tracing::warn!(
                "task_worker: enroll response missing 'id' for task {task_id}: {enroll_resp}"
            );
            return;
        }
    };

    // Capture this ephemeral agent's own per-agent token from the enroll
    // response. The tower mints it at enrollment (Seam 2). When present, it is
    // injected as the subprocess's EP_AGENT_TOKEN so the agent authenticates as
    // itself; when absent (older tower, or mint disabled) the subprocess
    // inherits the daemon's shared EP_AGENT_TOKEN — graceful fallback.
    let agent_token = enroll_resp
        .get("agent_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    if agent_token.is_none() {
        tracing::warn!(
            "task_worker: enroll response for task {task_id} has no 'agent_token'; \
             subprocess will inherit the daemon's shared EP_AGENT_TOKEN"
        );
    }

    tracing::debug!("task_worker: enrolled ephemeral agent={agent_id} for task={task_id}");

    // ── Step 2: Claim the task ─────────────────────────────────────────────

    let claim_body = serde_json::json!({ "agent_id": agent_id });
    if let Err(e) = client
        .raw_post(&format!("/work/tasks/{task_id}/claim"), &claim_body)
        .await
    {
        tracing::warn!(
            "task_worker: claim task {task_id} with agent {agent_id} failed: {e:#}. \
             Cleaning up agent."
        );
        delete_agent_soft(client, &agent_id, task_id).await;
        return;
    }

    tracing::debug!("task_worker: claimed task={task_id} with agent={agent_id}");

    // ── Step 3: Allocate per-task worktree ────────────────────────────────
    //
    // P2: just `mkdir -p`. Real `git worktree add` is a refinement for later.
    // This ensures the subagent has an isolated cwd per the design doc.

    let worktree = worktree_path_for_task(task_id);
    if let Err(e) = std::fs::create_dir_all(&worktree) {
        tracing::warn!(
            "task_worker: could not create worktree {}: {e:#}. Aborting task {task_id}.",
            worktree.display()
        );
        // Best-effort complete the task as failed so it doesn't dangle.
        complete_task_failed(client, task_id, &agent_id).await;
        delete_agent_soft(client, &agent_id, task_id).await;
        return;
    }

    // ── Step 4: Start AgentRun (durable audit record) ─────────────────────
    //
    // NB: `/runs` expects `agent_id`/`task_id` in the request body — these
    // bind to `agentrun.mesh_agent_id`/`agentrun.mesh_task_id` server-side.
    // The naming mismatch is documented in the design doc (prototype finding #1);
    // serde aliases are now in `models::run::StartRunRequest` so both names work.

    let run_body = serde_json::json!({
        "agent_id": agent_id,
        "task_id": task_id,
        "runtime_kind": "claude_headless",
    });

    let run_resp: Value = match client.post("/runs", &run_body).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "task_worker: start AgentRun failed for task {task_id}: {e:#}. \
                 Continuing without audit record (task will still run)."
            );
            // Non-fatal: we still run the subprocess even without the run row.
            // The audit trail is imperfect but the work gets done.
            Value::Null
        }
    };

    let run_id = run_resp
        .get("id")
        .and_then(|v| v.as_str())
        .map(String::from);

    if let Some(ref rid) = run_id {
        tracing::debug!("task_worker: AgentRun started run_id={rid} for task={task_id}");
    }

    // ── Step 5: Build the prompt ──────────────────────────────────────────

    let prompt = build_prompt(task);

    // ── Step 5a: Resolve capability set → --allowed-tools ─────────────────
    //
    // `required_capabilities` is a TEXT column on MeshTask, expected to hold
    // a JSON array of coarse capability names (e.g. `["fs:read","shell:write"]`).
    //
    // Translation: each name maps to a set of `--allowed-tools` fragments via
    // the v1 capability vocabulary in `crate::capabilities`. Subsuming
    // capabilities (e.g. `vault:write` ⊇ `vault:read`) are expanded to the
    // full union; the result is deduplicated and sorted for determinism.
    //
    // Strict mode (config.task_worker_strict_capabilities = true):
    //   Missing or empty required_capabilities → fail the task immediately.
    //   Forces dispatchers to declare blast radius.
    //
    // Lenient mode (default, strict = false):
    //   Missing or empty required_capabilities → use
    //   config.task_worker_default_capabilities (default: ["fs:read","shell:read"]).
    //   If defaults are also invalid, fall back to fs:read only.

    let raw_caps = task
        .get("required_capabilities")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let parsed_caps = match parse_required_capabilities(raw_caps) {
        Ok(caps) => caps,
        Err(e) => {
            tracing::warn!(
                "task_worker: required_capabilities parse error for task={task_id}: {e:#}. \
                 Failing task."
            );
            complete_task_failed(client, task_id, &agent_id).await;
            delete_agent_soft(client, &agent_id, task_id).await;
            if let Err(rm_err) = std::fs::remove_dir_all(&worktree) {
                tracing::warn!(
                    "task_worker: could not remove worktree {}: {rm_err:#}",
                    worktree.display()
                );
            }
            return;
        }
    };

    let allowed_tools = match resolve_capabilities(
        &parsed_caps,
        config.task_worker_strict_capabilities,
        &config.task_worker_default_capabilities,
    ) {
        Ok(tools) => tools,
        Err(e) => {
            tracing::warn!(
                "task_worker: capability resolution failed for task={task_id}: {e:#}. \
                 Failing task."
            );
            complete_task_failed(client, task_id, &agent_id).await;
            delete_agent_soft(client, &agent_id, task_id).await;
            if let Err(rm_err) = std::fs::remove_dir_all(&worktree) {
                tracing::warn!(
                    "task_worker: could not remove worktree {}: {rm_err:#}",
                    worktree.display()
                );
            }
            return;
        }
    };

    let allowed_tools_str = allowed_tools.to_cli_string();
    tracing::info!(
        "task_worker: task={task_id} allowed_tools=[{}]",
        allowed_tools_str
    );

    // ── Step 6: Spawn `claude -p` subprocess ─────────────────────────────

    tracing::info!(
        "task_worker: spawning {} for task={task_id} in {}",
        config.task_worker_subagent_command,
        worktree.display()
    );

    let mut cmd = tokio::process::Command::new(&config.task_worker_subagent_command);
    cmd.arg("-p")
        .arg(&prompt)
        .arg("--output-format")
        .arg("json")
        .current_dir(&worktree)
        // Strip edgeplaned-internal env vars so the child doesn't inherit the secrets
        // gateway binding; see CLAUDE.md feedback on claude-code-acp runtime.
        .env_remove("EP_SECRETS_SOCKET")
        .env_remove("EP_SECRETS_SESSION");

    // Inject this ephemeral agent's own per-agent token so it authenticates to
    // the tower as itself rather than as the shared daemon. When the enroll
    // response carried no token, leave EP_AGENT_TOKEN inherited from the daemon
    // environment (graceful fallback).
    if let Some(ref tok) = agent_token {
        cmd.env("EP_AGENT_TOKEN", tok);
    }

    // Add --allowed-tools only when the capability set is non-empty.
    // An empty set (task declared `required_capabilities = []`) means no tools
    // are allowed — the claude CLI defaults to all tools when the flag is
    // absent, so we must pass an explicit empty string in that case.
    // However, `claude --allowed-tools ""` is equivalent to no allowed tools
    // per the CLI docs, and an empty capabilities set is a valid but unusual
    // configuration (the subagent can still output via stdout). We always pass
    // the flag so the restriction is explicit rather than silently omitted.
    cmd.arg("--allowed-tools").arg(&allowed_tools_str);

    let spawn_result = cmd.output().await;

    // ── Step 7: Cleanup regardless of spawn outcome ───────────────────────
    //
    // Use a nested block so we always clean up worktree + agent even on
    // spawn failure. The pattern mirrors the "finalizer" approach from the
    // brief — cleanup is not conditional on success.

    let subprocess_ok = match spawn_result {
        Ok(output) => {
            let exit_ok = output.status.success();
            if exit_ok {
                tracing::info!("task_worker: subprocess exited successfully for task={task_id}");
                if tracing::enabled!(tracing::Level::DEBUG) {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    tracing::debug!(
                        "task_worker: subprocess stdout (first 500 chars): {}",
                        &stdout.chars().take(500).collect::<String>()
                    );
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(
                    "task_worker: subprocess exited with status={} for task={task_id}. \
                     stderr (first 200 chars): {}",
                    output.status,
                    &stderr.chars().take(200).collect::<String>()
                );
            }
            exit_ok
        }
        Err(e) => {
            tracing::warn!(
                "task_worker: could not spawn {} for task {task_id}: {e:#}",
                config.task_worker_subagent_command
            );
            false
        }
    };

    // Complete AgentRun.
    if let Some(ref rid) = run_id {
        let status = if subprocess_ok { "completed" } else { "failed" };
        let complete_body = serde_json::json!({ "status": status });
        if let Err(e) = client
            .raw_post(&format!("/runs/{rid}/complete"), &complete_body)
            .await
        {
            tracing::warn!("task_worker: complete AgentRun {rid} failed for task {task_id}: {e:#}");
        }
    }

    // Complete MeshTask.
    if subprocess_ok {
        let complete_task_body = serde_json::json!({ "agent_id": agent_id });
        if let Err(e) = client
            .raw_post(
                &format!("/work/tasks/{task_id}/complete"),
                &complete_task_body,
            )
            .await
        {
            tracing::warn!("task_worker: complete MeshTask {task_id} failed: {e:#}");
        }
    } else {
        complete_task_failed(client, task_id, &agent_id).await;
    }

    // Delete the ephemeral MeshAgent (Decision #1 — FK ON DELETE SET NULL
    // preserves AgentRun audit trail automatically).
    delete_agent_soft(client, &agent_id, task_id).await;

    // Remove the worktree directory.
    if let Err(e) = std::fs::remove_dir_all(&worktree) {
        tracing::warn!(
            "task_worker: could not remove worktree {}: {e:#} (leaving for manual cleanup)",
            worktree.display()
        );
    }

    tracing::info!(
        "task_worker: lifecycle complete for task={task_id} agent={agent_id} \
         success={subprocess_ok}"
    );
}

/// Build the prompt string for `claude -p`. Prefers `input_json.prompt` if
/// present, falls back to the task's `description` field, and ultimately to
/// a generic fallback so claude always receives something.
fn build_prompt(task: &Value) -> String {
    // Try input_json.prompt first (structured dispatcher convention).
    if let Some(prompt) = task
        .get("input_json")
        .and_then(|ij| {
            // input_json may be a JSON string (encoded) or a JSON object.
            if let Some(s) = ij.as_str() {
                serde_json::from_str::<Value>(s).ok()
            } else {
                Some(ij.clone())
            }
        })
        .as_ref()
        .and_then(|v| v.get("prompt"))
        .and_then(|p| p.as_str())
    {
        return prompt.to_string();
    }

    // Fall back to description.
    if let Some(desc) = task.get("description").and_then(|d| d.as_str())
        && !desc.is_empty()
    {
        return desc.to_string();
    }

    // Last resort: use the task title.
    task.get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Complete the assigned task.")
        .to_string()
}

/// Best-effort: mark the MeshTask as failed. Logs on error but does not panic.
async fn complete_task_failed(client: &BackendClient, task_id: &str, agent_id: &str) {
    let body = serde_json::json!({ "agent_id": agent_id });
    if let Err(e) = client
        .raw_post(&format!("/work/tasks/{task_id}/fail"), &body)
        .await
    {
        tracing::warn!("task_worker: fail MeshTask {task_id} (best-effort) failed: {e:#}");
    }
}

/// Best-effort: delete the ephemeral MeshAgent. Logs on error but does not
/// crash. The controlplane's FK `ON DELETE SET NULL` on `agentrun.mesh_agent_id`
/// ensures AgentRun audit records survive deletion (Decision #1).
async fn delete_agent_soft(client: &BackendClient, agent_id: &str, task_id: &str) {
    if let Err(e) = client.delete(&format!("/work/agents/{agent_id}")).await {
        tracing::warn!(
            "task_worker: DELETE /work/agents/{agent_id} failed for task {task_id}: {e:#}. \
             Agent row may linger — manual cleanup: \
             DELETE FROM meshagent WHERE id='{agent_id}'"
        );
    } else {
        tracing::debug!("task_worker: deleted ephemeral agent={agent_id} for task={task_id}");
    }
}

// ── Pure logic (unit-testable without controlplane) ───────────────────────────

/// Parse the `target_profile` field from a `claim_policy` JSON string.
///
/// The dispatcher embeds target routing as:
///   `{"target_profile": "research"}`
///
/// Returns `None` if the string is not valid JSON, if `target_profile` is
/// absent, or if it is not a string value. P2 skips tasks where this returns
/// `None`; unscoped tasks remain ready until explicitly routed or surfaced.
pub fn parse_target_profile_from_claim_policy(claim_policy: &str) -> Option<String> {
    let v: Value = serde_json::from_str(claim_policy).ok()?;
    v.get("target_profile")?.as_str().map(String::from)
}

/// Decide whether a task should be claimed by this node.
///
/// Returns `true` iff:
/// - The task's `claim_policy.target_profile` parses to a non-empty string.
/// - That profile name is in `supervised_profiles`.
///
/// Skips tasks with:
/// - No `claim_policy` field (defaults to `{}`).
/// - A `claim_policy` that isn't valid JSON or lacks `target_profile`.
/// - A `target_profile` not in the supervised set.
pub fn should_claim(task: &Value, supervised_profiles: &HashSet<String>) -> bool {
    let claim_policy = task
        .get("claim_policy")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");

    match parse_target_profile_from_claim_policy(claim_policy) {
        Some(profile) if !profile.is_empty() => supervised_profiles.contains(&profile),
        _ => false,
    }
}

/// Return the per-task working directory path.
///
/// Convention: `~/.ep/worktrees/<task_id>/`
///
/// Uses `edgeplaned_core::paths::ep_home_dir()` so the base is the same as the rest
/// of edgeplaned's data (`~/.ep/`). The `worktrees/` subdirectory is created by the
/// caller (`std::fs::create_dir_all`) — this function only computes the path.
pub fn worktree_path_for_task(task_id: &str) -> PathBuf {
    edgeplaned_core::paths::ep_home_dir()
        .join("worktrees")
        .join(task_id)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── parse_target_profile_from_claim_policy ────────────────────────────

    #[test]
    fn parse_target_profile_present() {
        let policy = r#"{"target_profile":"research"}"#;
        assert_eq!(
            parse_target_profile_from_claim_policy(policy),
            Some("research".to_string())
        );
    }

    #[test]
    fn parse_target_profile_absent_returns_none() {
        let policy = r#"{"some_other_field":"foo"}"#;
        assert!(parse_target_profile_from_claim_policy(policy).is_none());
    }

    #[test]
    fn parse_target_profile_empty_object_returns_none() {
        assert!(parse_target_profile_from_claim_policy("{}").is_none());
    }

    #[test]
    fn parse_target_profile_invalid_json_returns_none() {
        assert!(parse_target_profile_from_claim_policy("not-json").is_none());
    }

    #[test]
    fn parse_target_profile_non_string_value_returns_none() {
        // target_profile is a number, not a string.
        let policy = r#"{"target_profile":42}"#;
        assert!(parse_target_profile_from_claim_policy(policy).is_none());
    }

    #[test]
    fn parse_target_profile_broadcast_policy_returns_none() {
        // broadcast is a plain string claim_policy, not JSON.
        // The controlplane stores "broadcast" directly (not as JSON).
        // Our parser correctly returns None for non-JSON strings.
        assert!(parse_target_profile_from_claim_policy("broadcast").is_none());
    }

    // ── should_claim ──────────────────────────────────────────────────────

    #[test]
    fn should_claim_matches_supervised_profile() {
        let task = json!({
            "id": "t-1",
            "claim_policy": r#"{"target_profile":"research"}"#,
        });
        let supervised: HashSet<String> = ["operator", "research", "work"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(should_claim(&task, &supervised));
    }

    #[test]
    fn should_claim_skips_unsupervised_profile() {
        let task = json!({
            "id": "t-2",
            "claim_policy": r#"{"target_profile":"unknown-profile"}"#,
        });
        let supervised: HashSet<String> = ["operator", "research"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!should_claim(&task, &supervised));
    }

    #[test]
    fn should_claim_skips_tasks_without_target_profile() {
        let task = json!({
            "id": "t-3",
            "claim_policy": "{}",
        });
        let supervised: HashSet<String> = ["operator"].iter().map(|s| s.to_string()).collect();
        assert!(!should_claim(&task, &supervised));
    }

    #[test]
    fn should_claim_skips_tasks_with_missing_claim_policy_field() {
        // No claim_policy field at all — defaults to "{}" internally.
        let task = json!({ "id": "t-4" });
        let supervised: HashSet<String> = ["operator"].iter().map(|s| s.to_string()).collect();
        assert!(!should_claim(&task, &supervised));
    }

    #[test]
    fn should_claim_skips_broadcast_tasks() {
        // "broadcast" is not valid JSON → parse_target_profile returns None.
        let task = json!({
            "id": "t-5",
            "claim_policy": "broadcast",
        });
        let supervised: HashSet<String> = ["operator"].iter().map(|s| s.to_string()).collect();
        assert!(!should_claim(&task, &supervised));
    }

    #[test]
    fn should_claim_empty_supervised_set_never_claims() {
        let task = json!({
            "id": "t-6",
            "claim_policy": r#"{"target_profile":"operator"}"#,
        });
        assert!(!should_claim(&task, &HashSet::new()));
    }

    // ── worktree_path_for_task ────────────────────────────────────────────

    #[test]
    fn worktree_path_ends_with_task_id() {
        let path = worktree_path_for_task("abc-123");
        assert!(
            path.to_string_lossy().ends_with("worktrees/abc-123"),
            "expected path ending in worktrees/abc-123, got {}",
            path.display()
        );
    }

    #[test]
    fn worktree_path_is_under_mc_home() {
        let path = worktree_path_for_task("task-x");
        let ep_home = edgeplaned_core::paths::ep_home_dir();
        assert!(
            path.starts_with(&ep_home),
            "worktree path {} should be under ep_home {}",
            path.display(),
            ep_home.display()
        );
    }

    // ── build_prompt ──────────────────────────────────────────────────────

    #[test]
    fn build_prompt_prefers_input_json_prompt() {
        let task = json!({
            "description": "Generic description",
            "input_json": r#"{"prompt":"Specific prompt from dispatcher"}"#,
            "title": "Task title",
        });
        assert_eq!(build_prompt(&task), "Specific prompt from dispatcher");
    }

    #[test]
    fn build_prompt_falls_back_to_description() {
        let task = json!({
            "description": "Do the thing.",
            "input_json": "{}",
            "title": "Title",
        });
        assert_eq!(build_prompt(&task), "Do the thing.");
    }

    #[test]
    fn build_prompt_falls_back_to_title() {
        let task = json!({
            "description": "",
            "title": "My task title",
        });
        assert_eq!(build_prompt(&task), "My task title");
    }

    #[test]
    fn build_prompt_last_resort_generic() {
        let task = json!({});
        assert_eq!(build_prompt(&task), "Complete the assigned task.");
    }

    #[test]
    fn build_prompt_handles_object_input_json() {
        // input_json may be deserialized as an object (not a string) in some
        // response shapes.
        let task = json!({
            "input_json": {"prompt": "Object-style prompt"},
            "description": "Fallback",
        });
        assert_eq!(build_prompt(&task), "Object-style prompt");
    }
}
