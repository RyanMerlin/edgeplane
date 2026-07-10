---
title: What is EdgePlane?
description: A coordination layer for AI agents and human collaborators — structured domains, durable task ownership, and governed artifact publication.
---

AI agents can write code, run tools, and reason over architecture. What they cannot do is **coordinate**.

Without a shared system of record, parallel agents duplicate effort, diverge on state, and collide on artifacts with no resolution path. There is no overlap detection, no structured ownership, no audit trail, no governance boundary.

EdgePlane is the **coordination layer**. It is a control plane for AI agents and human collaborators operating against shared, durable, governed state.

> Kubernetes orchestrates containers.  
> EdgePlane orchestrates agents, domains, and knowledge.

## What EdgePlane is not

- Not a workflow runner
- Not a pipeline framework
- Not a chatbot UI

It is infrastructure that enables parallel, governed, auditable AI execution inside an organizational context.

## Core Primitives

**Domains** define the bounded objective, knowledge scope, and permission boundary (owners/contributors). Domains don't complete — they scope. Tasks complete.

**Missions** are workstreams inside a domain for a targeted outcome. Artifacts cohere here; context continuity lives here.

**Tasks** are units of work inside a mission. They have owners, dependencies, definitions of done, and status.

**Artifacts** are persisted outputs bound to a mission — documents, binaries, and agent results. Content is stored inline in Postgres today; S3-compatible object storage for artifact bytes is planned but not yet implemented.

**Agents** are identities (human or AI) that perform work. They carry capabilities, status, and a domain anchor.

## What EdgePlane Provides

| Capability | What it does |
|------------|-------------|
| **Overlap Detection** (planned) | The `get_overlap_suggestions` MCP tool and its backing table exist, but the background similarity analysis that would populate it isn't implemented yet — today the table is only ever written by tests |
| **Artifact Ledger** | Every mutation recorded in Postgres, committed to Git with full provenance |
| **MCP-Native Interface** | Standard MCP stdio tools via `edgeplane serve`, plus an HTTP MCP surface (`/api/mcp/tools`, `/api/mcp/call`) — works with any MCP-compatible agent runtime, no sidecar or custom SDK |
| **Authorization** | Membership-based, default-deny — per-domain `owners`/`contributors` plus an admin allowlist, enforced on HTTP and MCP. A versioned policy engine with approval tokens is on the roadmap |
| **Agent Client Protocol (ACP)** | Persistent agent sessions — `edgeplaned` manages agent processes; sessions reconnect after a restart with event replay for mid-session catch-up (full context restore across a daemon crash is not yet automatic) |
| **Web Dashboard** | React UI for fleet monitoring, live event feed, domain/task drill-down, and ACP conversation panes |
| **Mesh Execution** | Agents claim and execute `MeshTask` units from a shared queue — distributed work without a central scheduler |
| **Semantic Search** (planned) | Hybrid full-text + vector search across tasks, docs, and missions — no embeddings or vector queries are implemented yet |
| **Personal Agent Profiles** | Operator profiles travel with the operator, not the machine — sync across devices instantly |

## The Stack Position

Traditional software stacks have hardware → OS → containers → orchestration → application. The AI-native stack adds a missing layer:

```
Models
Agents
Tooling Interfaces (Skills / MCP)
Coordination Layer  ← EdgePlane
Working File Store (Postgres today; S3-compatible storage planned)
Authorization + Policy
Organizational Memory of Record (Git)
```

EdgePlane fills the coordination layer between autonomous agents and durable system state.

## Rust-Native Trust Boundary

`edgeplane` is a compiled Rust binary that carries MCP transport, policy context, session wiring, and local orchestration in one deterministic artifact. This reduces runtime dependency drift, narrows the local attack surface, and creates clear audit boundaries between the agent runtime and the control plane.

Agents request actions. EdgePlane authorizes and records them.

## Next Steps

- [Philosophy](/concepts/philosophy/) — the design principles behind these decisions
- [Domains, Missions & Tasks](/concepts/domains-missions-tasks/) — the organizational model in detail
- [Entity Reference](/concepts/entity-reference/) — canonical definitions for every entity
- [ACP — Agent Client Protocol](/concepts/acp/) — how persistent agent sessions work
- [MeshTask System](/concepts/mesh-tasks/) — distributed agent-to-agent task execution
- [Profiles](/concepts/profiles/) — personal operator profiles and how they travel
