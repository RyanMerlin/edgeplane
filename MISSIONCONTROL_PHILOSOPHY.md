# MissionControl — Philosophy

## The Coordination Layer

AI agents can write code, run tools, and reason over architecture.
What they cannot do is coordinate.

Without a shared system of record, parallel agents duplicate effort,
diverge on state, and collide on artifacts. There is no overlap
detection, no structured ownership, no audit trail, no governance
boundary. The capability compounds; the coordination doesn't.

MissionControl is the coordination layer. It is a control plane for AI
agents and human collaborators operating against shared, durable,
governed state.

It is not a workflow runner. It is not a pipeline framework. It is not
a chatbot UI.

> Kubernetes orchestrates containers.\
> MissionControl orchestrates agents, missions, and knowledge.

------------------------------------------------------------------------

# The Core Thesis

AI agents can now write software.

But they cannot coordinate.

They lack:

-   Shared durable memory
-   Structured task ownership
-   Overlap detection
-   Governance boundaries
-   Permission hierarchies
-   Organizational context
-   Mission-scoped tooling
-   Personal operational profiles that travel with the operator
-   A working file store decoupled from prompt context
-   A long-term memory of record beyond the current session

MissionControl provides these primitives.

------------------------------------------------------------------------

# Architectural Position in the Stack

Traditional stack:

-   Hardware
-   OS
-   Containers
-   Orchestration
-   Application
-   CI/CD

AI-native stack:

-   Models
-   Agents
-   Tooling Interfaces (Skills/MCP)
-   **Coordination Layer (MissionControl)**
-   Working File Store (S3)
-   Governance + Policy
-   Organizational Memory of Record (Git)

MissionControl fills the missing coordination layer between autonomous
agents and durable system state.

------------------------------------------------------------------------

# Rust-Native Trust Boundary

MissionControl is intentionally Rust-forward at the agent edge.

The `mc` runtime is a compiled Rust binary that carries MCP transport,
policy context, session wiring, and local orchestration in one
deterministic artifact.

This supports enterprise requirements:

-   Reduced runtime dependency drift
-   Smaller local attack surface than ad-hoc script stacks
-   Stronger operational predictability for IT/SRE teams
-   Clearer audit boundaries between agent runtime and control plane

Security is not bolted on. It is part of the system boundary design:
agents request actions, MissionControl authorizes and records them.

------------------------------------------------------------------------

# Mission-Centric Organizational Model

MissionControl organizes work around **Missions**.

A Mission is:

-   A bounded objective
-   A scoped knowledge domain
-   A policy surface
-   A permission boundary
-   A tool/skill profile

Each Mission defines a **Mission Profile**:

-   Approved tools and integrations
-   Required skills and knowledge domains
-   Governance strictness level
-   Permission tiers
-   Artifact structure expectations

Teams and agents can switch between Mission Profiles when moving between
projects without losing integrity or context.

Context switching becomes structured, intentional, and safe.

------------------------------------------------------------------------

# Personal Agent Profiles

Every operator — human or agent — carries a personal profile.

A profile is a curated bundle of environment configuration, tool
settings, instruction files, and context that defines how that operator
engages with MissionControl and their local AI toolchain.

Profiles are:

-   Stored on the MissionControl backend, scoped strictly to the owner
-   Synced to the local machine automatically on agent startup
-   Applied via atomic symlink swap for clean, instant transitions
-   Versioned, pushable, and pullable from any client or machine
-   User-creatable and user-switchable by design (for example:
    research, coding, security review) with fast local transitions
-   Durable across machines through profile publish/pull/activate flows
    in MissionControl (config + auth references + policy-relevant context)

This means an agent operator can move between machines, reinstall their
toolchain, onboard to a new mission, or hand off context to a teammate
without losing their working configuration.

The agent's operational identity — its environment, its instruction
files, its tool profile — travels with the operator, not with the
machine.

Profile switching is structured and intentional. Context drift across
machines or sessions is eliminated.

Operators should be able to create a profile once, switch profiles in
seconds, and resume the same operational posture on any machine without
manual re-bootstrap work.

------------------------------------------------------------------------

# The Organizational Communication Layer

For AI-native coordination to scale inside real organizations, it must
meet users where they already operate.

That surface is wherever your team communicates — Slack, Microsoft
Teams, Google Chat, or any webhook-capable platform.

MissionControl is channel-agnostic by design. Each communication
provider is a pluggable integration, not a hard dependency. The core
coordination primitives — missions, tasks, approvals, artifacts — are
the same regardless of which channel surfaces them.

The communication layer provides:

-   Mission-aware notifications
-   Task creation from conversation threads
-   Overlap warnings delivered in-channel
-   Artifact publish alerts
-   Governance approval requests and responses
-   Search queries from within the communication tool
-   Role-aware mutation controls

The human-accessible edge of the coordination layer becomes the
platform your organization already uses.

Non-technical stakeholders can:

-   View mission state
-   Search knowledge
-   Review artifacts
-   Trigger workflows
-   Approve changes (if permitted)

Without leaving their existing communication workflow.

This lowers friction dramatically and accelerates adoption across
technical and non-technical teams alike.

MissionControl is not only agent-native — it is organization-native.

------------------------------------------------------------------------

# Shared Structured Memory

Agents and humans operate against durable, structured entities:

-   Missions - high level organizational goal/initiative
-   Klusters - knoweldge cluster inside mission for a targeted outcome
-   Tasks
-   Artifacts
-   Documents
-   Roles
-   Governance Policies

This eliminates prompt-fragmentation and creates continuity across time
and contributors.

Search is semantic. State is durable. Ownership is explicit.

------------------------------------------------------------------------

# Overlap Detection as a First-Class Primitive

MissionControl evaluates intent before mutation.

Before a task or artifact is created:

-   Fuzzy similarity analysis runs
-   Vector similarity search runs
-   Existing mission state is checked
-   Artifact history is evaluated

Collisions are detected proactively.

This enables safe parallelism at scale.

------------------------------------------------------------------------

# Governance and Permission Model

AI-native development without guardrails is not scalable.

MissionControl implements:

## Role Types

-   Admin: Full mutation and policy control
-   Contributor: Can create and modify within mission scope
-   Viewer: Can search, inspect, and utilize artifacts but cannot mutate
    state

## Policy Enforcement

-   Approval requirements for sensitive mutations
-   Publish controls
-   Mutation restrictions
-   Environment-specific overrides
-   Draft → Active → Rollback lifecycle

Governance is integrated directly into the execution path.

------------------------------------------------------------------------

# Persistence Architecture: Three-Tier Memory Model

MissionControl uses three distinct, complementary persistence layers.
Each serves a specific role in the information lifecycle.

## PostgreSQL — Structured State and Collaboration

The operational database. All structured entities — missions, klusters,
tasks, roles, governance policies, approval records — live in Postgres
with pgvector for semantic indexing.

This is the source of truth for:

-   Who owns what, and with what permissions
-   Current task and artifact status
-   Overlap detection state
-   Approval lifecycle records
-   Vector-indexed search across all entities

Postgres is the coordination substrate. Fast, queryable, role-scoped.

## S3 — Working File Persistence

Artifact content — documents, binaries, skill bundles, agent outputs —
is stored in S3-compatible object storage, not inline in the database.

S3 is the working store: immediately available, mutable during active
work, and scoped per mission and kluster:

    missions/{mission_id}/klusters/{kluster_id}/{entity}/{filename}

This means agents can read, write, and iterate on file content without
polluting the structured state database. Storage scales independently.
Any S3-compatible backend works — AWS S3, MinIO, RustFS — with no code
changes.

MissionControl ships with RustFS bundled in the Docker Compose stack.
No external infrastructure required to run with full file persistence
locally.

S3 is not optional infrastructure. It is where active work lives.

## Git — Long-Term Memory of Record

When a mutation is approved and published, it is committed to Git.

Git is the memory of record: immutable, auditable, and version-controlled.
Artifact provenance metadata (repo, branch, path, commit hash) is written
back to Postgres, creating a permanent link between the operational record
and the historical record.

The flow is:

1.  Agent produces artifact → stored in S3 (working)
2.  Mutation recorded in Postgres (structured state)
3.  Approval granted → committed to Git (memory of record)
4.  Provenance written back → full chain of custody established

This creates:

-   Traceable AI actions with deterministic lineage
-   Audit-ready change history outside the control plane
-   Reproducible mission state from Git alone if needed
-   A permanent organizational knowledge base that survives
    infrastructure changes

AI activity becomes accountable. The full trail — who did what, when,
approved by whom, committed where — is preserved at every layer.

------------------------------------------------------------------------

# Agent-Native Interface (MCP)

MissionControl is AI-first infrastructure.

Agents interact via structured MCP tool calls:

-   search_tasks
-   search_klusters
-   detect_overlaps
-   create_mission
-   publish_pending_ledger_events
-   list_pending_ledger_events
-   get_entity_history

The system is designed for autonomous orchestration, not manual UI
interaction.

------------------------------------------------------------------------

# Organizational Acceleration

MissionControl becomes a central nervous system for:

-   Rapid onboarding of new contributors and agents
-   Encoding institutional knowledge into durable, searchable state
-   Standardizing toolchains across teams and missions
-   Defining mission-scoped skills and capability profiles
-   Switching operational contexts safely and intentionally
-   Surfacing AI workflows through the communication tools teams
    already use

A new contributor — human or agent — attaches to a Mission Profile and
loads their personal agent profile.

The Mission defines:

-   Tools and approved integrations
-   Skills and knowledge domains
-   Governance strictness and approval requirements
-   State, ownership, and permission tiers
-   Communication surface (whichever channel the team uses)

This compresses onboarding time and reduces cognitive overhead for both
humans and AI agents joining a mission mid-flight.

------------------------------------------------------------------------

# Scalable Parallel AI Execution

Without coordination:

-   Agents duplicate effort
-   State diverges across sessions
-   Artifacts conflict with no resolution path
-   No clear ownership or audit trail

With MissionControl:

-   Parallel task execution is structured by mission and kluster scope
-   Overlap is detected before damage occurs
-   Ownership is explicit and role-enforced
-   State is synchronized across agents, sessions, and machines
-   Policy is enforced at every mutation point
-   Organizational stakeholders remain informed through whatever
    communication channel the team uses

This enables 5, 10, 50 agents to operate simultaneously inside a
coherent, governed, auditable system.

------------------------------------------------------------------------

# Vision

MissionControl is infrastructure for AI-native organizations.

As AI becomes a primary production actor, coordination becomes the
limiting factor.

MissionControl ensures that intelligence scales without fragmentation.

It connects agents, humans, governance, and communication into a single
coordinated execution layer.

It turns isolated AI capability into governed, mission-driven execution
at scale.
