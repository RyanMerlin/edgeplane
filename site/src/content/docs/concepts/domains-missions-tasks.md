---
title: Domains, Missions & Tasks
description: The organizational model — how work, knowledge, and governance scope together.
---

EdgePlane organizes work around three nested layers: **Domains**, **Missions**, and **Tasks**. Understanding their boundaries is essential for working with the system effectively.

## Domains

A Domain is:

- A **bounded objective** — the high-level "what we are doing and why"
- A **scoped knowledge domain** — context for all work inside it
- A **policy surface** — governance strictness, approval requirements
- A **permission boundary** — who can do what
- A **tool/skill profile** — approved tools, required skills, capability expectations

Domains carry a `northstar_md` narrative, owner list, contributor list, and visibility/status fields.

**Domains do not complete. They scope. Tasks complete.**

This distinction matters. A domain like "Build authentication system" provides context and governance for all work inside it indefinitely. Individual tasks inside that domain complete, but the domain itself remains as the scoping container.

### Domain Profiles

Each Domain defines a **Domain Profile** that agents and humans load when joining the domain:

- Approved tools and integrations
- Required skills and knowledge domains
- Governance strictness level
- Permission tiers
- Artifact structure expectations

Context switching between domains is structured and intentional. A contributor joins a domain, loads its profile, and operates with the correct tool set and governance posture immediately.

## Missions

A Mission is a **knowledge cluster inside a domain for a targeted outcome**. This is the workstream.

Missions are where:

- Artifacts cohere (documents, binaries, outputs)
- Context continuity lives across sessions
- Agents pick up and resume work without re-establishing context
- S3 storage is scoped: `domains/{domain_id}/missions/{mission_id}/{entity}/{filename}`

A mission has a `workstream_md` describing its targeted outcome, an optional domain anchor, owners, and status. Missions can be domain-free (useful for standalone workstreams not yet attached to a broader domain).

Do not call domains workstreams. Missions are workstreams.

### What lives in a Mission

| Entity | Description |
|--------|-------------|
| Tasks | Units of work with owners and definitions of done |
| MeshTasks | Agent-claimable tasks for distributed execution |
| Artifacts | Persisted outputs (documents, binaries, skill bundles) |

## Tasks

A Task is a **unit of work inside a mission**. It has:

- An owner
- Optional dependencies on other tasks
- A definition of done
- Status lifecycle (pending → in progress → complete / blocked)
- Links to related artifacts

Tasks complete. That is their purpose.

### Task vs. MeshTask

| | Task | MeshTask |
|---|---|---|
| **Purpose** | UI/operator-facing work tracking | Agent-claimable distributed execution |
| **Claim model** | Manual assignment | Lease-based claim by capable agents |
| **Capabilities** | Not required | Required capabilities gated by `claim_policy` |
| **Result** | Status update | Recorded as an artifact (`result_artifact_id`) |

For human-driven workflows, use Tasks. For agent swarms executing work autonomously, use MeshTasks. Whether these surfaces will converge is an open architecture question — for now, treat them as parallel.

## Overlap Detection

Before a task or artifact is created, EdgePlane runs:

- Fuzzy similarity analysis
- Vector similarity search
- Existing domain and mission state check
- Artifact history evaluation

Collisions surface before damage occurs. This enables safe parallelism at scale — multiple agents can work inside the same domain without stepping on each other.

## The Hierarchy in Practice

\`\`\`
Domain: "Build Authentication System"
├── northstar, owners, governance policy
├── Mission: "OIDC Integration"
│   ├── workstream_md, artifacts
│   ├── Task: "Implement /callback route"
│   ├── Task: "Write integration tests"
│   └── MeshTask: "Generate API docs"
└── Mission: "Token Management"
    ├── Task: "Design refresh token schema"
    └── Task: "Implement revocation endpoint"
\`\`\`

Domains scope. Missions stream. Tasks complete.

## See Also

- [Entity Reference](/concepts/entity-reference/) — full schema-backed definitions for every entity
- [Architecture: Persistence](/architecture/persistence/) — how domains, missions, and tasks are stored across Postgres, S3, and Git
