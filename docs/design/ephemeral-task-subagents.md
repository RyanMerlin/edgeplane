# Ephemeral Task Subagents — Identity Model

**Status:** Shipped 2026-05-21 (mcd 0.15.4–0.15.7). All four phases complete.
**Author:** Aria (mc-engineer) with Merlin
**Supersedes:** signal-injection pattern for delegated work
**Implementation:** `crates/mcd/crates/mcd/src/{bootstrap,task_worker,capabilities}.rs`

---

## Problem

Today, when a scheduled job (e.g. `fleet-self-health`) surfaces something actionable, the dispatcher has two bad options:

1. **Signal-inject** into a live profile session via `mc agent signal`. The signal lands as a user message in whatever conversation is active, interrupting focused work and polluting context.
2. **Write a note to the vault** and hope the operator notices on next check-in. Loses urgency, breaks the loop, no feedback.

What we want: the dispatcher submits a task to MC, an ephemeral subagent claims it, runs it to completion in an isolated context, and reports back. The persistent profile session is never touched. MC retains full visibility.

The architectural question this doc answers: **what is the identity model for that ephemeral subagent?**

---

## Answer (per entities.md)

**The ephemeral subagent is not a new `Agent`.** It is an `AgentRun` of an existing `Agent` (the parent profile), surfaced into the mesh as a transient `MeshAgent`, executing in an `ExecutionSession`, holding an `AISession`.

Concretely, for one task lifecycle:

| Entity | Lifetime | Identity |
|--------|----------|----------|
| `Agent` | Permanent | Parent profile (e.g. `aria-mc-engineer`, `agent.public_id = "aria-mc-engineer-a1b2c3d4"`). **Reused, never created per task.** |
| `MeshTask` | Task lifecycle | Created by dispatcher (e.g. health check) via `submit_mesh_task` with `required_capabilities` and target `kluster_id` |
| `MeshAgent` | Task lifecycle | **New per subagent.** `agent_public_id` FK back to parent. `current_task_id` set, `last_heartbeat_at` ticked while running. Archived/superseded on completion. |
| `ExecutionSession` | Task lifecycle | Compute lease for the subagent process. Released on exit. |
| `AISession` | Task lifecycle | A fresh claude session (`runtime_kind = "claude_agent_acp"` or `"claude_headless"`). No resume by default — ephemerality is the point. |
| `AgentRun` | **Durable** | Joins `mesh_agent_id` + `mesh_task_id` + `runtime_session_id` + `parent_run_id`. **This is the audit record. It persists after the task completes.** |

**Why this works:**

- `entities.md` line 96: *"One agent identity can have multiple meshagent rows over time as it enrolls on different nodes."* This is exactly that pattern: one `Agent` identity, N concurrent `MeshAgent` projections (one persistent operator + M ephemeral task subagents).
- `entities.md` line 131: *"This is what you query to answer 'what session did agent X use last time it touched kluster Y?'"* `AgentRun` is the durable trace. Even after the subagent's `MeshAgent` and `AISession` are gone, the `AgentRun` row stays, with `total_cost_cents`, `idempotency_key`, `resume_token`, and `parent_run_id` intact.
- `meshagent.current_task_id` is singular (per schema) — that's why we need a *new* `MeshAgent` per concurrent subagent, not multiple claims on one row.

---

## Visibility — what the user sees

Per entity model, MC retains full visibility despite ephemerality:

- **`mc agent list`** — shows the parent identity plus its active subagent projections. UI can collapse them under the parent (e.g. *"aria-mc-engineer (3 in flight)"*).
- **`list_mesh_tasks`** — shows the work in progress with `claimed_by_agent_id` pointing at the ephemeral `MeshAgent`.
- **`get_entity_history`** on the parent `Agent` — joins through `AgentRun` to show every subagent execution ever, including the ephemeral ones long after their `MeshAgent` is gone.
- **Cost rollup** — `agentrun.total_cost_cents` summed per parent agent, per kluster, per mission — gives clean accountability.

The ephemeral nature is in the runtime projection, not the audit trail.

---

## Lifecycle (one task)

```
1. Dispatcher (e.g. cron job, signal handler, another agent)
   └─> submit_mesh_task(kluster_id, prompt, required_capabilities, target_agent_hint)
       returns: meshtask.id

2. Spawner worker (mcd's `task-worker` module — new component)
   └─> Polls list_mesh_tasks for open tasks matching its supervised profiles
   └─> For each match:
       a. enroll_mesh_agent(
            agent_public_id = parent.public_id,
            mission_id = parent.current_mission_id,
            node_id = local node,
            runtime_kind = "claude_headless" | "claude_agent_acp",
            capabilities = parent.capabilities,
            supervision_mode = "ephemeral",
            labels = {"role": "task-subagent", "parent_run_id": <optional>}
          )
          → ephemeral MeshAgent row
       b. claim_mesh_task(mesh_task_id, mesh_agent_id) → lease
       c. (Skipped — ExecutionSession entity is designed for attachable PTY
          sessions with attach_token/lease. Headless `claude -p` subprocesses
          don't need it; the AgentRun audit row + the OS process are sufficient.)
       d. Start AISession (fresh claude session) with cwd = profile dir,
          extra context = task prompt, allowed tools = parent's capability set
       e. Create AgentRun(mesh_agent_id, mesh_task_id, runtime_session_id,
                          parent_run_id = NULL, status = "running")
       f. Spawn `claude -p "<prompt>"` (or ACP equivalent) in profile cwd

3. Subagent process runs
   └─> Periodic heartbeat → meshagent.last_heartbeat_at
   └─> Work product written as artifacts (S3 via kluster path)
   └─> On done: emits result_artifact_uri + status

4. Spawner worker observes exit
   └─> complete_mesh_task(mesh_task_id, result_artifact_id, status)
   └─> Update AgentRun: status="completed", total_cost_cents=<measured>
   └─> End AISession
   └─> Release ExecutionSession
   └─> Update MeshAgent: status="archived", current_task_id=NULL
       (or DELETE — open question, see below)

5. AgentRun row persists. MeshAgent row archived or deleted.
```

---

## Concurrency + collision handling

- **Multiple subagents per parent**: trivially supported — N `MeshAgent` rows, one parent `Agent`. `current_task_id` is per-MeshAgent.
- **Concurrent git operations on a shared repo**: real hazard. Two `claude -p` subagents both running `git add` in `~/code/aria` can corrupt the index. **Mitigation: per-task git worktrees** — spawner allocates `~/.mc/worktrees/<task-id>/` before launch. Subagent's cwd is the worktree, not the live checkout.
- **API rate limits**: enforced at the spawner via `max_concurrent_subagents` config (default 3). Tasks pile up in the mesh queue, claimed FIFO as slots free.
- **Filesystem appends (`.learnings/`, etc.)**: append-only writes from concurrent processes are mostly safe (POSIX guarantee for ≤PIPE_BUF). Vault writes go through `aria vault note write` which is atomic at the git layer.

---

## Open questions (decide before implementation)

1. **Archive or delete MeshAgent on completion? — RESOLVED: delete.**
   - Schema audit (2026-05-21) confirmed `agentrun.mesh_agent_id` FK is `ON DELETE SET NULL` (migrations/0001:1576). AgentRun row survives MeshAgent deletion — audit trail intact.
   - Also: MeshAgent `status` is validated against whitelist `["online", "busy", "idle", "offline", "errored"]` in `work.rs:1791`. "archived" is not accepted; adding it would require a code change with no benefit since delete handles the lifecycle.
   - **Decision: delete on completion.** No GC needed, no row bloat, no whitelist change, no migration.

2. **Resume-token usage for subagents?**
   - Default: no resume. Fresh AISession per task. Maximizes isolation.
   - Optional: spawner can request resume for "continuation" tasks (e.g. multi-step refactors). `agentrun.parent_run_id` links the chain.
   - **Recommend: opt-in via `MeshTask.metadata.continuation_of = <prior_agentrun_id>`. Off by default.**

3. **Capability set for the subagent — inherit or restrict?**
   - Inherit: subagent gets every tool the parent has. Simple. Risks broad blast radius.
   - Restrict: subagent gets only `required_capabilities` from the MeshTask. Safer. Requires the dispatcher to declare needed capabilities upfront.
   - **Recommend: restrict.** Dispatcher must declare `required_capabilities` on every MeshTask. Spawner enforces by filtering parent's capability set down to the declared subset before launching.

4. **Where does the spawner live?**
   - Option A: new `mcd::task_worker` module — keeps everything in the daemon already running.
   - Option B: separate daemon (`mc-task-worker.service`) — clean separation, independent restart.
   - **Recommend: start in mcd as a module** (already has agent context, registry access, lifecycle hooks). Extract to a separate daemon only if it grows beyond ~500 lines or needs independent scaling.

---

## What this proposal does *not* cover

- The autonomous *claimer policy* — which subagents claim which tasks. Out of scope here; that's a scheduler concern (see future `docs/design/task-scheduling.md`).
- The migration path from current signal-injection patterns (e.g. `fleet-self-health`'s current goose dispatch). Per-skill migration is a follow-up.
- UI/TUI surfacing of ephemeral subagents. Once the data model is in place, UI follows.

---

## Schema audit results (2026-05-21)

**Verdict: schema fully supports the model. Zero migrations required.**

| Check | Result |
|-------|--------|
| `meshagent` columns sufficient | ✓ `status`, `current_task_id`, `labels`, `agent_public_id`, `last_heartbeat_at`, `supervision_mode`, `capabilities` all present |
| `agentrun` columns sufficient | ✓ `mesh_agent_id`, `mesh_task_id`, `runtime_session_id`, `parent_run_id`, `resume_token`, `total_cost_cents`, `idempotency_key`, `metadata_json` all present |
| `meshtask` columns sufficient | ✓ `required_capabilities`, `claim_policy`, `parent_task_id`, `claimed_by_agent_id`, `result_artifact_id`, `lease_expires_at` all present |
| `agentrun.mesh_agent_id` FK behavior | ✓ `ON DELETE SET NULL` — AgentRun survives MeshAgent deletion (audit trail preserved) |
| `agentrun.mesh_task_id` FK behavior | ✓ `ON DELETE SET NULL` — same |
| `agentrun.parent_run_id` FK behavior | ✓ `ON DELETE SET NULL` — resume chains break gracefully |

**Code-surface gaps that don't block (but worth noting):**

1. `enroll_mesh_agent` HTTP handler (`work.rs:1678`) hardcodes `supervision_mode = NULL` and `status = 'online'` on insert. To tag a subagent as ephemeral, **use the `labels` JSON field** (e.g. `{"role": "task-subagent", "ephemeral": true}`). Zero code change. `supervision_mode` enum can be extended later if we want it first-class.
2. `enroll_mesh_agent` MCP tool (`mcp.rs:112`) accepts only `mission_id`, `agent_id`, `capabilities_json`, `runtime_kind`, `agent_name` — no `labels`. The mcd spawner runs on the same host as the controlplane with admin auth, so **it should call the HTTP API directly**. MCP tool extension is a follow-up if cross-host MCP-driven spawning becomes a use case.
3. `submit_mesh_task` MCP tool (`mcp.rs:100`) doesn't accept `required_capabilities` or `claim_policy` even though the columns exist. Same workaround: spawner uses HTTP API. MCP extension is a separate ticket.

**Bottom line:** the entity model is sound and the database is ready. The only build work is the mcd spawner + the 3 small MCP tool extensions (which are independent and can ship later).

---

## Prototype findings (2026-05-21)

Walked the full lifecycle end-to-end via `scripts/proto/ephemeral-subagent.sh` against the live controlplane. Two findings worth landing before `mcd::task_worker`:

**1. API field-name inconsistency in `/runs` (small, fix anytime).**
`models::run::StartRunRequest` accepts `agent_id` and `task_id`, but the columns it writes to are `agentrun.mesh_agent_id` and `agentrun.mesh_task_id`. Callers who follow the column naming silently get NULL FKs because serde drops unknown fields. Either rename the request fields to `mesh_agent_id`/`mesh_task_id` (better, but breaks API back-compat) or alias both via serde (zero-breakage). Recommend the alias.

**2. No admin DELETE endpoint for meshagent (blocker for the spawner).**
The only path that issues `DELETE FROM meshagent` is `revoke_node_agent` at `DELETE /runtime/nodes/{node_id}/agents/{agent_id}`. It requires the agent to be assigned to a registered runtimenode AND the caller to be the node's owner. The ephemeral subagent model doesn't fit this constraint — subagents won't always be registered against a runtimenode, and the spawner's cleanup path needs a generic delete.

**Fix:** add `DELETE /work/agents/{agent_id}` to `crates/mc-controlplane/src/routes/work.rs`, requiring admin or `meshagent.enrolled_by_subject == principal.subject`. Reuses the existing `DELETE FROM meshagent WHERE id=$1` SQL. Estimated 30 lines. **This must land before `mcd::task_worker` can clean up after itself.**

Aside from these two, the schema-level FK behavior (`ON DELETE SET NULL` on `agentrun.mesh_agent_id`) is enforced by Postgres and does not require runtime testing — declaring the constraint in migrations/0001 is sufficient proof. Prototype confirmed lifecycle steps 1–9 (mission → kluster → meshtask → enroll meshagent → claim → start AgentRun with proper FKs → spawn `claude -p` → complete) work end-to-end against the live controlplane.

---

## Recommendation

**Accept this identity model and proceed to:**

1. ~~Schema audit~~ — done. Zero migrations needed.
2. ~~Build a 50-line shell prototype that walks through the lifecycle once end-to-end~~ — done; `scripts/proto/ephemeral-subagent.sh` validated the model.
3. ~~If prototype works: scope the `mcd::task_worker` module.~~ — phased into P1-P4 below.
4. Wire `fleet-self-health` as the first production caller.
5. ~~Follow-up: extend MCP tools~~ — walked back. CLI is the right surface for these; see decision log.

---

## Decision log (post-review 2026-05-21)

After walking through the open questions, the following were locked:

| # | Decision | Why |
|---|----------|-----|
| 1 | **Delete MeshAgent on completion.** Not archive. | FK `ON DELETE SET NULL` on `agentrun.mesh_agent_id` preserves audit trail automatically. Status whitelist doesn't include "archived" anyway. |
| 2 | **No resume tokens in v1.** Fresh AISession per task. | Subagents chain via artifacts (S3-stored) + `parent_run_id` audit, not session state. YAGNI on cross-session resume. |
| 3 | **Restrict capabilities** via dispatcher-declared `required_capabilities`, enforced via `claude -p --allowed-tools`. | Forces dispatchers to declare blast radius. Limits damage from buggy automation. |
| 4 | **`mcd::task_worker` module**, per-node sharding by supervised profiles. | mcd is the supervisor; spawning is supervision. Per-node naturally shards without a central coordinator. |
| 5 | **One fleet-ops mission + one `intake` kluster**, spawner-as-triage. | Walked back the per-node `home-{hostname}` model — `Mission.kind` was a write-only column with zero readers (leaked Aria-specific operational pattern into MC's schema). Replaced with a single mission (`aria-fleet-ops` by default, overridable via `MC_OPS_MISSION_NAME`) holding one `intake` kluster. **Triage** in spawner: rule (target_profile set → claim) → goose categorization at confidence >0.85 → vault surface to `mc-engineer/inbox.md` for low-confidence cases. Non-interruptive throughout. |
| S2 | **Routing creates child meshtasks**, not kluster_id rebinds. | Intake task stays in intake kluster as routing log (status=`dispatched`); child meshtask under the routed kluster carries the work. Uses existing `parent_task_id` schema, no new mutations. |
| MCP | **Don't extend the MCP tool surface** for write operations. | Every MCP tool definition costs context tokens in every session forever. CLI (`mc daemon task submit` etc.) is the same effort with zero context tax. The spawner calls HTTP directly anyway. |

**Soft-deprecated:** `Mission.kind` column. New code MUST NOT write or filter on it. Existing `provision_home_for_node` in `routes/runtime.rs` still writes `kind='home'` but is dormant (no runtime nodes registered) — flagged as cleanup follow-up.

---

## Implementation phasing

| Phase | Scope | Status |
|-------|-------|--------|
| **P1: Bootstrap** | mcd auto-provisions `aria-fleet-ops` mission + `intake` kluster on startup. Idempotent, soft-fail. | ✓ done — `crates/mcd/crates/mcd/src/bootstrap.rs` |
| **P2: Claimer loop** | `mcd::task_worker` polls open meshtasks, claims via lease, spawns `claude -p` in a per-task worktree, completes the task. Handles tasks with explicit `target_profile` only. | Next |
| **P3: Triage logic** | Three-tier triage (rule → goose → vault surface). Introduces parent/child task pattern for intake routing. | After P2 |
| **P4: Capability enforcement** | `required_capabilities` → `--allowed-tools` translation. Coarse vocabulary: `shell:read/write`, `fs:read/write`, `vault:read/write`, `mc:read/write`, `web:fetch`, `gh:read/write`. | After P2 |
