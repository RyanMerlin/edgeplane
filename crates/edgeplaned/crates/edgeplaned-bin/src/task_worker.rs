//! Ephemeral task subagent claimer loop (Phase 2) and triage loop (Phase 3).
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
//! # Phase 3 — triage loop
//!
//! A second long-running tokio task that examines unscoped tasks in the intake
//! mission (tasks with no `target_profile` in `claim_policy` and no prior triage
//! attempt). For each such task, it applies three-tier routing:
//!
//!   1. **Rule tier**: if the task already has a `target_profile` in
//!      `claim_policy`, it's scoped — skip (P2 handles it).
//!   2. **Goose categorization**: call `aria goose` with a structured prompt
//!      listing the task description and supervised profiles. Parse the returned
//!      `{"target_profile": "...", "confidence": 0.0–1.0, "reason": "..."}`.
//!   3. **Route or surface**:
//!      - Confidence ≥ threshold AND profile in supervised set →
//!        create child meshtask with `claim_policy = {"target_profile": "<name>"}`;
//!        claim + complete the intake task (status = `finished`).
//!      - Otherwise → block the intake task (status = `blocked`). If
//!        `task_worker_surface_command` is configured, additionally invoke
//!        it with `<task_id> <title> <reason>` appended; otherwise discovery
//!        is via `edgeplane task ls --status blocked` (MC-native).
//!
//! # Soft-fail philosophy
//!
//! Individual task failures log a warning and continue. Neither loop crashes the
//! daemon — controlplane unreachable is logged as a warning and retried on the
//! next poll interval. The daemon stays alive through any transient API error or
//! goose subprocess failure.
//!
//! # Concurrency
//!
//! P2: A `tokio::sync::Semaphore` caps active subagent processes at
//! `config.task_worker_max_concurrent`. Tasks beyond the cap stay `ready` in
//! the queue and are picked up when a slot frees.
//!
//! P3: Triage is sequential within a cycle (one goose call at a time) and capped
//! at `config.task_worker_max_triage_per_cycle`. A `tokio::sync::Mutex<bool>`
//! prevents overlapping triage cycles if a cycle exceeds the poll interval.
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
//! # HTTP endpoint surface used (P3 triage)
//!
//! - `GET /domains` → `GET /domains/{id}/k` → find intake mission by name.
//! - `GET /work/missions/{intake_mission_id}/tasks?status=ready` — list unscoped tasks.
//! - `POST /work/missions/{intake_mission_id}/tasks` — create child meshtask.
//! - `POST /work/tasks/{intake_task_id}/dispatched` — mark intake task finished (routed).
//!   Single admin-or-owner call; transitions `ready` → `finished` without a claim.
//!   Shipped in 0.15.10 specifically to replace the 4-call temp-agent dance.
//! - `POST /work/tasks/{intake_task_id}/block` — mark intake task blocked (low-confidence).
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
use crate::fleet_import::SOURCE_FLEET_IMPORT;
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

    let poll_interval =
        std::time::Duration::from_secs(config.task_worker_poll_interval_secs);
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

/// Read the local registry and return the set of profile names this edgeplaned node
/// supervises. These are the names imported from `fleet-profiles.toml` via
/// `fleet_import`. Falls back to an empty set on any registry error so the
/// daemon continues without crashing.
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

    match reg.list_by_source(SOURCE_FLEET_IMPORT) {
        Ok(records) => records.into_iter().map(|r| r.id).collect(),
        Err(e) => {
            tracing::warn!("task_worker: could not list fleet_import agents: {e:#}");
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
                tracing::debug!(
                    "task_worker: concurrency cap reached, deferring remaining tasks"
                );
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
            let id = task_c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
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

        let missions: Vec<Value> = match client
            .get(&format!("/domains/{domain_id}/m"))
            .await
        {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    "task_worker: GET /domains/{domain_id}/m failed: {e:#}, skipping"
                );
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
        task.get("claim_policy").and_then(|v| v.as_str()).unwrap_or("{}"),
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
            tracing::warn!(
                "task_worker: enroll_mesh_agent failed for task {task_id}: {e:#}"
            );
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

    tracing::debug!(
        "task_worker: enrolled ephemeral agent={agent_id} for task={task_id}"
    );

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

    let run_id = run_resp.get("id").and_then(|v| v.as_str()).map(String::from);

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
                tracing::info!(
                    "task_worker: subprocess exited successfully for task={task_id}"
                );
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
            tracing::warn!(
                "task_worker: complete AgentRun {rid} failed for task {task_id}: {e:#}"
            );
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
            tracing::warn!(
                "task_worker: complete MeshTask {task_id} failed: {e:#}"
            );
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
        tracing::warn!(
            "task_worker: fail MeshTask {task_id} (best-effort) failed: {e:#}"
        );
    }
}

/// Best-effort: delete the ephemeral MeshAgent. Logs on error but does not
/// crash. The controlplane's FK `ON DELETE SET NULL` on `agentrun.mesh_agent_id`
/// ensures AgentRun audit records survive deletion (Decision #1).
async fn delete_agent_soft(client: &BackendClient, agent_id: &str, task_id: &str) {
    if let Err(e) = client
        .delete(&format!("/work/agents/{agent_id}"))
        .await
    {
        tracing::warn!(
            "task_worker: DELETE /work/agents/{agent_id} failed for task {task_id}: {e:#}. \
             Agent row may linger — manual cleanup: \
             DELETE FROM meshagent WHERE id='{agent_id}'"
        );
    } else {
        tracing::debug!(
            "task_worker: deleted ephemeral agent={agent_id} for task={task_id}"
        );
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
/// `None` — P3 will handle unscoped tasks via triage.
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
    edgeplaned_core::paths::ep_home_dir().join("worktrees").join(task_id)
}

// ── Phase 3: Triage loop ──────────────────────────────────────────────────────

/// Parsed result from a goose categorization call.
#[derive(Debug, Clone, PartialEq)]
pub struct CategorizationResult {
    pub target_profile: String,
    /// 0.0–1.0 confidence that the routing is correct.
    pub confidence: f64,
    /// One-line reason from goose for the routing decision.
    pub reason: String,
}

/// Entry point for the P3 triage loop. Spawned alongside the P2 claimer loop
/// by `daemon.rs`. Loops forever at a slower cadence than P2 (default 60s vs
/// 30s), examining unscoped ready tasks in the intake mission.
///
/// Uses a `Mutex<bool>` to prevent overlapping triage cycles — if a cycle
/// takes longer than the poll interval, the next tick is skipped silently.
pub async fn run_triage_loop(client: Arc<BackendClient>, config: DaemonConfig) {
    if !config.task_worker_triage_enabled {
        tracing::info!("triage: disabled by config, not starting triage loop");
        return;
    }

    let poll_interval =
        std::time::Duration::from_secs(config.task_worker_triage_poll_interval_secs);
    let cycle_running = Arc::new(tokio::sync::Mutex::new(false));

    tracing::info!(
        "triage: starting loop (interval={}s, max_per_cycle={}, confidence_threshold={:.2})",
        config.task_worker_triage_poll_interval_secs,
        config.task_worker_max_triage_per_cycle,
        config.task_worker_triage_confidence_threshold,
    );

    loop {
        tokio::time::sleep(poll_interval).await;

        // Skip if a previous cycle is still running.
        let mut guard = cycle_running.lock().await;
        if *guard {
            tracing::debug!("triage: previous cycle still in flight, skipping tick");
            continue;
        }
        *guard = true;
        drop(guard); // release lock before async work

        let supervised = discover_supervised_profiles();
        if supervised.is_empty() {
            tracing::debug!(
                "triage: no supervised profiles in local registry, skipping triage cycle"
            );
        } else {
            triage_cycle(&client, &config, &supervised).await;
        }

        *cycle_running.lock().await = false;
    }
}

/// One triage cycle: find unscoped ready tasks in the intake mission and
/// attempt to route or surface each one.
async fn triage_cycle(
    client: &Arc<BackendClient>,
    config: &DaemonConfig,
    supervised: &HashSet<String>,
) {
    // Resolve intake mission.
    let (intake_mission_id, intake_domain_id) =
        match resolve_intake_mission(client).await {
            Some(pair) => pair,
            None => {
                tracing::debug!(
                    "triage: intake mission not found, skipping cycle \
                     (bootstrap may not have run yet)"
                );
                return;
            }
        };

    // Fetch ready tasks from the intake mission.
    let tasks: Vec<Value> = match client
        .get(&format!(
            "/work/missions/{intake_mission_id}/tasks?status=ready"
        ))
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(
                "triage: GET /work/missions/{intake_mission_id}/tasks failed: {e:#}"
            );
            return;
        }
    };

    // Filter to unscoped tasks only (no target_profile, not already triaged).
    let unscoped: Vec<Value> = tasks
        .into_iter()
        .filter(|t| should_triage(t, supervised))
        .take(config.task_worker_max_triage_per_cycle)
        .collect();

    if unscoped.is_empty() {
        tracing::debug!("triage: no unscoped tasks in intake mission");
        return;
    }

    tracing::info!(
        "triage: {} unscoped task(s) to triage in intake mission {intake_mission_id}",
        unscoped.len()
    );

    for task in &unscoped {
        triage_one(
            client,
            config,
            task,
            &intake_mission_id,
            &intake_domain_id,
            supervised,
        )
        .await;
    }
}

/// Triage one unscoped task. Calls goose to categorize, then either:
/// - Routes via child meshtask + marks intake task finished (high confidence).
/// - Blocks the intake task + optionally invokes the surface command (low confidence).
async fn triage_one(
    client: &Arc<BackendClient>,
    config: &DaemonConfig,
    task: &Value,
    intake_mission_id: &str,
    intake_domain_id: &str,
    supervised: &HashSet<String>,
) {
    let task_id = match task.get("id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            tracing::warn!("triage: task missing id, skipping");
            return;
        }
    };
    let title = task
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(no title)");

    tracing::info!("triage: categorizing task={task_id} title='{title}'");

    // Step 1: Build prompt and call goose.
    let prompt = build_categorization_prompt(task, supervised);
    let categorization = call_goose_categorize(&prompt, config).await;

    // Step 2: Decide routing.
    let should_route = match &categorization {
        Some(r) => should_route_to_profile(r, config.task_worker_triage_confidence_threshold, supervised),
        None => false,
    };

    if should_route {
        let result = categorization.as_ref().unwrap();
        tracing::info!(
            "triage: routing task={task_id} to profile='{}' (confidence={:.2}, reason='{}')",
            result.target_profile,
            result.confidence,
            result.reason,
        );
        route_task(client, config, task, intake_mission_id, intake_domain_id, &result.target_profile).await;
    } else {
        let reason_summary = categorization
            .as_ref()
            .map(|r| format!("confidence={:.2} reason='{}'", r.confidence, r.reason))
            .unwrap_or_else(|| "goose categorization failed".to_string());

        tracing::info!(
            "triage: low-confidence for task={task_id} — blocking + surfacing ({reason_summary})"
        );
        surface_to_inbox(client, task, &categorization, intake_mission_id, config.task_worker_surface_command.as_ref()).await;
    }
}

/// Route a task: create child meshtask, claim intake task, complete intake task.
///
/// State machine note: `complete_task` requires status = `claimed`/`running`.
/// Since the intake task starts as `ready`, we must claim it (with a temporary
/// triage agent) before completing. The temporary agent is deleted after.
async fn route_task(
    client: &Arc<BackendClient>,
    config: &DaemonConfig,
    intake_task: &Value,
    intake_mission_id: &str,
    intake_domain_id: &str,
    target_profile: &str,
) {
    let intake_task_id = match intake_task.get("id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            tracing::warn!("triage: route_task called with task missing id");
            return;
        }
    };

    // ── Step 1: Create child meshtask in the intake mission ────────────────
    //
    // Child task carries the work; intake task becomes the routing record.
    // claim_policy routes to the target profile. parent_task_id links back.

    let child_claim_policy = serde_json::json!({ "target_profile": target_profile }).to_string();
    let child_body = serde_json::json!({
        "title": intake_task.get("title").and_then(|v| v.as_str()).unwrap_or("(untitled)"),
        "description": intake_task.get("description").and_then(|v| v.as_str()).unwrap_or(""),
        "input_json": intake_task.get("input_json").and_then(|v| v.as_str()).unwrap_or("{}"),
        "claim_policy": child_claim_policy,
        "parent_task_id": intake_task_id,
        "priority": intake_task.get("priority").and_then(|v| v.as_i64()).unwrap_or(0),
    });

    let child_resp: Value = match client
        .post(
            &format!("/work/missions/{intake_mission_id}/tasks"),
            &child_body,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "triage: create child task failed for intake={intake_task_id}: {e:#}. \
                 Falling back to vault surface."
            );
            // Fall back to blocking + surfacing rather than leaving task unhandled.
            surface_to_inbox_low_confidence(client, intake_task, "child task creation failed", intake_mission_id, config.task_worker_surface_command.as_ref()).await;
            return;
        }
    };

    let child_id = child_resp.get("id").and_then(|v| v.as_str()).unwrap_or("(unknown)");
    tracing::debug!(
        "triage: created child task={child_id} for intake={intake_task_id} → profile={target_profile}"
    );

    // ── Step 2: Mark intake task as dispatched (single admin call) ─────────
    //
    // As of 0.15.10, `POST /work/tasks/{id}/dispatched` transitions a `ready`
    // task to `finished` for admin-or-owner — no claim needed. Replaces the
    // 4-call dance (enroll temp agent + claim + complete + delete agent) that
    // we used in 0.15.6–0.15.9 because `complete_task` requires a claimed
    // status. Single call now.
    if let Err(e) = client
        .raw_post(&format!("/work/tasks/{intake_task_id}/dispatched"), &serde_json::json!({}))
        .await
    {
        tracing::warn!(
            "triage: dispatch intake task={intake_task_id} failed: {e:#}. \
             Child task created (id={child_id}) but intake task left in ready state — will re-triage next cycle."
        );
        return;
    }

    tracing::info!(
        "triage: intake task={intake_task_id} marked finished (routed to profile={target_profile}, \
         child={child_id}, domain={intake_domain_id})"
    );

    // ── Step 3: Ignore goose_timeout_secs here — used by call_goose_categorize ─
    let _ = config.task_worker_goose_timeout_secs; // suppress unused warning
}

/// Block the intake task and (optionally) invoke the surface command.
async fn surface_to_inbox(
    client: &Arc<BackendClient>,
    intake_task: &Value,
    categorization: &Option<CategorizationResult>,
    intake_mission_id: &str,
    surface_command: Option<&Vec<String>>,
) {
    let reason = categorization
        .as_ref()
        .map(|r| format!(
            "goose confidence={:.2}, target='{}', reason='{}'",
            r.confidence, r.target_profile, r.reason
        ))
        .unwrap_or_else(|| "goose categorization returned no result".to_string());
    surface_to_inbox_low_confidence(client, intake_task, &reason, intake_mission_id, surface_command).await;
}

/// Internal helper: block the intake task and (optionally) invoke a
/// deployment-configured surface command.
///
/// MC itself is decoupled from any particular human-interface convention.
/// The default behavior is just `status=blocked` — operators discover via
/// `edgeplane task ls --status blocked`. Deployments that want to chain an external
/// alert (vault note, Slack, GitHub Issue, email, etc.) set
/// `config.task_worker_surface_command`; edgeplaned shells out with
/// `<task_id> <title> <reason>` appended after the configured args.
async fn surface_to_inbox_low_confidence(
    client: &Arc<BackendClient>,
    intake_task: &Value,
    reason: &str,
    _intake_mission_id: &str,
    surface_command: Option<&Vec<String>>,
) {
    let task_id = intake_task
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)");
    let title = intake_task
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(no title)");

    // Block the intake task (transitions from any status to 'blocked').
    // block_task has no request body.
    if let Err(e) = client
        .raw_post(&format!("/work/tasks/{task_id}/block"), &serde_json::json!({}))
        .await
    {
        tracing::warn!(
            "triage: block intake task={task_id} failed: {e:#}. \
             Task left in ready state — will be re-triaged next cycle."
        );
        return;
    }

    tracing::info!(
        "triage: intake task={task_id} blocked (needs human triage). reason={reason}"
    );

    // Optional surface hook — deployment-configured external alert.
    let Some(cmd) = surface_command else {
        return;
    };
    if cmd.is_empty() {
        return;
    }

    let program = &cmd[0];
    let base_args: Vec<&str> = cmd.iter().skip(1).map(String::as_str).collect();
    let extra_args = [task_id, title, reason];

    let surface_result = tokio::process::Command::new(program)
        .args(&base_args)
        .args(extra_args)
        .output()
        .await;

    match surface_result {
        Ok(out) if out.status.success() => {
            tracing::debug!("triage: surface command ok for task={task_id}");
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::warn!(
                "triage: surface command for task={task_id} failed (exit={}): {}",
                out.status,
                stderr.chars().take(200).collect::<String>()
            );
        }
        Err(e) => {
            tracing::warn!(
                "triage: could not spawn surface command '{program}' for task={task_id}: {e:#}"
            );
        }
    }
}

/// Resolve the intake mission id and its parent domain id.
///
/// Walks domains → missions, looking for a mission named `INTAKE_MISSION_NAME`.
/// Returns `None` if the intake mission doesn't exist yet (bootstrap hasn't run)
/// or if the controlplane is unreachable.
async fn resolve_intake_mission(client: &Arc<BackendClient>) -> Option<(String, String)> {
    use crate::bootstrap::INTAKE_MISSION_NAME;

    let domains: Vec<Value> = match client.get("/domains").await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("triage: GET /domains failed: {e:#}");
            return None;
        }
    };

    for domain in &domains {
        let domain_id = domain.get("id").and_then(|v| v.as_str())?;

        let missions: Vec<Value> = match client
            .get(&format!("/domains/{domain_id}/m"))
            .await
        {
            Ok(k) => k,
            Err(_) => continue,
        };

        for k in &missions {
            if k.get("name").and_then(|v| v.as_str()) == Some(INTAKE_MISSION_NAME)
                && let Some(kid) = k.get("id").and_then(|v| v.as_str())
            {
                return Some((kid.to_string(), domain_id.to_string()));
            }
        }
    }

    None
}

/// Call `aria goose` with the categorization prompt and parse the response.
///
/// The `aria goose` response shape is:
///   `{"ok": bool, "data": <string | object>, ...}`
///
/// If `ok=false`, returns `None` (low-confidence fallback).
/// If `data` is a JSON string, attempts to parse it as `CategorizationResult`.
/// If `data` is already an object, uses it directly.
pub async fn call_goose_categorize(
    prompt: &str,
    config: &DaemonConfig,
) -> Option<CategorizationResult> {
    let timeout_secs = config.task_worker_goose_timeout_secs;

    let output = tokio::process::Command::new("aria")
        .args(["goose", prompt, "--timeout", &timeout_secs.to_string()])
        .output()
        .await
        .map_err(|e| {
            tracing::warn!("triage: could not spawn 'aria goose': {e:#}");
            e
        })
        .ok()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            "triage: aria goose exited with status={} stderr={}",
            output.status,
            stderr.chars().take(200).collect::<String>()
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_goose_response(&stdout)
}

/// Parse the JSON output of `aria goose` into a `CategorizationResult`.
///
/// Exposed as `pub` for unit testing without spawning subprocesses.
pub fn parse_goose_response(raw: &str) -> Option<CategorizationResult> {
    let outer: Value = serde_json::from_str(raw.trim())
        .map_err(|e| {
            tracing::warn!("triage: could not parse goose response as JSON: {e:#} (raw={raw})");
            e
        })
        .ok()?;

    // Check `ok` field.
    if outer.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        tracing::warn!(
            "triage: goose reported ok=false: {}",
            outer.get("error").and_then(|v| v.as_str()).unwrap_or("(no error field)")
        );
        return None;
    }

    // `data` may be a JSON string (goose wraps the reply) or an object.
    let data = outer.get("data")?;
    let inner: Value = if let Some(s) = data.as_str() {
        // Goose returned data as a JSON-encoded string — parse it.
        serde_json::from_str(s)
            .map_err(|e| {
                tracing::warn!(
                    "triage: data field is a string but not valid JSON: {e:#} (data={s})"
                );
                e
            })
            .ok()?
    } else {
        // data is already an object.
        data.clone()
    };

    let target_profile = inner
        .get("target_profile")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)?;

    let confidence = inner
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let reason = inner
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("(no reason)")
        .to_string();

    Some(CategorizationResult {
        target_profile,
        confidence,
        reason,
    })
}

/// Build the categorization prompt for goose.
///
/// Instructs goose to return a JSON object with `target_profile`, `confidence`,
/// and `reason`. Includes the task's title and description and the list of
/// supervised profiles so goose can make an informed routing decision.
pub fn build_categorization_prompt(task: &Value, supervised: &HashSet<String>) -> String {
    let title = task
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(no title)");
    let description = task
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let desc_excerpt: String = description.chars().take(500).collect();

    // Sort profiles for deterministic prompt output (aids testing).
    let mut profiles: Vec<&str> = supervised.iter().map(String::as_str).collect();
    profiles.sort_unstable();
    let profiles_list = profiles.join(", ");

    format!(
        "You are a task router for an AI agent fleet. Categorize this task and return ONLY a \
        JSON object (no markdown, no explanation) with fields: \
        \"target_profile\" (string), \"confidence\" (float 0.0-1.0), \"reason\" (one-line string).\n\n\
        Available profiles: {profiles_list}\n\n\
        Task title: {title}\n\
        Task description: {desc_excerpt}\n\n\
        Which profile should handle this task? If you are not confident (< 0.85), \
        set a low confidence score. Only output the JSON object."
    )
}

/// Decide if a task needs triage.
///
/// A task should be triaged if:
/// - It has no `target_profile` in `claim_policy` (unscoped).
/// - Its `claim_policy` does not identify it as already-scoped for a supervised profile.
/// - The task does not have `status='blocked'` (blocked = already surfaced, awaiting human).
///
/// Note: the query already filters by `status=ready`, so `status` checking is
/// belt-and-suspenders (the response should only include ready tasks, but we
/// guard here for future use).
pub fn should_triage(task: &Value, supervised: &HashSet<String>) -> bool {
    // Skip if already scoped (P2 handles it).
    let claim_policy = task
        .get("claim_policy")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    if parse_target_profile_from_claim_policy(claim_policy).is_some() {
        return false;
    }

    // Skip if already blocked (prior low-confidence triage — awaiting human).
    let status = task
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("ready");
    if status == "blocked" {
        return false;
    }

    // Skip if supervised set is empty (no profiles to route to).
    if supervised.is_empty() {
        return false;
    }

    true
}

/// Decide whether a categorization result warrants auto-routing.
///
/// Returns `true` iff:
/// - `confidence >= threshold`
/// - `target_profile` is in `supervised_profiles` (non-empty after trim).
pub fn should_route_to_profile(
    result: &CategorizationResult,
    threshold: f64,
    supervised: &HashSet<String>,
) -> bool {
    if result.confidence < threshold {
        return false;
    }
    if result.target_profile.trim().is_empty() {
        return false;
    }
    supervised.contains(&result.target_profile)
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
        let supervised: HashSet<String> =
            ["operator", "research", "work"].iter().map(|s| s.to_string()).collect();
        assert!(should_claim(&task, &supervised));
    }

    #[test]
    fn should_claim_skips_unsupervised_profile() {
        let task = json!({
            "id": "t-2",
            "claim_policy": r#"{"target_profile":"unknown-profile"}"#,
        });
        let supervised: HashSet<String> =
            ["operator", "research"].iter().map(|s| s.to_string()).collect();
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

    // ── P3 triage: parse_goose_response ──────────────────────────────────

    #[test]
    fn parse_goose_response_valid_string_data() {
        // Happy path: aria goose returns data as a JSON-encoded string.
        let raw = r#"{"ok":true,"data":"{\"target_profile\":\"research\",\"confidence\":0.9,\"reason\":\"Looks like research work\"}"}"#;
        let result = parse_goose_response(raw).unwrap();
        assert_eq!(result.target_profile, "research");
        assert!((result.confidence - 0.9).abs() < 1e-6);
        assert_eq!(result.reason, "Looks like research work");
    }

    #[test]
    fn parse_goose_response_data_already_object() {
        // Some goose versions return data as a JSON object, not a string.
        let raw = r#"{"ok":true,"data":{"target_profile":"work","confidence":0.95,"reason":"Work task"}}"#;
        let result = parse_goose_response(raw).unwrap();
        assert_eq!(result.target_profile, "work");
        assert!((result.confidence - 0.95).abs() < 1e-6);
        assert_eq!(result.reason, "Work task");
    }

    #[test]
    fn parse_goose_response_ok_false_returns_none() {
        let raw = r#"{"ok":false,"error":"LiteLLM 429 rate limit"}"#;
        assert!(parse_goose_response(raw).is_none());
    }

    #[test]
    fn parse_goose_response_invalid_outer_json_returns_none() {
        assert!(parse_goose_response("this is not json").is_none());
    }

    #[test]
    fn parse_goose_response_invalid_inner_json_returns_none() {
        // data is a string but not valid JSON.
        let raw = r#"{"ok":true,"data":"not-valid-json-either"}"#;
        assert!(parse_goose_response(raw).is_none());
    }

    #[test]
    fn parse_goose_response_missing_target_profile_returns_none() {
        let raw = r#"{"ok":true,"data":{"confidence":0.9,"reason":"something"}}"#;
        assert!(parse_goose_response(raw).is_none());
    }

    #[test]
    fn parse_goose_response_empty_target_profile_returns_none() {
        let raw = r#"{"ok":true,"data":{"target_profile":"","confidence":0.9,"reason":"x"}}"#;
        assert!(parse_goose_response(raw).is_none());
    }

    // ── P3 triage: should_route_to_profile ───────────────────────────────

    #[test]
    fn should_route_above_threshold_with_supervised_profile() {
        let result = CategorizationResult {
            target_profile: "research".to_string(),
            confidence: 0.9,
            reason: "Clearly research".to_string(),
        };
        let supervised: HashSet<String> =
            ["operator", "research", "work"].iter().map(|s| s.to_string()).collect();
        assert!(should_route_to_profile(&result, 0.85, &supervised));
    }

    #[test]
    fn should_route_below_threshold_returns_false() {
        let result = CategorizationResult {
            target_profile: "research".to_string(),
            confidence: 0.5,
            reason: "Uncertain".to_string(),
        };
        let supervised: HashSet<String> =
            ["research"].iter().map(|s| s.to_string()).collect();
        assert!(!should_route_to_profile(&result, 0.85, &supervised));
    }

    #[test]
    fn should_route_above_threshold_unsupervised_profile_returns_false() {
        let result = CategorizationResult {
            target_profile: "some-other-profile".to_string(),
            confidence: 0.99,
            reason: "Very confident but unknown profile".to_string(),
        };
        let supervised: HashSet<String> =
            ["operator", "research"].iter().map(|s| s.to_string()).collect();
        assert!(!should_route_to_profile(&result, 0.85, &supervised));
    }

    #[test]
    fn should_route_exact_threshold_boundary() {
        let result = CategorizationResult {
            target_profile: "operator".to_string(),
            confidence: 0.85,
            reason: "At threshold".to_string(),
        };
        let supervised: HashSet<String> =
            ["operator"].iter().map(|s| s.to_string()).collect();
        // Exactly at threshold: >= 0.85 should route.
        assert!(should_route_to_profile(&result, 0.85, &supervised));
    }

    #[test]
    fn should_route_just_below_threshold_boundary() {
        let result = CategorizationResult {
            target_profile: "operator".to_string(),
            confidence: 0.849,
            reason: "Just below threshold".to_string(),
        };
        let supervised: HashSet<String> =
            ["operator"].iter().map(|s| s.to_string()).collect();
        assert!(!should_route_to_profile(&result, 0.85, &supervised));
    }

    #[test]
    fn should_route_empty_supervised_set_returns_false() {
        let result = CategorizationResult {
            target_profile: "research".to_string(),
            confidence: 0.99,
            reason: "Very confident".to_string(),
        };
        assert!(!should_route_to_profile(&result, 0.85, &HashSet::new()));
    }

    // ── P3 triage: should_triage ──────────────────────────────────────────

    #[test]
    fn task_already_triaged_scoped_skip() {
        // A task with target_profile already set is scoped — P2's job.
        let task = json!({
            "id": "t-triaged",
            "status": "ready",
            "claim_policy": r#"{"target_profile":"research"}"#,
        });
        let supervised: HashSet<String> = ["research"].iter().map(|s| s.to_string()).collect();
        assert!(!should_triage(&task, &supervised));
    }

    #[test]
    fn task_blocked_is_skipped() {
        // Blocked = already surfaced for human triage.
        let task = json!({
            "id": "t-blocked",
            "status": "blocked",
            "claim_policy": "{}",
        });
        let supervised: HashSet<String> = ["operator"].iter().map(|s| s.to_string()).collect();
        assert!(!should_triage(&task, &supervised));
    }

    #[test]
    fn task_unscoped_ready_needs_triage() {
        let task = json!({
            "id": "t-unscoped",
            "status": "ready",
            "claim_policy": "{}",
        });
        let supervised: HashSet<String> = ["operator"].iter().map(|s| s.to_string()).collect();
        assert!(should_triage(&task, &supervised));
    }

    #[test]
    fn task_missing_claim_policy_needs_triage() {
        // No claim_policy field at all — treat as unscoped.
        let task = json!({ "id": "t-nopolicy", "status": "ready" });
        let supervised: HashSet<String> = ["operator"].iter().map(|s| s.to_string()).collect();
        assert!(should_triage(&task, &supervised));
    }

    #[test]
    fn should_triage_empty_supervised_set_returns_false() {
        let task = json!({ "id": "t-x", "status": "ready", "claim_policy": "{}" });
        assert!(!should_triage(&task, &HashSet::new()));
    }

    // ── P3 triage: build_categorization_prompt ────────────────────────────

    #[test]
    fn build_categorization_prompt_includes_task_title_and_description() {
        let task = json!({
            "id": "t-1",
            "title": "Analyze Q2 data",
            "description": "Run analysis on the Q2 dataset and produce a report.",
        });
        let supervised: HashSet<String> = ["research"].iter().map(|s| s.to_string()).collect();
        let prompt = build_categorization_prompt(&task, &supervised);
        assert!(prompt.contains("Analyze Q2 data"), "prompt should include task title");
        assert!(
            prompt.contains("Run analysis on the Q2 dataset"),
            "prompt should include task description"
        );
    }

    #[test]
    fn build_categorization_prompt_lists_supervised_profiles() {
        let task = json!({ "id": "t-2", "title": "Some task", "description": "" });
        let supervised: HashSet<String> =
            ["operator", "research", "work"].iter().map(|s| s.to_string()).collect();
        let prompt = build_categorization_prompt(&task, &supervised);
        assert!(prompt.contains("operator"), "prompt should list 'operator' profile");
        assert!(prompt.contains("research"), "prompt should list 'research' profile");
        assert!(prompt.contains("work"), "prompt should list 'work' profile");
    }

    #[test]
    fn build_categorization_prompt_requests_json_output() {
        let task = json!({ "id": "t-3", "title": "Task", "description": "" });
        let supervised: HashSet<String> = ["operator"].iter().map(|s| s.to_string()).collect();
        let prompt = build_categorization_prompt(&task, &supervised);
        assert!(
            prompt.contains("target_profile"),
            "prompt should mention 'target_profile' field"
        );
        assert!(
            prompt.contains("confidence"),
            "prompt should mention 'confidence' field"
        );
        assert!(
            prompt.contains("JSON"),
            "prompt should ask for JSON output"
        );
    }
}
