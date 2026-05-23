---
title: What is MissionControl?
description: A coordination layer for AI agents and human collaborators — structured domains, durable task ownership, and governed artifact publication.
---

AI agents can write code, run tools, and reason over architecture. What they cannot do is **coordinate**.

Without a shared system of record, parallel agents duplicate effort, diverge on state, and collide on artifacts with no resolution path. There is no overlap detection, no structured ownership, no audit trail, no governance boundary.

MissionControl is the **coordination layer**. It is a control plane for AI agents and human collaborators operating against shared, durable, governed state.

> Kubernetes orchestrates containers.  
> MissionControl orchestrates agents, domains, and knowledge.

## What MissionControl is not

- Not a workflow runner
- Not a pipeline framework
- Not a chatbot UI

It is infrastructure that enables parallel, governed, auditable AI execution inside an organizational context.

## Core Primitives

**Domains** define the bounded objective, knowledge scope, permission boundary, and governance policy. Domains don't complete — they scope. Tasks complete.

**Missions** are workstreams inside a domain for a targeted outcome. Artifacts cohere here; context continuity lives here.

**Tasks** are units of work inside a mission. They have owners, dependencies, definitions of done, and status.

**Artifacts** are persisted outputs bound to a mission — documents, binaries, skill bundles, agent results. Stored in S3-compatible object storage.

**Agents** are identities (human or AI) that perform work. They carry capabilities, status, and a domain anchor.

## What MissionControl Provides

| Capability | What it does |
|------------|-------------|
| **Overlap Detection** | Fuzzy + vector similarity runs before task and artifact creation — collisions surface before damage occurs |
| **Artifact Ledger** | Every mutation recorded in Postgres, vector-indexed for search, committed to Git with full provenance |
| **MCP-Native Interface** | Standard MCP stdio tools — works with any MCP-compatible agent runtime |
| **Governance & Approvals** | Versioned policy lifecycle, role-based access, approval tokens |
| **Persistent Agent Sessions** | `mcd` manages long-running agent processes; sessions survive crashes and reconnects |
| **Semantic Search** | Tasks, docs, and missions are vector-indexed (pgvector) for similarity and hybrid search |
| **Personal Agent Profiles** | Operator profiles travel with the operator, not the machine — sync across devices instantly |

## The Stack Position

Traditional software stacks have hardware → OS → containers → orchestration → application. The AI-native stack adds a missing layer:

```
Models
Agents
Tooling Interfaces (Skills / MCP)
Coordination Layer  ← MissionControl
Working File Store (S3)
Governance + Policy
Organizational Memory of Record (Git)
```

MissionControl fills the coordination layer between autonomous agents and durable system state.

## Rust-Native Trust Boundary

`mc` is a compiled Rust binary that carries MCP transport, policy context, session wiring, and local orchestration in one deterministic artifact. This reduces runtime dependency drift, narrows the local attack surface, and creates clear audit boundaries between the agent runtime and the control plane.

Agents request actions. MissionControl authorizes and records them.

## Next Steps

- [Philosophy](/missioncontrol/concepts/philosophy/) — the design principles behind these decisions
- [Domains, Missions & Tasks](/missioncontrol/concepts/domains-missions-tasks/) — the organizational model in detail
- [Entity Reference](/missioncontrol/concepts/entity-reference/) — canonical definitions for every entity
