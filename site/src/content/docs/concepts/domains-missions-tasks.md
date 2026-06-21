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

Domains carry a **Northstar** narrative, owner list, contributor list, and visibility/status fields.

### Northstar and Brief Narratives

A Domain's **Northstar** is a narrative document describing its purpose, scope, and direction — the "why" that orients all work inside the domain. It answers questions like: what is this domain trying to achieve, what is out of scope, and what does success look like over the long term.

A Mission's **Brief** describes the targeted outcome for that mission — the "what and how" for the effort underway. It gives agents and contributors the context they need to pick up work without re-establishing intent from scratch.

Both are Markdown documents stored alongside the entity in S3 at the mission's scoped path. They are first-class fields, not free-form notes.

Authoring support via `edgeplane domain northstar edit` and `edgeplane mission brief edit` is available now. The expected structure for each is documented in the schema-pack templates at [`docs/schema-packs/NORTHSTAR.example.md`](https://github.com/edgeplane/edgeplane/blob/main/docs/schema-packs/NORTHSTAR.example.md) and [`docs/schema-packs/BRIEF.example.md`](https://github.com/edgeplane/edgeplane/blob/main/docs/schema-packs/BRIEF.example.md) in the repository.

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

A mission has a `brief_md` describing its targeted outcome, an optional domain anchor, owners, and status. Missions can be domain-free (useful for standalone workstreams not yet attached to a broader domain). The legacy `workstream_md` column remains for backward compatibility.

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

```
Domain: "Build Authentication System"
├── Northstar, owners, governance policy
├── Mission: "OIDC Integration"
│   ├── brief_md, artifacts
│   ├── Task: "Implement /callback route"
│   ├── Task: "Write integration tests"
│   └── MeshTask: "Generate API docs"
└── Mission: "Token Management"
    ├── Task: "Design refresh token schema"
    └── Task: "Implement revocation endpoint"
```

Domains scope. Missions stream. Tasks complete.

![Entity hierarchy diagram](/diagrams/entity-hierarchy.svg)

## See Also

- [Entity Reference](/concepts/entity-reference/) — full schema-backed definitions for every entity
- [Architecture: Persistence](/architecture/persistence/) — how domains, missions, and tasks are stored across Postgres, S3, and Git
- [Overlap Detection](/guides/overlap-detection/) — similarity analysis that surfaces duplicate work before it lands
- [Domain Access Control](/guides/governance-and-approvals/) — how owners/contributors control domain access
