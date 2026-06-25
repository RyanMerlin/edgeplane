# Codex Multi-Session Swarm Workflow (No Nested `codex exec`)

This is the canonical Codex-native swarm workflow for Edgeplane.

Goal: run multiple Codex sessions collaborating on the same domain/mission without using nested `codex exec` pressure workers.

## Why this path

- Avoids nested MCP handshake contention from `codex exec` inside `codex exec`.
- Uses the same production control plane (`edgeplane daemon` shim + Edgeplane API).
- Produces run artifacts for replay/debug (`artifacts/collab/<run_id>/summary.json`).

## Prerequisites

- Edgeplane full stack healthy (`postgres`, `api`, `mcpd`, `mosquitto`, `rustfs`).
- Local shim reachable at `127.0.0.1:8765`.
- `EP_AGENT_TOKEN` valid for your API (or a valid OIDC session from `edgeplane auth login`).

## 1) Start a collaboration run (driver session)

Run in one terminal:

```bash
EP_BASE_URL=http://localhost:8008 \
EP_AGENT_TOKEN="<ep_sa_token>" \
EP_STACK_PROFILE=full \
EP_COLLAB_DURATION_SEC=600 \
EP_COLLAB_POLL_SEC=5 \
bash scripts/edgeplane-collab-driver.sh
```

Driver behavior:

- creates domain + mission + 3 scenario tasks (unless you provide existing IDs)
- prints `domain_id` and `mission_id`
- samples task state during the run window
- emits `summary.json` at completion

Attach to an existing domain/mission instead:

```bash
EP_BASE_URL=http://localhost:8008 \
EP_AGENT_TOKEN="<ep_sa_token>" \
EP_STACK_PROFILE=full \
EP_COLLAB_DOMAIN_ID="<domain_id>" \
EP_COLLAB_MISSION_ID="<mission_id>" \
bash scripts/edgeplane-collab-driver.sh
```

## 2) Join with additional Codex sessions

Open 2-5 additional Codex sessions.

Each session should:

- use Edgeplane MCP shim (`EP_MCP_MODE=shim`, daemon `127.0.0.1:8765`)
- target the same `domain_id`/`mission_id`
- pick one task and move it through: `proposed -> in_progress -> blocked|done`
- include concise updates in task descriptions

Recommended role split:

- Session A: Runtime/debug (MCP handshake, daemon path)
- Session B: Harness instrumentation (reporting, categorization)
- Session C: Ops docs/runbook updates

## 3) Collaboration protocol

- Claim: update task status to `in_progress` + ownership note **before doing any work**.
- Work: append concrete findings/changes (not generic notes).
- Handoff: if blocked, set `blocked` + explicit unblock requirement.
- Complete: set `done` + short outcome summary.

Keep writes intentional; do not spam status flaps.

### Task coordination: avoid double-claiming

There is currently **no server-side atomic claim** on tasks. Two agents can both call `update_task` on the same task ID without conflict — the last write wins. This is a known gap (see [Known coordination gap](#known-coordination-gap) below).

Safe claim pattern until a `claim_task` tool exists:

1. Call `list_tasks` and filter for `status == "proposed"`.
2. Pick the task you intend to claim and **immediately** call `update_task` setting `status: in_progress` and `owner: <agent_id>`.
3. Re-read the task and verify `owner` matches your agent ID before proceeding. If another agent claimed it first, pick a different task.

For scripted/parallel launches, assign task IDs explicitly per session using `EP_COLLAB_TASK_ID` or equivalent env vars rather than relying on agents to self-coordinate from `list_tasks`.

## Known coordination gap

**Problem:** `update_task` is protected by domain-level write authz but has no per-task ownership lock or optimistic concurrency check. Multiple agents targeting the same mission can silently overwrite each other's status transitions.

**Observed in swarm run `20260320050257`:** sessions B and C both claimed task `58a0b952a9b9` — B moved it to `done`, then C overwrote the description (still `done`, so outcome was harmless but work was duplicated).

**Implications for real workloads:**

| Scenario | Risk | Severity |
|----------|------|----------|
| Two agents claim same task | Duplicate work, one agent's description overwritten | Medium |
| Agent A sets `done`, agent B sets `in_progress` (race) | Task appears regressed; ledger shows false state transition | High |
| Agent writes stale `owner` field after another agent claimed | Ownership tracking corrupted | Medium |
| High-frequency concurrent writes to same task | Last-write-wins; intermediate states lost from ledger | High |

**What's needed (tracked under EP-MCP-007 idempotency work):**

- `claim_task` MCP tool: atomic `proposed → in_progress` transition that fails if the task is already owned — backend enforces with a DB-level check on `(status == proposed OR owner == requester)`.
- Optimistic locking on `update_task`: accept an optional `expected_status` or `version` field; return `error_code: conflict` if current state doesn't match.
- Owner guard: `update_task` should reject status transitions from agents that don't own the task (unless platform admin or mission owner).

Until these are in place, coordinate task assignment out-of-band (pre-assigned IDs per session) for any workload where correctness matters.

## 4) Acceptance criteria

A run is considered healthy when:

- at least 3 tasks exist in the target mission
- each task has at least one meaningful state transition
- at least one task reaches `done`
- driver report shows non-zero `observed_changes`

Check:

```bash
cat artifacts/collab/<run_id>/summary.json
```

Key fields:

- `observed_changes`
- `status_counts`
- `tasks[]` (`status`, `updated_at`)

## 5) Troubleshooting

- If `mcpd` fails to start on `:8765`, stop any local `edgeplane daemon` already bound there.
- If Codex reports MCP startup incomplete, verify `EP_*` env names.
- If stack services are stale, run `bash scripts/dev-up.sh` (defaults to full profile and clears preexisting/orphan containers first).

## 6) Recommended pressure sequence

1. `playbook` gate (`scripts/edgeplane-pressure-test.sh`) as deterministic baseline.
2. Codex multi-session swarm run (this doc) for real collaboration behavior.
3. Only after both are stable, revisit nested `agent` pressure mode.
