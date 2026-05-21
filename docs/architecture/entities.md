# MissionControl — Canonical Entity Reference

**This is the single source of truth for what each MC entity means.** If anything in another doc, code comment, or AI response contradicts this file, this file wins. Update this file *first*, then propagate.

Every entity below cites both the philosophy doc (definition) and the schema (structural truth). Drift between them is a bug — file an issue, fix here first.

- **Schema:** `crates/mc-controlplane/migrations/0001_initial_schema.sql`
- **Philosophy:** `MISSIONCONTROL_PHILOSOPHY.md`

---

## Mission

**A bounded organizational objective.** The high-level "what we are doing and why." Carries the northstar narrative, owners, governance scope.

- Philosophy: line 98–106 ("A Mission is: A bounded objective; A scoped knowledge domain; A policy surface; A permission boundary; A tool/skill profile")
- Schema: `public.mission` (0001) — has `northstar_md`, `owners`, `contributors`, `visibility`, `status`
- Owns: many klusters (`kluster.mission_id`)

> **DEPRECATED:** `Mission.kind` (migration 0006, values `'work'` | `'home'`) is soft-deprecated as of 2026-05-21. The column was set in exactly one code path (`provision_home_for_node` in `routes/runtime.rs`) and read by zero — a write-only tag that leaked an Aria-specific operational pattern into the schema. New code MUST NOT write or filter on `kind`. The column stays for now (migrations are forward-only); a future migration may drop it. Operational coordination missions are just regular missions; convention names them (e.g. `aria-fleet-ops`) and `Agent.home_mission_id` still points at them as before — that FK was never constrained to `kind='home'` anyway.

Missions do **not** complete. They scope. Tasks complete.

---

## Kluster

**A knowledge cluster inside a mission for a targeted outcome. This is the workstream.** Klusters are where artifacts cohere and where context continuity lives.

- Philosophy: line 207 ("Klusters - knowledge cluster inside mission for a targeted outcome")
- Schema: `public.kluster` (line 442) — has `workstream_md`, `workstream_version`, `mission_id` (nullable — klusters can be mission-free), `owners`, `status`
- S3 layout: `missions/{mission_id}/klusters/{kluster_id}/{entity}/{filename}` (philosophy line 290)
- Owns: tasks (`task.kluster_id`), meshtasks (`meshtask.kluster_id`), artifacts (`artifact.kluster_id`)

The column `kluster.workstream_md` is the literal name. A kluster *is* a workstream. Do not call missions workstreams.

---

## Task

**A unit of work inside a kluster.** Has owner, dependencies, definition of done, status. Completes.

- Schema: `public.task` (line 1070) — has `kluster_id` (FK, required), `epic_id` (optional), `owner`, `definition_of_done`, `dependencies`, `related_artifacts`
- No direct `mission_id` — mission is reached via kluster

The local/UI-facing task. See `meshtask` for the agent-claimable mesh variant.

---

## MeshTask

**An agent-claimable task in the mesh.** Same role as `task`, but built for distributed claim-and-execute by agents with leases, capabilities, and parent/child structure.

- Schema: `public.meshtask` (line 550) — has `kluster_id` (required), `mission_id` (denormalized, required), `parent_task_id`, `claim_policy`, `required_capabilities`, `claimed_by_agent_id`, `claim_lease_id`, `lease_expires_at`, `result_artifact_id`
- Result is recorded as an artifact (`result_artifact_id`)
- Links to artifacts via `meshtaskartifact` (input/output role)

Whether `task` and `meshtask` will converge is an open architecture question — for now, treat them as parallel surfaces, mesh for agents.

---

## Artifact

**A persisted output bound to a kluster.** Documents, binaries, skill bundles, agent results — anything S3-stored.

- Philosophy: line 287–301 ("S3 is the working store... scoped per mission and kluster")
- Schema: `public.artifact` (line 184) — has `kluster_id` (required), `uri`, `storage_backend`, `content_sha256`, `version`, `provenance`, `status`
- Storage path: `missions/{mission_id}/klusters/{kluster_id}/{entity}/{filename}` (philosophy line 290)
- Owns: nothing — artifacts are leaves. Linked from tasks via `meshtaskartifact`.

---

## Agent

**An identity that performs work.** Human or AI. Carries capabilities, status, metadata, mission anchor.

- Philosophy: line 122–149 (agent profiles travel with the operator)
- Schema: `public.agent` (0001) — base columns: `name`, `capabilities`, `status`, `metadata`
- Migration 0007 adds: `archived_at`, `display_name`, `node_id`, `last_seen_at` (lifecycle + presence metadata; reserved-name enforcement happens in code, not as a CHECK constraint)
- Migration 0008 adds: `public_id` (`{name}-{8-char-suffix}`) — the stable, human-readable identifier used by `/agents/{public_id}/messages` and the unified `mc agent` surface. Immutable after creation
- Migration 0010 adds: `home_mission_id` (permanent anchor — set once at registration, never cleared) and `current_mission_id` (active attachment — follows the agent's working context, resets to home on detach). Both nullable FKs to `mission(id)`

**Note:** there is no separate `agent_identity` table. Migration 0007 only adds columns to `agent`. Earlier doc revisions claimed a distinct table — that was inaccurate.

See `MeshAgent` (below) for the discoverable, runtime-bound projection.

---

## MeshAgent

**The runtime-bound, discoverable projection of an agent into the mesh.** This is the row the controlplane scheduler matches against when claiming a `meshtask`. Distinct from `agent` (the canonical identity row).

- Schema: `public.meshagent` (0001) — has `mission_id`, `node_id`, `runtime_kind`, `runtime_version`, `capabilities`, `labels`, `status`, `current_task_id`, `enrolled_by_subject`, `enrolled_at`, `last_heartbeat_at`, `runtime_node_id`, `profile_json`, `machine_json`, `runtime_json`, `supervision_mode`
- Migration 0004 adds: `discovered_capabilities` — runtime-introspected capability set, union'd with declared `capabilities` during scheduling
- Migration 0009 adds: `agent_public_id` (nullable FK to `agent.public_id`) — links the mesh projection back to the canonical agent identity. Nullable so legacy meshagent rows can exist without a paired `agent`

Why two tables: `agent` is identity (who); `meshagent` is presence + capability (where + what they can do right now). One agent identity can have multiple meshagent rows over time as it enrolls on different nodes.

---

## Session entities — three layers, separate concerns

There are three "session" tables. Confusion here is the root cause of past architecture mistakes. Each layer has a distinct responsibility.

### AISession — logical AI conversation

**The persistent AI session as a logical entity.** Title, owner, runtime kind, status. Survives across runs.

- Schema: `public.aisession` (0001) — has `id`, `owner_subject`, `title`, `status`, `runtime_kind` (e.g. `claude_agent_acp`), `runtime_session_id`, `workspace_path`, `policy_json`, `capability_snapshot_json`
- `status` tracks lifecycle (active/ended/etc.); `capability_snapshot_json` freezes the capability set at session start for policy enforcement consistency across the session's lifespan
- This is what the user sees as "a session" — what you'd resume.

### AgentSession — local agent ↔ claude binding

**An agent's local handle on a claude session.** Connects an agent identity to a `claude_session_id` over a time window.

- Schema: `public.agentsession` (line 55) — has `agent_id` (FK), `claude_session_id`, `context` (free-text), `started_at`, `ended_at`, `end_reason`, `audit_log`
- Note: does **not** carry kluster_id or mission_id. Work-binding is via `agentrun` (below).

### ExecutionSession — runtime compute slot

**A leased compute slot for running an agent.** Pure infrastructure: which lease, which runtime class, PTY-or-not, attach token prefix.

- Schema: `public.executionsession` (line 331) — has `lease_id` (FK), `runtime_class`, `pty_requested`, `attach_token_prefix`, `status`
- Not for resume-key lookups. Compute-tier only.

### AgentRun — the actual binding

**A single execution of an agent doing a task.** This is where agent + task + runtime-session converge.

- Schema: `public.agentrun` (line 36) — has `mesh_agent_id`, `mesh_task_id`, `runtime_kind`, `runtime_session_id`, `status`, `resume_token`, `parent_run_id`, `total_cost_cents`, `idempotency_key`
- This is what you query to answer "what session did agent X use last time it touched kluster Y?" — join `agentrun → meshtask → kluster`.

**Resume-key for AI session continuity:** `(agent_id, kluster_id) → claude_session_id` is derived via `agentrun.mesh_task_id → meshtask.kluster_id` and `agentrun.runtime_session_id → aisession`. No new table needed.

---

## MissionRoleMembership

**Who has what role on a mission.** Admin / Contributor / Viewer (philosophy line 244–248).

- Schema: `public.missionrolemembership` (line 659) — has `mission_id`, `subject`, `role`
- Used by governance enforcement at every mutation point.

---

## What this doc is not

- Not an API reference (see `docs/catalog/api.yaml`)
- Not a deployment guide (see `docs/runbooks/`)
- Not a design proposal (see `docs/design/`)
- Not exhaustive — covers the load-bearing entities AI agents reason about. Add others here as they become load-bearing.

## Update protocol

Before editing this file:
1. Open the schema migration and confirm columns match what you're about to write.
2. Open `MISSIONCONTROL_PHILOSOPHY.md` and confirm the definition matches.
3. If schema and philosophy disagree, **stop** — that's a real architectural drift, not a doc edit.
4. Update this file, then update any contradicting doc.
