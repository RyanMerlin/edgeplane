# Architecture

## Core Runtime
- **Postgres (+ pgvector)**: authoritative coordination state (missions, klusters, tasks, approvals, profiles, ledger).
- **S3-compatible object storage**: active artifact/document bytes and workspace file persistence.
- **Git publication layer**: explicit memory-of-record projection for routable entities.
- **MCP/API control plane**: policy-gated tool execution, publish planning, and audited mutations.

## Persistence Model
- **Coordination truth stays in MissionControl (Postgres)**.
- **Git is a projection sink**, never the authority for mission ownership, approvals, or governance.
- **Mission-scoped routing** controls where publication events land:
  - `repo_connections`
  - `repo_bindings`
  - `mission_persistence_policies`
  - `mission_persistence_routes`
  - `publication_records`

## Publish Flow
1. Mutation enters ledger (`pending`) in Postgres.
2. Approval/policy checks run in MissionControl.
3. Route resolver picks binding/repo/branch/path from mission policy.
4. Provider adapter acquires server-side credential.
5. Publisher writes canonical file(s) to Git and records commit provenance.
6. Ledger/publication records are marked and queryable via API/MCP.

## Ephemeral Task Subagents (mcd task_worker)
- **`mcd::task_worker`** runs two long-running loops per node: a *claim* loop and a *triage* loop. See `docs/design/ephemeral-task-subagents.md` for the full identity model.
- **Bootstrap** (mcd startup): ensures a default `home` mission (overridable via `MC_HOME_MISSION_NAME`) and an `intake` kluster under it. Idempotent, soft-fail. The `home` mission is a regular mission — no special `kind` — that serves as the default container for operational scaffolding (the intake kluster, optionally agents' `home_mission_id`).
- **Claim loop**: polls for `MeshTask`s with `status='ready'` whose `claim_policy.target_profile` matches a profile supervised on this node. For each match: enrolls an ephemeral `MeshAgent` under the parent `Agent` identity, claims the task, opens an `AgentRun`, spawns `claude -p` in a per-task worktree (`~/.mc/worktrees/<task_id>/`) with `--allowed-tools` derived from `required_capabilities`, completes on subprocess exit, DELETEs the `MeshAgent` (FK `ON DELETE SET NULL` on `agentrun.mesh_agent_id` preserves audit).
- **Triage loop**: polls the intake kluster for unscoped tasks. Calls `aria goose` (local Qwen3.6-27B) for categorization; on confidence ≥ threshold, creates a child `MeshTask` with `parent_task_id` chain and `claim_policy.target_profile` set (which the claim loop then picks up). Below threshold → marks the intake task `blocked` (discoverable via `mc task ls --status blocked`); if `task_worker_surface_command` is configured, also invokes that command with `<task_id> <title> <reason>` so deployments can chain external alerts (vault note, Slack, etc.) without MC encoding any particular interface.
- **Capability enforcement**: a coarse vocabulary (`shell:read/write`, `fs:read/write`, `vault:read/write`, `mc:read/write`, `web:fetch`, `gh:read/write`) maps to `claude -p --allowed-tools` patterns. Dispatchers declare blast radius via `MeshTask.required_capabilities` (JSON array of capability names).
- **Audit invariant**: `Agent` identity is reused per parent profile (never created per task); `MeshAgent` rows are ephemeral; `AgentRun` rows are permanent and carry the durable trace (`total_cost_cents`, `idempotency_key`, `parent_run_id`, `runtime_session_id`). Multiple concurrent subagents share one `Agent` and have N concurrent `MeshAgent` projections — exactly the pattern `entities.md` line 96 anticipates.
