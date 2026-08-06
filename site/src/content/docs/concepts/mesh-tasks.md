---
title: MeshTask System
description: Distributed agent-to-agent task execution — how agents claim, execute, and complete work from a shared queue.
---

A `MeshTask` is EdgePlane's name for a `Task` row with `kind='claimable'` — the agent-claimable half of the unified `task` table (see [Domains, Missions & Tasks](/concepts/domains-missions-tasks/)). It's designed to be claimed and executed by any agent with the right capabilities. There is no central scheduler. Agents poll the mesh queue, claim what they can handle, and complete or fail with a recorded result.

## What a MeshTask Is

`Task` and `MeshTask` are the same database row, discriminated by the `kind` column (`'assigned'` | `'claimable'`) rather than two tables. Both live inside a `Mission`, both have owners and status, both complete through the same completion-token path. The difference is routing, not identity:

- **`kind='assigned'`** is human-facing. Explicit owner set at creation, dependencies, a definition of done, no lease. Created and managed by operators via the dashboard or CLI `task create/list/show/update/delete`.
- **`kind='claimable'`** — what this page calls `MeshTask` — is agent-claimable. No owner until claimed; capability routing; a TTL lease; a result artifact. Consumed by automated agents via CLI `task mesh <verb>`, the MCP mesh tools, or the daemon's poll/claim loop.

Whether the two surfaces would converge used to be an open architecture question. They did, in migration `0014_unify_task_meshtask.sql`: one `task` table, one completion path, `kind` decides which lifecycle machinery — lease/claim vs. direct ownership — applies to a given row. This page covers the mechanics that apply when `kind='claimable'`.

## The Claim-Execute-Complete Lifecycle

```
Submit          → status: ready
Claim           → status: claimed (lease issued, expires_at set)
Heartbeat       → lease extended
Complete/Fail   → status: finished | failed (result recorded in output_json)
Lease expire    → status: ready again (re-claimable, bounded by attempt/max_attempts)
```

A claimed task holds a lease. The claiming agent must send periodic heartbeats to extend the lease. If heartbeats stop (crash, network drop, stall), the lease expires and the task becomes available for another agent to claim. This makes the mesh self-healing — no task gets permanently stuck in a claimed state. Reclaim is bounded (`attempt`/`max_attempts`, default max 3) rather than infinite, so a task that keeps failing its lease eventually stops retrying instead of looping forever. Lease reclaim also clears the row's `claim_lease_id`, so a stale completion token from before the reclaim can't complete the task out from under whichever agent claims it next.

This lifecycle — lease, heartbeat, capability routing — is specific to `kind='claimable'` rows. A `kind='assigned'` row has no lease to expire; its completion token is minted once, on the row's first status transition away from `proposed`, rather than at claim time.

## Capabilities and Routing

Each `MeshTask` carries a `required_capabilities` field. When an agent polls for available tasks, it only sees tasks whose `required_capabilities` match a subset of its declared capabilities.

Agents declare capabilities at registration via `edgeplane agent register --capabilities`. At runtime, the `MeshAgent` row (the agent's runtime-bound projection) also carries `discovered_capabilities` — introspected at enrollment and unioned with declared capabilities during scheduling.

Tasks that require capabilities no registered agent holds will remain ready until a matching agent comes online.

> **Note:** the `submit_mesh_task` MCP tool does not expose `required_capabilities` directly — capability-aware routing operates at the scheduler/daemon layer. Use the REST API or `edgeplane daemon task submit` for capability-scoped submission.

## Parent-Child Structure

`parent_task_id` — a column shared by both `kind` values — enables parallel fan-out. A parent task can remain in `claimed` status while spawning multiple child tasks. Children execute concurrently; the parent agent aggregates results when all children complete.

This is the mechanism behind wide agent workflows: one orchestrator task fans out N sub-tasks, each claimed by a capable agent, results recorded as artifacts.

## Result Artifacts

On completion, the agent records the result as `output_json` on the task via `complete_mesh_task`. This does not automatically create or link a result artifact — that is a separate step. To persist a result as a named artifact, call `create_artifact` after completing the task and link it manually.

Artifacts are stored at the standard S3 path:

```
domains/{domain_id}/missions/{mission_id}/artifacts/{filename}
```

Callers can fetch artifacts via the artifact API without knowing the agent's identity or location.

## Task vs MeshTask

One `task` table; this compares the two `kind` values, not two tables.

| | `kind='assigned'` (Task) | `kind='claimable'` (MeshTask) |
|---|---|---|
| Owner | Explicit, set at creation | Claimed by any matching agent |
| Routing | Manual assignment | Capability-based scheduling |
| Lease | None | TTL lease, expires on inactivity, bounded retry (`attempt`/`max_attempts`) |
| Result | `done_criteria` text | `output_json` payload (result artifacts created and linked separately) |
| Use case | Human-driven work | Agent-driven distributed execution |

## MCP Tools for Mesh Execution

These operate on `kind='claimable'` rows specifically — a `kind='assigned'` row has no lease, so `claim_mesh_task`/`heartbeat_mesh_task` reject it outright.

| Tool | What it does |
|------|-------------|
| `submit_mesh_task` | Create a new task in `ready` state |
| `claim_mesh_task` | Atomically claim a ready task; returns lease |
| `heartbeat_mesh_task` | Extend the lease to prevent expiry |
| `complete_mesh_task` | Mark task finished; `output_json` payload recorded. Result artifacts can be created and linked separately via `create_artifact`. Accepts the row's `claim_lease_id`, `claimed_by_agent_id`, or (for `kind='assigned'` rows) `owner` match. |
| `fail_mesh_task` | Mark task failed with error details |
| `list_mesh_tasks` | Query tasks by status, domain, mission, or capability |

All tools require a valid session token and enforce domain-scoped access control.

## What's Next

- [Entity Reference: Task](/concepts/entity-reference/#task) — full column reference
- [Architecture: Data Flow](/architecture/data-flow/) — how task claims flow through the system
- [Concepts: ACP](/concepts/acp/) — the session model agents use to interact with the mesh
