---
title: MeshTask System
description: Distributed agent-to-agent task execution — how agents claim, execute, and complete work from a shared queue.
---

A `MeshTask` is a unit of work designed to be claimed and executed by any agent with the right capabilities. There is no central scheduler. Agents poll the mesh queue, claim what they can handle, and complete or fail with a recorded result.

## What a MeshTask Is

`MeshTask` and `Task` are parallel surfaces — both live inside a `Mission`, both have owners and status. The difference is the execution model:

- A `Task` is human-facing. It has an explicit owner, dependencies, and a definition of done. It is created and managed by operators.
- A `MeshTask` is agent-claimable. It has capability routing, a TTL lease, and a result artifact. It is designed to be consumed by automated agents.

Whether the two will converge is an open architecture question. For now, use `Task` for human-driven work and `MeshTask` for agent-driven work.

## The Claim-Execute-Complete Lifecycle

```
Submit          → status: pending
Claim           → status: claimed (lease issued, expires_at set)
Heartbeat       → lease extended
Complete/Fail   → status: completed | failed (result artifact recorded)
Lease expire    → status: pending again (available for re-claim)
```

A claimed task holds a lease. The claiming agent must send periodic heartbeats to extend the lease. If heartbeats stop (crash, network drop, stall), the lease expires and the task becomes available for another agent to claim. This makes the mesh self-healing — no task gets permanently stuck in a claimed state.

## Capabilities and Routing

Each `MeshTask` carries a `required_capabilities` field. When an agent polls for available tasks, it only sees tasks whose `required_capabilities` match a subset of its declared capabilities.

Agents declare capabilities at registration via `edgeplane agent register --capabilities`. At runtime, the `MeshAgent` row (the agent's runtime-bound projection) also carries `discovered_capabilities` — introspected at enrollment and unioned with declared capabilities during scheduling.

Tasks that require capabilities no registered agent holds will remain pending until a matching agent comes online.

## Parent-Child Structure

`MeshTask.parent_task_id` enables parallel fan-out. A parent task can remain in `claimed` status while spawning multiple child tasks. Children execute concurrently; the parent agent aggregates results when all children complete.

This is the mechanism behind wide agent workflows: one orchestrator task fans out N sub-tasks, each claimed by a capable agent, results recorded as artifacts.

## Result Artifacts

On completion, the agent records a result as an artifact at the standard S3 path:

```
domains/{domain_id}/missions/{mission_id}/artifacts/{filename}
```

The `MeshTask.result_artifact_id` FK points to this artifact. Callers can fetch the result via the artifact API without knowing the agent's identity or location.

## MeshTask vs Task

| | Task | MeshTask |
|---|---|---|
| Owner | Explicit, set at creation | Claimed by any matching agent |
| Routing | Manual assignment | Capability-based scheduling |
| Lease | None | TTL lease, expires on inactivity |
| Result | `definition_of_done` text | Artifact (`result_artifact_id`) |
| Use case | Human-driven work | Agent-driven distributed execution |

## MCP Tools for Mesh Execution

| Tool | What it does |
|------|-------------|
| `submit_mesh_task` | Create a new task in `pending` state |
| `claim_mesh_task` | Atomically claim a pending task; returns lease |
| `heartbeat_mesh_task` | Extend the lease to prevent expiry |
| `complete_mesh_task` | Mark task completed, record result artifact |
| `fail_mesh_task` | Mark task failed with error details |
| `list_mesh_tasks` | Query tasks by status, domain, mission, or capability |

All tools require a valid session token and enforce domain-scoped access control.

## What's Next

- [Entity Reference: MeshTask](/concepts/entity-reference/#meshtask) — full column reference
- [Architecture: Data Flow](/architecture/data-flow/) — how task claims flow through the system
- [Concepts: ACP](/concepts/acp/) — the session model agents use to interact with the mesh
