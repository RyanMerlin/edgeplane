# Edgeplane — Philosophy

> **For precise entity definitions (Domain, Mission, Task, Artifact, Session, Agent), see [`docs/architecture/entities.md`](docs/architecture/entities.md).** This doc is the *why*; entities.md is the *what*. If they disagree, entities.md wins until reconciled.

## The Coordination Layer

AI agents can write code, run tools, and reason over architecture.
What they cannot do is coordinate.

Without a shared system of record, parallel agents duplicate effort,
diverge on state, and collide on artifacts. There is no overlap
detection, no structured ownership, no audit trail, no governance
boundary. The capability compounds; the coordination doesn't.

Edgeplane is the coordination layer. It is a control plane for AI
agents and human collaborators operating against shared, durable,
governed state.

It is not a workflow runner. It is not a pipeline framework. It is not
a chatbot UI.

> Kubernetes orchestrates containers.\
> Edgeplane orchestrates agents, domains, missions, and knowledge.

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
-   Domain/mission-scoped tooling
-   Personal operational profiles that travel with the operator
-   A working file store decoupled from prompt context
-   A long-term memory of record beyond the current session

Edgeplane provides these primitives.

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
-   **Coordination Layer (Edgeplane)**
-   Working File Store (S3)
-   Governance + Policy
-   Organizational Memory of Record (Git)

Edgeplane fills the missing coordination layer between autonomous
agents and durable system state.

------------------------------------------------------------------------

# Rust-Native Trust Boundary

Edgeplane is intentionally Rust-forward at the agent edge.

The `edgeplane` runtime is a compiled Rust binary that carries MCP transport,
policy context, session wiring, and local orchestration in one
deterministic artifact.

This supports enterprise requirements:

-   Reduced runtime dependency drift
-   Smaller local attack surface than ad-hoc script stacks
-   Stronger operational predictability for IT/SRE teams
-   Clearer audit boundaries between agent runtime and control plane

Security is not bolted on. It is part of the system boundary design:
agents request actions, Edgeplane authorizes and records them.

------------------------------------------------------------------------

# Domain-Centric Organizational Model

Edgeplane organizes work around **Domains** and **Missions**.

A Domain is:

-   A bounded objective
-   A scoped knowledge domain
-   A policy surface
-   A permission boundary
-   A tool/skill profile

Each Domain defines a **Domain Profile**:

-   Approved tools and integrations
-   Required skills and knowledge domains
-   Governance strictness level
-   Permission tiers
-   Artifact structure expectations

A Mission is a workstream inside a Domain — where artifacts cohere and
context continuity lives.

Teams and agents can switch between Domain Profiles when moving between
projects without losing integrity or context.

Context switching becomes structured, intentional, and safe.

------------------------------------------------------------------------

# Personal Agent Profiles

Every operator — human or agent — carries a personal profile.

A profile is a curated bundle of environment configuration, tool
settings, instruction files, and context that defines how that operator
engages with Edgeplane and their local AI toolchain.

Profiles are:

-   Stored on the Edgeplane backend, scoped strictly to the owner
-   Synced to the local machine via explicit publish/pull/activate flows
    (goal: automatic sync on every agent startup; not yet built)
-   Applied via atomic symlink swap for clean, instant transitions
-   Versioned, pushable, and pullable from any client or machine
-   User-creatable and user-switchable by design (for example:
    research, coding, security review) with fast local transitions
-   Durable across machines through profile publish/pull/activate flows
    in Edgeplane (config + auth references + policy-relevant context)

This means an agent operator can move between machines, reinstall their
toolchain, onboard to a new domain or mission, or hand off context to a teammate
without losing their working configuration.

The agent's operational identity — its environment, its instruction
files, its tool profile — travels with the operator, not with the
machine.

Profile switching is structured and intentional. The goal is to
eliminate context drift across machines or sessions.

Operators should be able to create a profile once, switch profiles in
seconds, and resume the same operational posture on any machine without
manual re-bootstrap work.

------------------------------------------------------------------------

# The Organizational Communication Layer

For AI-native coordination to scale inside real organizations, it must
meet users where they already operate.

That surface is wherever your team communicates — Slack, Microsoft
Teams, Google Chat, or any webhook-capable platform.

Edgeplane is channel-agnostic by design. Each communication
provider is a pluggable integration, not a hard dependency. The core
coordination primitives — missions, tasks, approvals, artifacts — are
the same regardless of which channel surfaces them.

The communication layer provides:

-   Domain/mission-aware notifications
-   Task creation from conversation threads
-   Overlap warnings delivered in-channel
-   Artifact publish alerts
-   Governance approval requests and responses
-   Search queries from within the communication tool
-   Role-aware mutation controls

The human-accessible edge of the coordination layer becomes the
platform your organization already uses.

Non-technical stakeholders can:

-   View domain and mission state
-   Search knowledge
-   Review artifacts
-   Trigger workflows
-   Approve changes (if permitted)

Without leaving their existing communication workflow.

This lowers friction dramatically and accelerates adoption across
technical and non-technical teams alike.

Edgeplane is not only agent-native — it is organization-native.

------------------------------------------------------------------------

# Shared Structured Memory

Agents and humans operate against durable, structured entities:

-   Domains - high level organizational goal/initiative (bounded org objective)
-   Missions - knowledge cluster inside a domain for a targeted outcome (workstream)
-   Tasks
-   Artifacts
-   Documents
-   Roles
-   Governance Policies (planned — see Governance and Permission Model below)

This eliminates prompt-fragmentation and creates continuity across time
and contributors.

Search is structured today; semantic search is on the roadmap. State is
durable. Ownership is explicit.

------------------------------------------------------------------------

# Overlap Detection as a First-Class Primitive (Roadmap)

The goal is for Edgeplane to evaluate intent before mutation.

Before a task or artifact is created, the design calls for:

-   Fuzzy similarity analysis
-   Vector similarity search
-   Existing domain and mission state checked
-   Artifact history evaluated

so collisions are detected proactively.

**Current state:** the overlap-detection storage exists in the schema,
but no similarity computation runs against it yet — only test fixtures
populate it today. Existing domain/mission state is checked at
mutation time via ownership; the fuzzy/vector similarity layer above
it is not yet built.

This is designed to enable safe parallelism at scale.

------------------------------------------------------------------------

# Governance and Permission Model

AI-native development without guardrails is not scalable.

Edgeplane implements membership-based, default-deny authorization today.
A fuller governance model — role tiers, approval workflows, policy
lifecycle — is the goal but is not yet built.

## Membership (shipped)

-   Owner / Contributor: per-domain membership controls who can create
    and modify within that domain's scope
-   Admin allowlist: `EP_ADMIN_EMAILS` / `EP_ADMIN_GROUPS` grant
    cross-domain override

A dedicated Viewer role (read-only, no mutation) is planned; today
access within a domain is owner/contributor or nothing.

## Policy Enforcement (roadmap)

-   Approval requirements for sensitive mutations
-   Publish controls
-   Mutation restrictions
-   Environment-specific overrides
-   Draft → Active → Rollback lifecycle

**Current state:** a versioned policy-lifecycle and HMAC-signed approval
engine existed in an earlier schema and was removed
(`0009_drop_governance.sql`) as never-enforced, pending a redesign. The
goal remains governance integrated directly into the execution path.

------------------------------------------------------------------------

# Persistence Architecture: Three-Tier Memory Model

Edgeplane's target architecture is three distinct, complementary
persistence layers, each serving a specific role in the information
lifecycle. Today, artifact content lives inline in Postgres rather
than in the S3 tier described below — see "Current state" under S3.

## PostgreSQL — Structured State and Collaboration

The operational database. All structured entities — domains, missions,
tasks, roles — live in Postgres, along with artifact content itself
(inline, not in object storage — see S3 below). pgvector-backed semantic
indexing is planned; there is no embeddings/vector search in the
current build.

This is the source of truth for:

-   Who owns what, and with what permissions
-   Current task and artifact status
-   Overlap detection state (schema only — see Overlap Detection above)

Postgres is the coordination substrate. Fast, queryable, role-scoped.

## S3 — Working File Persistence (Roadmap)

The goal: artifact content — documents, binaries, skill bundles, agent
outputs — stored in S3-compatible object storage, not inline in the
database, scoped per domain and mission:

    domains/{domain_id}/missions/{mission_id}/{entity}/{filename}

**Current state:** artifact content lives inline in Postgres today.
The S3 write path for artifact content is not yet implemented — RustFS
ships bundled in the Docker Compose stack, but nothing writes artifact
bodies to it yet.

Once shipped, this is designed to let agents read, write, and iterate
on file content without polluting the structured state database, with
storage scaling independently. Any S3-compatible backend should work —
AWS S3, MinIO, RustFS — with no code changes.

## Git — Long-Term Memory of Record

When an artifact is published, it is committed to Git through a
policy-routed publish flow: connections and bindings resolved via
`resolve_publish_plan` determine the target repo/branch/path.

Git is the memory of record: immutable, auditable, and version-controlled.
Artifact provenance metadata (repo, branch, path, commit hash) is written
back to Postgres, creating a permanent link between the operational record
and the historical record.

The flow today is:

1.  Agent produces artifact → stored inline in Postgres (working; see
    S3 section above for the planned working-store tier)
2.  Mutation recorded in Postgres (structured state)
3.  Publish plan resolved against domain policy → committed to Git
    (memory of record)
4.  Provenance written back → full chain of custody established

This creates:

-   Traceable AI actions with deterministic lineage
-   Audit-ready change history outside the control plane
-   The goal of reproducible domain/mission state from Git alone —
    today, Git holds published artifacts and their provenance, not the
    full structured-state graph (ownership, task status, etc.), which
    lives in Postgres
-   A permanent organizational knowledge base that survives
    infrastructure changes

AI activity becomes accountable. The full trail — who did what, when,
published where — is preserved at every layer.

------------------------------------------------------------------------

# Agent-Native Interface (MCP)

Edgeplane is AI-first infrastructure.

Agents interact via structured MCP tool calls:

-   search_tasks
-   search_missions
-   detect_overlaps
-   create_domain
-   create_mission
-   publish_pending_ledger_events
-   list_pending_ledger_events
-   get_entity_history

The system is designed for autonomous orchestration, not manual UI
interaction.

------------------------------------------------------------------------

# Organizational Acceleration

Edgeplane becomes a central nervous system for:

-   Rapid onboarding of new contributors and agents
-   Encoding institutional knowledge into durable, searchable state
-   Standardizing toolchains across teams and domains
-   Defining domain/mission-scoped skills and capability profiles
-   Switching operational contexts safely and intentionally
-   Surfacing AI workflows through the communication tools teams
    already use

A new contributor — human or agent — attaches to a Domain Profile and
loads their personal agent profile.

The Domain defines:

-   Tools and approved integrations
-   Skills and knowledge domains
-   Governance strictness and approval requirements
-   State, ownership, and permission tiers
-   Communication surface (whichever channel the team uses)

Within a domain, missions (workstreams) carry the active work context.

This compresses onboarding time and reduces cognitive overhead for both
humans and AI agents joining a domain or mission mid-flight.

------------------------------------------------------------------------

# Scalable Parallel AI Execution

Without coordination:

-   Agents duplicate effort
-   State diverges across sessions
-   Artifacts conflict with no resolution path
-   No clear ownership or audit trail

With Edgeplane:

-   Parallel task execution is structured by domain and mission scope
-   Overlap detection catching collisions before damage occurs is the
    goal (see Overlap Detection above for current state)
-   Ownership is explicit and membership-enforced
-   State is synchronized across agents, sessions, and machines
-   Membership-based access is enforced at every mutation point (a
    fuller policy engine is on the roadmap — see Governance above)
-   Organizational stakeholders remain informed through whatever
    communication channel the team uses

This enables 5, 10, 50 agents to operate simultaneously inside a
coherent, governed, auditable system.

------------------------------------------------------------------------

# Vision

Edgeplane is infrastructure for AI-native organizations.

As AI becomes a primary production actor, coordination becomes the
limiting factor.

Edgeplane ensures that intelligence scales without fragmentation.

It connects agents, humans, governance, and communication into a single
coordinated execution layer.

It turns isolated AI capability into governed, domain-and-mission-driven
execution at scale.
