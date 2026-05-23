---
title: Ephemeral Task Agents
description: How edgeplaned spawns, manages, and audits short-lived agent subprocesses for distributed mesh task execution.
---

The **task worker** (`edgeplaned::task_worker`) enables distributed, parallel AI execution within Edgeplane's mesh. When a dispatcher submits a `MeshTask`, `edgeplaned` spawns an ephemeral agent subprocess to claim and execute it — without touching any persistent agent session or interrupting operator context.

## The Problem This Solves

Without ephemeral agents, when something actionable surfaces (a scheduled check, an automated analysis, an inbound request), dispatchers have two bad options:

1. **Signal-inject** into a live profile session — the signal lands as a user message in whatever conversation is active, interrupting focused work and polluting context.
2. **Write a note somewhere** and hope the operator notices — loses urgency, breaks the loop, no feedback.

The task worker provides a third path: submit a `MeshTask` to the mesh, let an ephemeral subagent claim it, run it to completion in an isolated context, and report back. The persistent session is never touched. Edgeplane retains full visibility and a durable audit trail.

## Identity Model

The ephemeral subagent is **not a new `Agent`**. It is an `AgentRun` of an existing `Agent` (the parent profile), surfaced into the mesh as a transient `MeshAgent`.

| Entity | Lifetime | Notes |
|--------|----------|-------|
| `Agent` | Permanent | Parent profile identity. **Reused, never created per task.** |
| `MeshTask` | Task lifecycle | Created by dispatcher with `required_capabilities` and target `mission_id` |
| `MeshAgent` | Task lifecycle | New per subagent. FK back to parent `Agent`. **Deleted on completion.** |
| `ExecutionSession` | Task lifecycle | Compute lease for the subagent process |
| `AISession` | Task lifecycle | Fresh AI session per task — ephemerality is the point. No resume by default. |
| `AgentRun` | **Permanent** | Joins agent + task + session. **This is the audit record. It persists after the task completes.** |

`AgentRun.mesh_agent_id` FK is `ON DELETE SET NULL` — the audit trail survives `MeshAgent` deletion.

One `Agent` identity can run N concurrent `MeshAgent` projections: one persistent operator session plus M ephemeral task subagents executing in parallel.

## Task Worker Components

`edgeplaned::task_worker` runs two loops per node:

### Claim Loop

Polls for `MeshTask`s with `status='ready'` whose `claim_policy.target_profile` matches a profile supervised on this node.

For each match:
1. Enrolls an ephemeral `MeshAgent` under the parent `Agent` identity (`labels: {"role": "task-subagent", "ephemeral": true}`)
2. Claims the task (lease-based)
3. Opens an `AgentRun` record
4. Allocates a per-task git worktree at `~/.ep/worktrees/<task_id>/` to prevent concurrent git collisions
5. Spawns the agent subprocess with `--allowed-tools` derived from `required_capabilities`
6. On exit: marks task complete, deletes the `MeshAgent` row, closes the `AgentRun`

### Triage Loop

Polls the `intake` mission for unscoped, unrouted tasks.

For each task:
- **Rule-based routing** (target profile explicitly set) → claim loop picks it up
- **Categorizer routing** (confidence ≥ threshold) → creates a child `MeshTask` in the appropriate mission with `parent_task_id` chain; claim loop picks it up
- **Low confidence** → marks the task `blocked`; optionally invokes `task_worker_surface_command` with `<task_id> <title> <reason>` so deployments can chain external notifications without MC encoding a specific interface

## Bootstrap

On startup, `edgeplaned` ensures a default `home` domain (name overridable via `EP_HOME_DOMAIN_NAME`) and an `intake` mission exist under it. This is idempotent and soft-fails silently if the control plane is unavailable.

The `home` domain is a regular domain — no special type — that provides a default container for operational scaffolding (intake mission, agent `home_domain_id` anchors).

## Capability Enforcement

Dispatchers declare blast radius via `MeshTask.required_capabilities` (JSON array). The task worker translates these to agent launch flags at spawn time.

Coarse capability vocabulary:

| Capability | What it grants |
|------------|---------------|
| `shell:read` | Read-only shell commands |
| `shell:write` | Shell commands with write effects |
| `fs:read` | Filesystem reads |
| `fs:write` | Filesystem writes |
| `vault:read` | Knowledge store reads |
| `vault:write` | Knowledge store writes |
| `edgeplane:read` | Edgeplane read operations |
| `edgeplane:write` | Edgeplane write operations |
| `web:fetch` | HTTP fetch |
| `gh:read` | GitHub reads |
| `gh:write` | GitHub writes |

If a task requires capabilities the parent agent doesn't have, the claim loop skips it.

## Visibility

Despite agent ephemerality, Edgeplane retains full visibility:

- **`edgeplane agent list`** — shows parent identity plus active subagent projections
- **`edgeplane task list --status running`** — shows work in progress with `claimed_by_agent_id`
- **`get_entity_history`** on parent `Agent` — joins through `AgentRun` to show every subagent execution, including ephemerals long after their `MeshAgent` is gone
- **Cost rollup** — `agentrun.total_cost_cents` queryable per parent agent, per mission, per domain

The ephemeral nature is in the runtime projection, not the audit trail.

## Concurrency Handling

| Hazard | Mitigation |
|--------|-----------|
| Concurrent git operations | Per-task worktrees at `~/.ep/worktrees/<task_id>/` — each subagent has its own checkout |
| API rate limits | `max_concurrent_subagents` config (default: 3); excess tasks queue in the mesh |
| Append-only writes | Safe under POSIX for ≤PIPE_BUF; vault writes are atomic at the git layer |

## Submitting a Task to the Mesh

```bash
# Via MCP tool (from within an agent session)
submit_mesh_task(
  mission_id = "...",
  prompt = "Run the full integration test suite and report results",
  required_capabilities = ["shell:read", "edgeplane:read"],
  claim_policy = {"target_profile": "my-profile"}
)

# Via CLI (coming in a future release)
edgeplane daemon task submit --mission-id <id> --prompt "..." --capabilities shell:read,edgeplane:read
```

## See Also

- [Concepts: Entity Reference](/edgeplane/concepts/entity-reference/) — canonical definitions for `MeshTask`, `MeshAgent`, `AgentRun`
- [Reference: edgeplaned Daemon](/edgeplane/reference/edgeplaned-daemon/) — daemon configuration and socket interface
- [Architecture: System Overview](/edgeplane/architecture/overview/) — where `edgeplaned` fits in the overall component map
