---
title: Data Flow
description: How a task moves through EdgePlane — from creation to agent execution to artifact publication.
---

This page traces two flows: the MeshTask lifecycle (how distributed work is coordinated between agents) and the ACP session flow (how a chat message from the web dashboard reaches an agent's terminal and back).

---

## MeshTask Lifecycle

MeshTasks are the primitive for distributing work across multiple agents. An orchestrator agent creates tasks; executor agents claim and complete them.

### 1. Task Submission

An agent calls the `submit_mesh_task` MCP tool:

```json
{
  "mission_id": "<id>",
  "title": "Analyze Q2 results",
  "description": "...",
  "priority": 5,
  "input_json": "{...}"
}
```

edgeplane-tower inserts the `MeshTask` record with `status = ready` and broadcasts a scoped SSE event to subscribers on the mission.

### 2. Agent Discovery

edgeplaned or any subscribed agent polls for available work using `list_mesh_tasks`:

```json
{ "mission_id": "<id>", "status": "ready" }
```

The response includes all tasks in `ready` status. Agents filter by capability requirements or task type.

### 3. Claim (Atomic Lease)

An agent calls `claim_mesh_task`:

```json
{ "task_id": "<id>", "agent_id": "<id>", "lease_seconds": 300 }
```

edgeplane-tower updates the row atomically: `status = claimed`, sets `lease_expires_at = now + lease_seconds`, and returns a `claim_lease_id`. If two agents race to claim the same task, only one wins — the other receives an error and can retry or move on.

### 4. Execution with Heartbeat

While executing, the agent sends `heartbeat_mesh_task` periodically to renew the lease:

```json
{ "task_id": "<id>", "claim_lease_id": "<lease_id>" }
```

If heartbeats stop (network partition, agent crash), the lease expires and `status` reverts to `ready`. Another agent can then claim it.

Agents can also emit typed progress events via `progress_mesh_task` — these appear in the SSE feed for the mission.

### 5. Task Completion

When done, the agent calls `complete_mesh_task`:

```json
{
  "task_id": "<id>",
  "claim_lease_id": "<lease_id>",
  "output_json": "{\"result\": \"...\"}"
}
```

edgeplane-tower sets `status = finished` and stores `output_json` on the task record.

### 6. Artifact Publication (Optional)

If the task produced output worth persisting, the agent calls `create_artifact`:

```json
{
  "mission_id": "<id>",
  "name": "analysis.md",
  "artifact_type": "document",
  "content_b64": "...",
  "content_sha256": "..."
}
```

edgeplane-tower streams the bytes to S3, inserts an artifact record in Postgres (with SHA-256 content hash for integrity), vector-indexes the content for later search, and writes a ledger entry. The artifact is now addressable by its ID and queryable by content similarity.

### 7. Overlap Detection

Before a task or artifact is created, edgeplane-tower checks for semantic overlap with existing tasks and artifacts in the same mission. Fuzzy text and vector similarity checks run against the pgvector index. If high-confidence overlaps exist, they are surfaced to the caller (via `get_overlap_suggestions`) before any write, preventing duplicate work.

---

## State at Each Stage

| Stage | Postgres | S3 | SSE broadcast |
|-------|----------|----|---------------|
| Submitted | `MeshTask` inserted (`status=ready`) | — | `task.created` event on mission |
| Claimed | `status=claimed`, `lease_expires_at` set | — | `task.claimed` event |
| Heartbeat | `lease_expires_at` extended | — | — |
| Progress | Progress event row inserted | — | `task.progress` event |
| Completed | `status=finished`, `output_json` stored | — | `task.completed` event |
| Artifact created | `Artifact` row inserted, vector index updated | Artifact bytes written | `artifact.created` event |
| Lease expired (no heartbeat) | `status=ready` (reverted) | — | `task.lease_expired` event |

---

## ACP Session Data Flow

The ACP (Agent Communication Protocol) session is how the web dashboard — or any WebSocket client — talks directly to an agent's terminal.

```
Browser (Web Dashboard)
  │  WS frame: { "kind": "prompt", "text": "summarize this" }
  │
  ▼
edgeplane-tower  (/api/runtime/nodes/{node_id}/agents/{agent_id}/attach)
  │  Validates session token / owner scope
  │  Proxies the WebSocket frame
  │
  ▼
edgeplaned (on the agent's node)
  │  PTY bridge: converts prompt frame → text + newline → writes to agent stdin
  │
  ▼
Agent subprocess (Claude Code, Codex, etc.)
  │  Processes input, produces output on stdout
  │
  ▼
edgeplaned PTY bridge
  │  Reads stdout, packages as SSE/WS frames
  │
  ▼
edgeplane-tower (proxies back)
  │
  ▼
Browser (renders output in terminal view)
```

The tower acts as the trust checkpoint: it validates that the connecting user owns or has access to the target agent before the WebSocket is established. After that, the PTY stream is bidirectional.

---

## What's Next

- [Security Model](/architecture/security/) — how authentication and authorization work at each boundary
- [Concepts: MeshTask System](/concepts/mesh-tasks/) — task states, claim policy, and capability enforcement
- [Concepts: ACP](/concepts/acp/) — the Agent Communication Protocol in detail
