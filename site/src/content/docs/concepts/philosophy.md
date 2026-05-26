---
title: Philosophy
description: The design principles and thesis behind EdgePlane.
---

## The Problem

AI agents can now write software.

But they cannot coordinate.

They lack:

- Shared durable memory
- Structured task ownership
- Overlap detection
- Governance boundaries
- Permission hierarchies
- Organizational context
- Domain-scoped tooling
- A working file store decoupled from prompt context
- A long-term memory of record beyond the current session

EdgePlane provides these primitives.

## The Coordination Layer

Without a shared system of record, parallel agents duplicate effort, diverge on state, and collide on artifacts. There is no overlap detection, no structured ownership, no audit trail, no governance boundary. The capability compounds; the coordination doesn't.

EdgePlane is the coordination layer. It is a control plane for AI agents and human collaborators operating against shared, durable, governed state.

It is not a workflow runner. It is not a pipeline framework. It is not a chatbot UI.

> Kubernetes orchestrates containers.  
> EdgePlane orchestrates agents, domains, and knowledge.

## Domain-Centric Organizational Model

Work is organized around **Domains** — bounded objectives that carry knowledge scope, policy, and permission boundaries. Every artifact, task, and agent session belongs to a domain (via its mission).

Each Domain defines a **Domain Profile**:

- Approved tools and integrations
- Required skills and knowledge domains
- Governance strictness level
- Permission tiers
- Artifact structure expectations

Context switching becomes structured, intentional, and safe. A contributor joins a domain, loads its profile, and operates with the correct tool set immediately — whether they are human or AI.

## Personal Agent Profiles

Every operator — human or AI — carries a personal profile: a curated bundle of environment configuration, tool settings, and instruction files that defines how that operator engages with EdgePlane and their local AI toolchain.

Profiles are:

- Stored server-side, scoped strictly to the owner
- Synced to the local machine on agent startup via atomic symlink swap
- Versioned, pushable, and pullable from any machine
- User-creatable and user-switchable (research, coding, security review)
- Durable across machines through profile publish/pull/activate flows

The agent's operational identity — its environment, instruction files, tool profile — travels with the operator, not with the machine.

## Overlap Detection as a First-Class Primitive

EdgePlane evaluates intent before mutation.

Before a task or artifact is created:

- Fuzzy similarity analysis runs
- Vector similarity search runs
- Existing domain state is checked
- Artifact history is evaluated

Collisions are detected proactively. This enables safe parallelism at scale.

## Governance and Permission Model

AI-native development without guardrails does not scale.

**Roles:**

| Role | Permissions |
|------|-------------|
| Admin | Full mutation and policy control |
| Contributor | Create and modify within domain scope |
| Viewer | Search, inspect, and use artifacts — no mutations |

**Policy enforcement:**

- Approval requirements for sensitive mutations
- Publish controls
- Mutation restrictions
- Environment-specific overrides
- Draft → Active → Rollback lifecycle

Governance is integrated directly into the execution path, not bolted on after the fact.

## Three-Tier Persistence

EdgePlane uses three complementary persistence layers. Each serves a specific role in the information lifecycle.

**PostgreSQL — Structured State**

All entities (domains, missions, tasks, artifacts, roles, governance policies, approval records) live in Postgres with pgvector for semantic indexing. This is the coordination substrate — fast, queryable, role-scoped.

**S3-Compatible Object Storage — Working File Store**

Artifact content lives in S3-compatible object storage, not inline in the database. Storage scales independently. Any S3-compatible backend works with no code changes. S3 is not optional infrastructure — it is where active work lives.

**Git — Long-Term Memory of Record**

When a mutation is approved and published, it is committed to Git. Artifact provenance metadata (repo, branch, path, commit hash) is written back to Postgres, creating a permanent link between the operational record and the historical record.

The flow:

1. Agent produces artifact → stored in S3 (working)
2. Mutation recorded in Postgres (structured state)
3. Approval granted → committed to Git (memory of record)
4. Provenance written back → full chain of custody established

AI activity becomes accountable. The full trail — who did what, when, approved by whom, committed where — is preserved at every layer.

## Rust-Native Trust Boundary

EdgePlane is intentionally Rust-forward at the agent edge.

The `edgeplane` runtime is a compiled Rust binary that carries MCP transport, policy context, session wiring, and local orchestration in one deterministic artifact.

This supports organizational requirements:

- Reduced runtime dependency drift
- Smaller local attack surface than ad-hoc script stacks
- Stronger operational predictability for IT/SRE teams
- Clearer audit boundaries between agent runtime and control plane

Security is not bolted on. It is part of the system boundary design: agents request actions, EdgePlane authorizes and records them.

## MCP-Native Interface

EdgePlane is AI-first infrastructure. Agents interact via structured MCP tool calls:

- `search_tasks` / `search_missions` — semantic search
- `detect_overlaps` — pre-mutation collision detection
- `create_domain` / `create_mission` / `create_task`
- `publish_pending_ledger_events` — commit approved mutations to Git
- `get_entity_history` — full provenance chain

The system is designed for autonomous orchestration. The TUI and CLI are operator conveniences, not the primary interface.

## Vision

As AI becomes a primary production actor, coordination becomes the limiting factor. EdgePlane ensures that intelligence scales without fragmentation.

It connects agents, humans, governance, and organizational communication into a single coordinated execution layer. Isolated AI capability becomes governed, domain-and-mission-driven execution at scale.
