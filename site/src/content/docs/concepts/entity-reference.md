---
title: Entity Reference
description: Canonical definitions for every load-bearing EdgePlane entity. This is the single source of truth.
---

This is the single source of truth for what each EdgePlane entity means. If anything in another doc, code comment, or AI response contradicts this page, this page wins (its source is `docs/architecture/entities.md` in the repo). Update the source doc first, then propagate.

## Domain

**A bounded organizational objective.** The high-level "what we are doing and why." Carries the northstar narrative, owners, governance scope.

- Columns: `northstar`, `owners`, `contributors`, `visibility`, `status`
- Owns: many missions (`mission.domain_id`)

**Domains do not complete. They scope. Tasks complete.**

:::note[Domain.kind is deprecated]
The `kind` column (values `'work'` | `'home'`) was a write-only tag that leaked a deployment-specific operational pattern into the schema. New code must not write or filter on `kind`. The column remains for now (migrations are forward-only). Operational coordination domains are regular domains; `edgeplaned` bootstraps a default domain named `home` (overridable via `EP_HOME_DOMAIN_NAME` env).
:::

---

## Mission

**A knowledge cluster inside a domain for a targeted outcome. This is the workstream.** Missions are where artifacts cohere and where context continuity lives.

- Columns: `brief_md`, `brief_version`, `brief_created_by`, `brief_modified_by`, `brief_created_at`, `brief_modified_at`, `brief_s3_path`, `domain_id` (nullable), `owners`, `status`
- Legacy compat: `workstream_md`, `workstream_version` (kept for backward compatibility; `brief_md` is the canonical field)
- S3 layout: `domains/{domain_id}/missions/{mission_id}/{entity}/{filename}`
- Owns: tasks (both `kind`s), artifacts

A mission *is* a workstream. The mission Brief (`brief_md`) documents its targeted outcome. Do not call domains workstreams.

---

## Task

**A unit of work inside a mission.** One primitive serving both human-tracked and agent-dispatched work, discriminated by `kind`. Completes.

- Table: `public.task` (was `meshtask`; absorbed the legacy integer-PK `task` table and took its name in migration `0014_unify_task_meshtask.sql`)
- Columns: `id` (string, the SSOT identity), `public_id`, `mission_id` (FK, required), `domain_id` (denormalized, required — reached via `mission.domain_id` for `kind='assigned'` rows too), `parent_task_id` (self-FK, parent/child fan-out), `kind` (`'assigned'` | `'claimable'`), `status`, `owner`, `contributors`, `done_criteria`, `dependencies_note`/`related_artifacts_note` (display-only, migrated from the legacy `task` table's free-text columns), `claim_policy`/`required_capabilities`/`claimed_by_agent_id`/`claim_lease_id`/`lease_expires_at` (claimable-only, nullable), `attempt`/`max_attempts`/`attempted_by`/`errors` (bounded retry), `depends_on`/`produces`/`consumes`, `result_artifact_id`, `finalized_at`
- Result is recorded as an artifact (`result_artifact_id`)
- Links to artifacts via `meshtaskartifact` (input/output role; table name unchanged by the rename)

The `kind` discriminator resolves what earlier revisions of this page called "an open architecture question": whether `Task` and `MeshTask` would converge. They did, 2026-07, via migration `0014_unify_task_meshtask.sql`:

- **`kind='assigned'`** (was the standalone `task` table) — explicit owner set at creation, manual completion, no lease. Human-facing: the web dashboard, CLI `task create/list/show/update/delete`.
- **`kind='claimable'`** (was `MeshTask`) — no owner until claimed; capability-routed pool; atomic claim with a lease; self-healing on lease expiry, bounded by `attempt`/`max_attempts`. Agent-dispatch: CLI `task mesh <verb>`, the MCP mesh tools, the daemon's poll/claim loop.

`kind` encodes routing (push vs. pull), not actor (human vs. agent) — an agent can be directly `assigned` a task exactly like a human can, and a human can complete a `claimable` task's result. Completion is one unified path for both kinds: a `claim_lease_id` completion token, minted at claim time for `claimable` rows and on first status transition for `assigned` rows, validated by `authz_task_owner` (lease-token match, or `claimed_by_agent_id`/`owner` match).

See [MeshTask System](/concepts/mesh-tasks/) for the `kind='claimable'` claim-execute-complete lifecycle in detail.

---

## Artifact

**A persisted output bound to a mission.** Documents, binaries, and agent results — anything S3-stored.

- Columns: `mission_id` (required), `uri`, `storage_backend`, `content_sha256`, `version`, `provenance`, `status`
- Storage path: `domains/{domain_id}/missions/{mission_id}/{entity}/{filename}`
- Artifacts are leaves — linked from tasks via `meshtaskartifact`

---

## Agent

**An identity that performs work.** Human or AI. Carries capabilities, status, metadata, and a domain anchor.

- Base columns: `name`, `capabilities`, `status`, `metadata`
- Added later: `archived_at`, `display_name`, `node_id`, `last_seen_at` — lifecycle and presence metadata
- `public_id`: `{name}-{8-char-suffix}` — stable, human-readable identifier used by `/agents/{public_id}/messages` and the unified `edgeplane agent` surface. Immutable after creation.
- `home_domain_id`: permanent anchor — set once at registration, never cleared
- `current_domain_id`: active attachment — follows the agent's working context, resets to home on detach

There is no separate `agent_identity` table. An `Agent` is the canonical identity row.

See `MeshAgent` for the discoverable, runtime-bound projection.

---

## AgentSession

**A live agent process attached to the control plane.** Created when `edgeplane run` starts an agent; destroyed on clean exit or timeout.

- Columns: `agent_id` (FK), `claude_session_id`, `context`, `started_at`, `ended_at`, `end_reason`, `audit_log`, `last_seen_at`
- Does not carry `mission_id` or `domain_id` — work-binding is via `AgentRun`

`runtime_kind` values:

| Value | Meaning |
|-------|---------|
| `claude_agent_acp` | ACP persistent — Claude Code with EdgePlane injected as an MCP server |
| `zellij_hosted` | PTY bridge — agent lives in a Zellij pane; EdgePlane bridges via PTY |
| `codex` | Driver subprocess |
| `gemini` | Driver subprocess |
| `openclaw` | Driver subprocess |
| `custom` | Driver subprocess |

Session status: `active` | `idle` | `terminated`

Distinct from `AiSession` — an `AgentSession` is the live process record; an `AiSession` is the conversation record in the web AI Console.

---

## AiSession

**A tracked conversation in the web AI Console.** Created when an operator opens a conversation in the dashboard's AI Console tab.

- Columns: `id`, `owner_subject`, `title`, `status`, `runtime_kind`, `runtime_session_id`, `workspace_path`, `policy_json`, `capability_snapshot_json`, `agent_id`, `turns`
- `capability_snapshot_json` freezes the capability set at session start for consistent policy enforcement
- Each turn carries `role`, `content`, and events (tool calls, progress frames)

Distinct from `AgentSession` — an `AiSession` is the conversation record; `AgentSession` is the live process.

---

## MeshAgent

**The runtime-bound, discoverable projection of an agent into the mesh.** This is the row the controlplane scheduler matches against when claiming a `kind='claimable'` task. Distinct from `agent` (the canonical identity row).

- Columns: `domain_id`, `node_id`, `runtime_kind`, `runtime_version`, `capabilities`, `labels`, `status`, `current_task_id`, `enrolled_by_subject`, `enrolled_at`, `last_heartbeat_at`, `runtime_node_id`, `profile_json`, `machine_json`, `runtime_json`, `supervision_mode`
- `discovered_capabilities`: runtime-introspected capability set, unioned with declared `capabilities` during scheduling
- `agent_public_id` (nullable FK): links the mesh projection back to the canonical agent identity

Why two tables: `agent` is identity (who); `meshagent` is presence and capability (where + what they can do right now). One agent identity can have multiple meshagent rows over time as it enrolls on different nodes.

**Ephemeral usage pattern:** the same `Agent` identity can hold a persistent fleet-operator MeshAgent AND N transient task-subagent MeshAgents simultaneously. Each spawned subagent gets its own MeshAgent row (`labels.role=task-subagent, ephemeral=true`), claims one task, runs, and is deleted on completion. Audit trail survives via `agentrun.mesh_agent_id` FK (`ON DELETE SET NULL`).

---

## ExecutionSession

**A leased compute slot for running an agent.** Pure infrastructure: lease, runtime class, PTY-or-not, attach token prefix.

- Columns: `lease_id` (FK), `runtime_class`, `pty_requested`, `attach_token_prefix`, `status`
- Not for resume-key lookups. Compute-tier only.

Distinct from `AgentSession` (live process record) and `AiSession` (conversation record). See those sections above for their definitions.

---

## AgentRun

**A single execution of an agent doing a task.** This is where agent + task + runtime-session converge.

- Columns: `mesh_agent_id`, `mesh_task_id`, `runtime_kind`, `runtime_session_id`, `status`, `resume_token`, `parent_run_id`, `total_cost_cents`, `idempotency_key`
- To answer "what session did agent X use last time it touched mission Y?" — join `agentrun → task → mission` (the `mesh_task_id` column name is unchanged by the `meshtask` → `task` rename — only the table it targets was renamed)

---

## What this reference covers

This reference covers load-bearing entities — the ones agents and operators reason about. It is not an API reference (see `docs/catalog/api.yaml` in the repo), not a deployment guide, and not exhaustive. Add entities here as they become load-bearing.
