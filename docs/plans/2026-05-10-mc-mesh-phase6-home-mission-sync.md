# mc-mesh Phase 6: Home Mission, Sync Loop, Goose-as-Router

**Date:** 2026-05-10
**Status:** Shipped (2026-05-10) — see commits `bde0cf4` (step 1), `113d48b` (step 2), `edc0169` (step 6), `7d1813f` (step 5). Step 4 was already in via Phase 4d/5d.
**Builds on:** Phase 5 (standalone-first + controlplane profiles, local SQLite registry)
**Prereqs landed:** broadcast delivery fix + task-mode pending-message buffer (commit `dc76952`)

---

## Framing

Phase 5 made mc-mesh standalone-first with a local SQLite registry, and split `mc-mesh` (kubelet) from `mc` (kubectl). What's missing:

1. **Bootstrap.** A freshly-registered node has no agents. The user has to hand-configure yaml or run `mc mesh agent enroll` manually.
2. **Dynamic discovery.** Once federated, the daemon never re-polls the controlplane for new agent assignments — it loads from yaml/SQLite once at startup.
3. **Per-node addressability.** There's no surface for the controlplane (or peer agents) to say "send this to node X." Tasks live in missions, not on nodes.
4. **Cheap routing.** Every task triage today wakes a Claude session. There's no low-cost decision layer.

Phase 6 fills all four with one shape: **every node owns a home mission, hosting a persistent Goose agent that routes work into domain missions.**

---

## The home mission

When a node registers (or, in standalone mode, on first `mc mesh agent enroll-home`), the system provisions:

- A mission named **`home-{tailscale_hostname}`** (e.g. `home-excalibur`)
  - Slug-stable, human-readable, unique across a Tailnet
  - Description: "Node-level coordination inbox for {hostname}"
- A default agent enrolled in that mission:
  - `runtime_kind: goose` (persistent, local Qwen-27B via LiteLLM)
  - `session_mode: persistent` (so it stays attached and gets live messages)
  - `capabilities: ["routing", "triage", "dispatch", "overlap_check"]`

The home mission is **not** a general work queue. It's a coordination surface. Useful things to put there:

- Routing decisions: "this task came in unsorted — which domain mission?"
- Triage tasks: "summarize / categorize this incoming artifact"
- Node-level housekeeping: "check disk, rotate logs, verify mounts"
- Overlap pre-checks before creating tasks in domain missions
- Status aggregation: "report current state of all agents on this node"

Heavy work (`infra`, `backend`, `research`, etc.) lives in its own domain mission — the home-agent's job is to *dispatch* there, not do it.

### Why Goose for the home agent

- **Local inference (Qwen-27B via LiteLLM)** — zero API cost; runs on commodity hardware
- **Persistent (ACP/PTY)** — gets live message delivery, can hold conversational context
- **Fast turnaround** — no cloud round-trips for routing decisions
- **Acceptable quality** — Qwen-27B is plenty for triage, "what mission does this belong to", "is this a duplicate", "format this into a task spec"

Claude/GPT-class models stay in domain missions where their cost is justified by output quality.

---

## Multi-agent / multi-mission model on one daemon

mc-mesh already supports running N task_loops concurrently — Phase 5 just didn't make use of it dynamically. Phase 6 codifies the shape:

```
node: excalibur
└── mc-mesh daemon
    ├── agent: home-excalibur-goose   → mission: home-excalibur   (persistent, always on)
    ├── agent: infra-cc-1             → mission: infra            (task-mode, when assigned)
    ├── agent: backend-cc-1           → mission: backend          (task-mode, when assigned)
    └── agent: research-cc-1          → mission: research         (task-mode, when assigned)
```

Each agent:
- Is mission-scoped (sees only that mission's klusters, roster, policy)
- Has its own task_loop + message relay
- Can collaborate with peers **within its mission** via the message bus (now actually working post-`dc76952`)
- Can post messages to **other missions' agents** via the cross-mission send endpoint (`POST /work/missions/{mid}/messages` with `to_agent_id`)

The daemon hosts them all. The home-Goose agent is the only "always on" one — others spin up when the controlplane assigns the node to a mission, and tear down when the assignment is revoked.

---

## The sync loop

Replace yaml-driven static mission config with periodic discovery:

```
On daemon startup (federated mode):
  1. Read state file → get node_id + active profile
  2. GET /runtime/nodes/{node_id}/agents → list of agent enrollments
  3. LocalRegistry::replace_source(cp_source, specs)  [already exists, Phase 5d]
  4. For each enrolled agent: spawn task_loop or session_supervisor

Every N minutes (also on WS push, see below):
  1. Re-poll GET /runtime/nodes/{node_id}/agents
  2. Diff against current set of running task_loops
  3. NEW assignments → spawn new loop
  4. REMOVED assignments → graceful shutdown of loop (drain in-flight task, then exit)
  5. CHANGED capabilities → update LocalRegistry, restart loop if config materially differs
```

Standalone mode skips steps 1-3 (no controlplane to query) and uses LocalRegistry directly. The yaml path remains as a local-only override for pre-Phase-6 setups.

### Push notification for assignments

Add a new WS message type alongside existing `task_available`:

```json
{ "type": "agent_assignment_changed", "node_id": "..." }
```

The notify WS connection is per-agent today; we'll either:
- (a) Open a separate per-node notify WS (`/runtime/nodes/{node_id}/notify`), or
- (b) Broadcast assignment-changed events to every existing agent WS on the node

Option (a) is cleaner — the home-Goose agent's WS becomes the natural carrier since it's always connected. Falls back to periodic poll if WS is down.

---

## Standalone vs federated paths

| Concern | Standalone | Federated |
|---|---|---|
| Home mission auto-provision | `mc mesh agent enroll-home` (manual, one-time) | `mc mesh profile add` triggers backend `provision_home_for_node` (automatic) |
| Mission/agent CRUD | Local-only: `mc mesh mission create / agent enroll` writes to SQLite via `mgmt_gateway` | Controlplane API; daemon syncs back to SQLite |
| Source of truth | SQLite (`SOURCE_LOCAL`) | Controlplane → sync → SQLite (`source_cp(profile)`) |
| Sync loop | n/a — local writes go through `mgmt_gateway` and reconcile immediately | Periodic poll + WS push |
| Cross-node collaboration | Not available — single daemon | Mission roster + messages span the fleet |
| Goose-as-router | Works — routes within local missions only | Works — routes across the fleet's missions |

The home mission concept itself is daemon-local in both modes; federation just adds discovery.

---

## API additions (backend)

```
POST /runtime/nodes/register            (existing — extend to provision home)
GET  /runtime/nodes/{node_id}/agents    (new — list enrollments)
POST /runtime/nodes/{node_id}/agents    (new — controlplane assigns agent to node)
DELETE /runtime/nodes/{node_id}/agents/{agent_id}  (new — revoke assignment)
```

`register` extension: if the request includes `tailscale_hostname`, the route additionally calls a new `provision_home_for_node(node_id, hostname, runtime_kind=goose)` helper that:

1. `INSERT INTO mission (id, name, description, owner_subject, ...)` for `home-{hostname}` (idempotent on conflict)
2. `INSERT INTO meshagent (mission_id, runtime_kind=goose, supervision_mode=persistent, capabilities=[routing,…], node_id=...)`
3. Returns the new agent's UUID in the response

Existing `register` callers that don't supply `tailscale_hostname` retain current behavior — provisioning is purely additive.

### The home-mission's mission record

Schema-wise, `home-{hostname}` is a normal `mission` row. To keep these grouped in the UI, add an optional `mission.kind` field:

```sql
ALTER TABLE mission ADD COLUMN IF NOT EXISTS kind varchar NOT NULL DEFAULT 'work';
```

Values: `work` (the default, existing missions), `home` (node-level inbox). The UI can filter the "home" missions into a separate section.

---

## mc-mesh daemon changes

1. **`merge_state_file`**: no change — the active profile's `node_id` is already pulled.
2. **New `sync_loop.rs`**:
   - Owns the lifecycle of task_loops in federated mode
   - On startup: full pull + spawn
   - Loop: poll every 5 min (or on WS push) + reconcile
   - Uses `LocalRegistry::replace_source` for atomic swap (Phase 5d already built this)
3. **`daemon.rs::run`**: when federated, hand mission/agent dispatch off to `sync_loop`. When standalone, continue reading from LocalRegistry as today.
4. **WS notify**: extend `run_notify_ws` to also handle `agent_assignment_changed` and wake the sync loop.

---

## mc CLI changes

```
mc mesh profile add ... --no-home          # opt out of home-mission auto-provision
mc mesh agent enroll-home                  # standalone-mode: create home mission locally
mc mesh use <profile>                      # already exists — re-sync after switch
mc mesh status                             # extend to show: home mission, active assignments, sync-loop state
```

`mc mesh status` is also where we fix the existing "daemon: stopped" detection bug — it should check the mgmt socket (`~/.mc/mc-mesh-mgmt.sock`) and return its agent/mission view, not just look at a PID file.

---

## Migration from yaml

For nodes that currently boot from `~/.mc/mc-mesh.yaml`:

1. yaml continues to work for at least one release — it loads into LocalRegistry under `SOURCE_LOCAL`
2. When a profile is added with `mc mesh profile add`, the sync loop activates and pulls authoritative state from the controlplane
3. yaml is treated as additive but lower-precedence than controlplane-sourced agents
4. New deprecation warning: "mc-mesh.yaml is legacy. Run `mc mesh profile add` to federate, or `mc mesh agent enroll-home` to use standalone discovery."

No forced migration. Existing yaml users keep working.

---

## Out of scope for Phase 6 (deferred)

- **In-task polling** (`mc mesh messages` from inside an agent workdir). Phase 5d's pending-message buffer covers the bulk of the coordination need; in-task polling waits for a concrete mid-task-pivot use case.
- **Standalone cross-node messaging.** Requires a local message bus + peer discovery (mDNS or static peer list). Federated mode covers the fleet use case for now.
- **Multi-controlplane federation.** Active profile is still one-at-a-time. Future work.
- **Goose MCP tooling for `mc.*`** so the home-Goose can call `create_task` / `detect_overlaps` itself. Hooking Goose to MCP is a separate work item — for v0 the home agent's actions go through `mc` CLI invocations.

---

## Order of implementation — as built

1. ✅ **Backend: `mission.kind` column + `provision_home_for_node` helper** — `bde0cf4`. Migration `0006_mission_kind.sql`, `slug_hostname` helper with 8 unit tests, register_node calls provisioning inline (collapsed step 3 into this).
2. ✅ **Backend: `POST` + `DELETE /runtime/nodes/{node_id}/agents`** — `113d48b`. GET was already in from Phase 4a; my POST adds `agent.assigned` broadcast, DELETE adds `agent.revoked`. Existing GET enhanced to join on mission for `mission_name`/`mission_kind`.
3. ✅ Folded into (1) — `register_node` now invokes `provision_home_for_node` directly.
4. ✅ **mc-mesh sync_loop** — *already shipped* in Phase 4d/5d. `resolve_agent_specs` does the initial controlplane pull, `Spawner::apply_plan` handles spawn/restart/shutdown, `watch_assignments_ws` + `poll_assignments` keep state converged. No new code needed; verified end-to-end wiring.
5. ✅ **mc CLI: `enroll-home` + extended `status`** — `7d1813f`. `mc mesh agent enroll-home` for standalone-mode home-mission setup. Status now shows mode/profile/agents and uses mgmt-socket probe so it correctly reports `running` for the systemd-managed daemon.
6. ✅ **mc CLI: `profile add` Tailscale auto-detection** — `edc0169`. Reads `tailscale status --json` when `--tailscale-fqdn` not given, surfaces home-mission info from the register response.
7. ✅ **Deprecation warning + plan-doc update** — this commit. yaml_specs now emits a tracing::warn when used, pointing users at `profile add` (federated) or `agent enroll-home` (standalone). MC-MESH.md is stale and should be rewritten separately.

A clean `mc mesh profile add ...` on a fresh node now yields a fully-bootstrapped node with a home-Goose agent ready to receive routing tasks.

---

## Open questions

1. **Home-mission ownership.** Who owns the `mission` row — the registering subject, a synthetic `node:{node_id}` principal, or shared/system-owned? Affects who can post tasks into it. Default: registering subject for now; revisit when multi-user shows up.
2. **Goose binary distribution.** mc-mesh-runtimes already has a `goose` runtime. Confirm the binary is installable on a fresh node (or have `ensure_installed` handle it) so home-mission Goose provisioning doesn't fail on bare metal.
3. **Sync-loop poll interval.** 5 min is a guess. WS push covers the urgent path; the poll is for safety. Could be longer (15 min?) if WS push is reliable.
4. **What happens when Tailscale hostname changes?** Rename the home mission? Create a new one and leave the old? For now: leave-as-is, document the edge case. Most operators won't change hostnames after node registration.
