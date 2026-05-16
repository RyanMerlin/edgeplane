# mcd: Controlplane-Driven Enrollment & State/Config Separation

**Date:** 2026-05-10
**Status:** Design — pending sign-off
**Author:** mc-engineer (Aria) + Merlin
**Supersedes (in part):** Sections of `docs/plans/mcd-persistent-session-architecture.md` describing yaml-based agent enrollment

---

## Why this exists

While wiring up the ACP-native persistent supervisor (Layer 3 of the persistent-session work), we hardened in a yaml shape that duplicates state already owned by `mc-controlplane`. Reading `MISSIONCONTROL_PHILOSOPHY.md` after the fact made the gap clear:

> Coordination truth stays in MissionControl (Postgres). Git is a projection sink, never the authority for mission ownership, approvals, or governance.

The same rule applies to mcd.yaml: **it must not be the authority for mission/agent assignment**. Today it is. Below is what's broken, what the target looks like, and a phased path to get there without throwing away Layers 1–3.

---

## What's broken today

Concrete evidence in the current `DaemonConfig` (`integrations/mcd/crates/mcd/src/config.rs`):

```yaml
backend_url: ...
token: ...
node_id: <user pastes UUID after register>     # state, not config
attach_secret: <user pastes hex secret>        # state, not config
attach_bind_addr: 0.0.0.0:8009                 # genuine local infra ✓
missions:                                      # SHOULD live in controlplane
  - mission_id: aria-core
    agents:
      - agent_id: aria-work
        runtime_kind: claude_agent_acp
        session_mode: persistent
        capabilities: [code.read, code.edit]   # SHOULD be discovered/enrolled
        profile_path: /home/merlin/.../work
```

Concrete failures this produces, mapped to Merlin's questions:

| Question | Today | Why it's wrong |
|---|---|---|
| "Hardcoding missions in mesh?" | Yes — `MissionEntry` is in the yaml | `meshagent.mission_id` already exists in Postgres |
| "What is mcd doing?" | Acting as a partial agent registry | Per docs: it's a **per-node arm**, like kubelet — claims work, doesn't define work |
| "Reassign aria-work to a new mission?" | Edit yaml on every node, restart daemon | Should be `mc daemon agent reassign` → controlplane UPDATE → daemon rebalances live |
| "Hardcoded capabilities?" | Yes — `AgentEntry.capabilities: Vec<String>` | Three real sources: runtime built-ins, controlplane meshagent, ACP `InitializeResponse` |
| "Specify node_id?" | Yes (after registering) | Should be in a daemon-managed state file, never user-edited |
| "Specify attach_secret?" | Yes (the user has to capture & paste) | Same — minted at register, written to state file |

Layers 1–3 are not undermined by this. The `mcd-acp` client, the `claude_agent_acp` runtime, and `acp_session_supervisor` are correct building blocks. What's wrong is **what feeds them**: yaml-defined enrollment.

---

## Target model

### Where each thing lives

| Concern | Source of truth | Cached locally? |
|---|---|---|
| node identity (`node_id`) | controlplane `runtimenode` | yes — state file (read-only to user) |
| `attach_secret` | controlplane (minted at register) | yes — state file, mode 0600 |
| `backend_url` + `token` | `~/.missioncontrol/session.json` (mc auth) | already managed by `mc auth` |
| missions | controlplane `mission` table | no — pulled per-need |
| agent assignments | controlplane `meshagent` rows where `runtime_node_id = self` | no |
| agent `mission_id` | meshagent | no |
| agent `runtime_kind` | meshagent | no |
| agent `supervision_mode` (task / persistent) | meshagent (column already exists) | no |
| agent `capabilities` | meshagent (user-set) ⊕ runtime built-ins ⊕ ACP-discovered | live, synced upstream on connect |
| agent `profile_json` (CLAUDE.md / launch context) | meshagent | no |
| `attach_bind_addr` | local infra (yaml) | yaml — this is genuine config |

### New `mcd.yaml` shape

Five lines, max:

```yaml
# ~/.mc/mcd.yaml  (or /etc/mcd/agent.yaml)

# Optional — most fields inherit from `mc auth`
backend_url: http://missioncontrol:8008    # optional override

# Local infra — genuinely a per-node setting
attach_bind_addr: 0.0.0.0:8009             # default

# Operational policy — local
offline_grace_secs: 30
offline_policy: safe_readonly
```

`node_id`, `attach_secret`, `missions`, `agents`, `capabilities` — all gone from yaml.

### State file: `~/.mc/mcd.state.json` (0600, daemon-managed)

```json
{
  "schema_version": 1,
  "node_id": "uuid-from-register",
  "attach_secret": "hex-from-register",
  "registered_at": "2026-05-10T15:30:00Z",
  "controlplane_url": "http://missioncontrol:8008"
}
```

**Written by:**
- `mc daemon node register --bootstrap-token <jt_…>` (explicit user action), OR
- the daemon's first-run path if it sees a `MCD_BOOTSTRAP_TOKEN` env var

**Read by:** the daemon at every startup. If missing → daemon refuses to start with a clear "run `mc daemon node register` first" message.

**Never edited by humans.**

### Daemon startup flow

```
1. Load yaml (5 lines)
2. Resolve auth (mc auth → backend_url + token)
3. Read state file → node_id, attach_secret
   └─ if missing/incomplete: error out unless MCD_BOOTSTRAP_TOKEN is set,
      in which case auto-register and write state
4. POST /runtime/nodes/{node_id}/heartbeat (existing)
5. GET /runtime/nodes/{node_id}/agents → [MeshAgent records]
6. For each: spawn appropriate runtime + supervisor (existing Layer 1-3 path)
7. Subscribe to controlplane WS for `agent.assignment_changed` events
   └─ on event: add/remove/reassign supervisors live, no daemon restart
```

### Capability model (the answer to question #4)

Capabilities surface in three layers, unioned at task-claim time:

1. **Runtime built-ins** — hardcoded in the runtime impl. Already works:
   `ClaudeAgentAcpRuntime::new()` advertises `code.read`, `code.edit`, `code.plan`, `test.run`, `claude_agent_acp`, `acp`.
2. **MeshAgent record** — `meshagent.capabilities` column (already exists, settable via API). User adjusts via `mc daemon agent capabilities <id> --add foo --remove bar`.
3. **Discovered** (ACP-only initially) — on connect, the agent's `InitializeResponse.agentCapabilities` is sent to the controlplane and stored in a new `meshagent.discovered_capabilities` JSON column. Last-seen wins. Surfaces things like `promptCapabilities.image`, `mcpCapabilities.http`, `sessionCapabilities.{fork,list,resume}` — info the user couldn't realistically maintain by hand.

The capability dispatcher (`mcd-core::capability_dispatcher`) unions all three when matching against `task.required_capabilities`.

### Mission reassignment flow

```
admin: mc daemon agent reassign aria-work --mission new-research
  →  controlplane: UPDATE meshagent SET mission_id='new-research' WHERE id='aria-work'
  →  controlplane WS: emit { type:"agent.assignment_changed",
                              agent_id, old_mission, new_mission, runtime_node_id }
  →  mcd on the host running aria-work:
        receive WS event
        gracefully shut down existing supervisor (calls AcpSession::shutdown for ACP)
        re-fetch agent record (now scoped to new mission)
        spawn new supervisor under new scope
  →  live, no daemon restart, no yaml edit
```

For ACP persistent sessions specifically: shutdown → spawn = the agent loses in-memory session state but the long-term context (CLAUDE.md, profile_path, vault) is intact. If `loadSession` on the new mission scope is desired later, ACP `sessionCapabilities.resume` already supports it.

---

## Implementation phases

### Phase 4a — Controlplane: GET-agents-for-node + assignment events

- [ ] `GET /runtime/nodes/{node_id}/agents` — returns meshagent rows where `runtime_node_id = node_id`. Owner-scoped via `Principal`.
- [ ] Verify `row_to_agent` includes `runtime_node_id`, `supervision_mode`, `profile_json` (currently omits the first two — small fix).
- [ ] Add `agent.assignment_changed` to the controlplane WS event stream. Emitted on enroll/reassign/unenroll where `runtime_node_id` is set.
- [ ] Migration if needed: `discovered_capabilities` JSON column on `meshagent` (Phase 4e uses this).
- [ ] Tests: integration test listing agents for a registered node, integration test for the WS event.

**No mesh changes in this phase. Code merges independently.**

### Phase 4b — mcd: state file + drop node_id/attach_secret from yaml

- [ ] New `state.rs` module: read / write `~/.mc/mcd.state.json` with 0600 perms, schema versioning, atomic rename.
- [ ] `mc daemon node register --bootstrap-token <jt_…>` → POSTs to `/runtime/nodes/register`, captures the response, writes state file.
- [ ] Daemon startup: read state file, error with clear message if missing.
- [ ] Backwards compat: if old yaml has `node_id` / `attach_secret`, migrate to state file on first daemon start, then warn-and-strip from yaml on next save.
- [ ] Drop both fields from `DaemonConfig` after the migration window (one release).

### Phase 4c — mcd: pull agent assignments from controlplane

- [ ] Daemon startup calls `GET /runtime/nodes/{node_id}/agents` after auth.
- [ ] Translate the response into the existing in-process structure that drives Layer 1-3 supervisors.
- [ ] Drop `missions: Vec<MissionEntry>` from `DaemonConfig`. Warn loudly and ignore if present (1-release deprecation).
- [ ] `AgentEntry` and `MissionEntry` types removed from `config.rs`. Their replacement is the `MeshAgentRecord` returned by the controlplane (already exists in `mcd-core::types`).

### Phase 4d — Live reassignment

- [ ] Daemon subscribes to controlplane WS, filters for `agent.assignment_changed` where `runtime_node_id == self`.
- [ ] On `removed` (this node no longer hosts the agent): graceful shutdown of supervisor, unregister from attach registry.
- [ ] On `added` (this node now hosts a new agent): spawn supervisor as in Phase 4c.
- [ ] On `reassigned` (mission_id changed): shutdown old, spawn new under new mission scope.
- [ ] `mc daemon agent reassign <id> --mission <new>` CLI surface (controlplane endpoint already supports the underlying mutation).
- [ ] Tests: e2e — start daemon with two agents, reassign one to a different mission, observe the supervisor cycle without daemon restart.

### Phase 4e — Capability discovery (ACP-specific perk)

- [ ] `mcd-acp::Agent` exposes `InitializeResponse.agentCapabilities` (already captured internally, just needs an accessor).
- [ ] ACP supervisor pushes the discovered capability set up via `POST /work/agents/{id}/discovered_capabilities` after initialize.
- [ ] Controlplane stores in `meshagent.discovered_capabilities`.
- [ ] Capability dispatcher unions runtime-builtin ⊕ meshagent.capabilities ⊕ meshagent.discovered_capabilities.
- [ ] Non-ACP runtimes: no-op. They keep relying on (1) + (2) only.

### Phase 4f — Drop `capabilities: Vec<String>` from yaml

- [ ] After 4c lands, the field has no input path that's not already covered by the meshagent record. Remove from `AgentEntry` (which itself is being removed in 4c).

---

## Open questions for sign-off

1. **Bootstrap token UX.** `MCD_BOOTSTRAP_TOKEN` env var on first daemon start, OR explicit `mc daemon node register --bootstrap-token <jt_…>` only? I lean explicit — env-var auto-register is too magic. Confirm?
2. **Poll vs WS for assignment changes.** WS exists already. Poll-only fallback when offline? (Current `offline_grace_secs` policy already covers reads-only-if-stale.)
3. **discovered_capabilities namespacing.** Should ACP's `agentCapabilities.promptCapabilities.image` flatten to `prompt.image` or stay nested as JSON? The capability dispatcher today matches strings; flattening is simpler.
4. **Version/release handling for legacy yamls.** One release with deprecation warnings, then hard-fail on `missions:` / `node_id:` in yaml? Or hard-fail immediately and require a one-time migration command?
5. **What about `attach_bind_addr`?** This stays in yaml today (genuine local infra). Confirm — or should it also live in state, configured via `mc daemon node register --attach-bind 0.0.0.0:8009`?

---

## What's NOT in scope

- Web attach UI for ACP sessions (became "Layer 5")
- Swapping `attach_ws.rs` from binary PTY frames to text JSON-RPC (Layer 5)
- `mc inject <agent-id>` CLI command (depends on Layer 5)
- Replacing the existing systemd profile services (`aria-*.service`) — that's a downstream consequence once Phase 4 is fleet-deployed and validated, not part of this plan

---

## Done = ?

- A user can run `mc daemon node register --bootstrap-token <jt_…>` once on a node.
- They never see `node_id` or `attach_secret` in yaml.
- They run `mc daemon agent enroll --node <node-id> --mission <mission-id> --runtime claude_agent_acp --supervision persistent --profile-path /…` to schedule an agent.
- They run `mc daemon agent reassign <agent-id> --mission <new>` and the daemon rebalances live, no restart.
- `mc daemon.yaml` on a fresh node is 5 lines.
- The Aria fleet (5 profiles, 1 host today) runs as a single `mcd.service` per host with all assignment owned by the controlplane.

---

## Sign-off needed

Reviewer should confirm:

1. The yaml-as-config / DB-as-state boundary as drawn above.
2. The state file path/perms/schema (or propose alternatives).
3. The capability model (3-source union).
4. The WS event shape for reassignment.
5. Phase ordering — specifically that 4a can land independently and the rest can be sequenced.

Once signed off: pick up with Phase 4a (controlplane endpoint) and proceed in order.
