# mcd: Standalone-First with Controlplane Profiles

**Date:** 2026-05-10
**Status:** Design — pending sign-off
**Builds on:** Phase 4a–d (controlplane-driven enrollment).
**Supersedes (in part):** the "Phase 4-cache" sketch from
`2026-05-10-mcd-controlplane-driven-enrollment.md`. A real local
source-of-truth is better than a stale-tolerant cache.

---

## Framing

mcd is an **agent runtime**, full stop. It always works locally. It
*optionally* federates with one controlplane at a time. The user can
save N controlplane profiles (work / personal / homelab / …) and select
which one is active. Inactive profiles are stored credentials, nothing
more.

This is consistent with `MISSIONCONTROL_PHILOSOPHY.md` — "coordination
truth stays in MissionControl" — because in standalone mode, **mcd
itself plays the MissionControl role for its local scope.** Truth lives
in local SQLite instead of controlplane Postgres. When a profile is
active, the controlplane becomes the upstream source of truth and SQLite
becomes a synced read-through.

States:

| State | Description |
|---|---|
| **standalone** | No active profile. SQLite is source of truth. `mc daemon agent enroll` writes locally. |
| **federated to `<profile>`** | One controlplane active. SQLite is a cache; controlplane Postgres is source of truth. Mutations write upstream first, sync back. |

There is **no third "multi-controlplane" mode**. Profiles are saved
configurations; only one is selected at a time.

---

## Architecture

```
┌─────────────── mcd daemon (always runs) ───────────────┐
│                                                            │
│  Always-local components:                                  │
│    supervisor / runtimes / attach gateway / secrets broker │
│    capability dispatcher / message bus (same-host)         │
│                                                            │
│  ── Local SQLite (~/.mc/mcd.db) ──                     │
│    agent registry (mirror of meshagent shape)              │
│    discovered capabilities                                 │
│    profile membership ("local" vs "controlplane:work")     │
│                                                            │
│  ── Active profile sync (optional) ──                      │
│    one controlplane → reconciler:                          │
│      WS subscriber + 60s poll (Phase 4d, unchanged)        │
│      writes into SQLite                                    │
│    mutations from CLI:                                     │
│      standalone → SQLite directly                          │
│      federated  → controlplane API → reconciler picks up   │
│                                                            │
└────────────────────────────────────────────────────────────┘
```

---

## Data model

### `~/.mc/mcd.db` (SQLite, mode 0600)

A small mirror of the controlplane's `meshagent` table, plus a
`source` column to namespace by profile.

```sql
CREATE TABLE agent (
    id              TEXT PRIMARY KEY,            -- agent UUID
    source          TEXT NOT NULL,               -- 'local' | 'controlplane:<profile>'
    mission_id      TEXT NOT NULL,
    runtime_kind    TEXT NOT NULL,
    supervision_mode TEXT NOT NULL,              -- 'task' | 'persistent'
    capabilities_json TEXT NOT NULL DEFAULT '[]',
    discovered_capabilities_json TEXT NOT NULL DEFAULT '[]',
    profile_json    TEXT,                        -- profile_path lives in here
    enrolled_at     TEXT NOT NULL,
    last_synced_at  TEXT,                        -- NULL for local agents
    UNIQUE (source, id)
);

CREATE INDEX agent_by_source ON agent (source);

CREATE TABLE schema_version (
    version INTEGER PRIMARY KEY
);
```

Standalone mode reads `WHERE source = 'local'`. Federated mode reads
`WHERE source = 'controlplane:<active-profile>'`. The local-only agents
are still in the DB when a profile is active — they just don't get
spawned. Switching back to standalone resumes them.

### `~/.mc/mcd.state.json` (extends Phase 4b)

```json
{
  "schema_version": 2,
  "active_profile": "work",
  "profiles": {
    "work": {
      "url": "https://mc.work.example.com",
      "auth": { "kind": "token", "token": "mcs_..." },
      "node_id": "uuid-issued-by-this-controlplane",
      "attach_secret": "hex",
      "registered_at": "2026-05-10T15:30:00Z",
      "tailscale_fqdn": "epyc.tail.ts.net"
    },
    "personal": { ... },
    "homelab": { ... }
  }
}
```

`active_profile: null` (or omitted) = standalone. Each profile has its
own node_id + attach_secret because each controlplane issues its own.
Switching profiles switches identity.

The schema bumps from v1 (Phase 4b) to v2. The migration is mechanical:
v1's `{node_id, attach_secret}` becomes
`profiles.{user-named}: {node_id, attach_secret, url, auth, …}` and
`active_profile = "<that-name>"`. The user is prompted (or defaulted) to
name the migrated profile.

---

## CLI surface

**Hard rule:** `mcd` (the daemon binary) is kubelet — it has no
user-facing subcommands. All operator surfaces go through `mc daemon …`
(the kubectl-equivalent). The `mcd` binary is left with:

- `mcd run` — daemon entry, called by systemd
- `mcd get-secret` — internal, called by spawned agent subprocesses
  via the secrets socket
- `mcd version`

The user-facing surfaces below all live in `integrations/mc/src/mesh.rs`
(where the existing `MeshCommand` enum already gathers `mc daemon up`,
`mc daemon agent`, `mc daemon task`, etc.). Phase 5c extends that enum.

```
mc daemon profile add <name> --url <…> --bootstrap-token <jt_…>
    Calls /runtime/nodes/register against the URL, stores the result
    plus URL+auth as a saved profile. Does NOT activate it.
    Replaces the temporary `mcd node-register` subcommand from
    Phase 4b — that one moves into `mc daemon profile add`'s
    implementation and the mcd binary loses the subcommand.

mc daemon profile list
    Lists saved profiles + which is active. Reads state.json.

mc daemon profile remove <name>
    Drops the profile entry. If active, prints "restart daemon to
    take effect" and clears active_profile.

mc daemon profile rename <old> <new>
    Cosmetic rename. Useful for the v1→v2 migration default ("default"
    → something meaningful).

mc daemon use <name>           # activate saved profile
mc daemon use --standalone     # deactivate (no controlplane)
    Updates state.json.active_profile. V1 prints "restart daemon to
    take effect." V2 (deferred) signals the running daemon to switch
    live via the mgmt socket.

mc daemon agent enroll \
        --mission <id> \
        --runtime <kind> \
        --supervision <task|persistent> \
        [--profile-path <path>]
    Standalone mode  → INSERT into local SQLite directly.
    Federated mode   → POST to active controlplane (existing path).
    In both modes: if the daemon is running locally, send a
    "reconcile-now" hint to its mgmt socket so it picks up the change
    immediately instead of waiting for the 60s poll (best-effort —
    silently skipped if the daemon isn't running).

mc daemon agent reassign <id> --mission <new>     # Phase 4d-b — same logic
mc daemon agent unenroll <id>                     # Phase 4d-b — same logic
mc daemon agent ls                                # query SQLite (both modes)
```

These already exist in skeleton form (`MeshAgentCommand::{Ls, Enroll}`)
in `integrations/mc/src/mesh.rs:88`. Phase 5c fills in `Reassign`,
`Unenroll`, and the `Profile` subcommand tree, and rewires `Enroll` to
branch on standalone vs. federated.

### How `mc` reaches the local SQLite + daemon

Two interfaces. `mc` chooses based on what it's doing:

- **Direct file I/O** (when daemon may not be running): reads/writes
  `~/.mc/mcd.state.json`, `~/.mc/mcd.db`. Used by:
  `profile add/list/remove/rename`, `use`, and the standalone-mode
  path of `agent enroll/reassign/unenroll/ls`. SQLite is opened in
  WAL mode so it's safe even when the daemon is also reading.

- **mcd mgmt socket** (when daemon is running, for hints):
  `mc daemon agent enroll` etc. send a "reconcile" message to
  `~/.mc/mcd-mgmt.sock` so the daemon re-runs its reconciler
  immediately. Best-effort; the 60s poll catches anything missed.

The federated-mode mutations (`agent enroll`, `agent reassign`, etc.)
hit the controlplane HTTP API directly from `mc` — no daemon round-trip
required, since the controlplane's WS push (Phase 4d) wakes the daemon
through the same path.

---

## Daemon startup flow (replaces Phase 4 startup)

```
1. Load mcd.yaml (small)
2. Resolve auth (mc auth → backend_url + token IF the active profile
   doesn't override)
3. Load state.json
   ├─ no profiles, no active   → standalone
   ├─ active = "<name>"        → federated to that profile's controlplane
   └─ profiles only, no active → standalone (loaded but not used)
4. Open SQLite, run migrations
5. If federated:
   - POST heartbeat (existing)
   - GET /runtime/nodes/<profile.node_id>/agents → upsert into SQLite
     with source = "controlplane:<profile_name>"
   - Subscribe to WS notify (Phase 4d unchanged)
6. Build initial agent_specs from SQLite WHERE source = (current source)
7. Run reconciler (Phase 4d) — spawn supervisors per spec
8. If federated, the WS + poll loops (Phase 4d) keep SQLite fresh and
   re-trigger reconciles on diff.
```

Standalone is the same flow with steps 5 + 8 elided. The reconciler
doesn't care where the specs came from — only that they're in SQLite
under the active source.

---

## Mutation flow

| Op | Standalone | Federated |
|---|---|---|
| `agent enroll` | INSERT into SQLite (source='local'); reconciler picks up on next tick | POST to controlplane; reconciler sees the agent.assigned WS event; SQLite gets the upsert; spawn |
| `agent reassign` | UPDATE SQLite | POST to controlplane; WS event → SQLite update → reconciler restarts |
| `agent unenroll` | DELETE from SQLite | DELETE on controlplane; WS event → SQLite delete → reconciler shuts down |
| `agent capabilities` | UPDATE SQLite | PATCH on controlplane; sync down |

Both paths converge on "SQLite gets updated → reconciler reacts." The
controlplane path adds a network hop and authoritative conflict
resolution; the local path doesn't.

---

## Profile-switching semantics

### V1 (this plan): switch requires restart

```
$ mcd use personal
Active profile: personal (was: work)
Restart the daemon to apply: systemctl --user restart mcd
```

On daemon restart, it sees the new active_profile, registers under that
profile's identity (already done at `mcd profile add` time), pulls
that profile's agent list, spawns. Old profile's agents are still in
SQLite under `source='controlplane:work'` — they're just not spawned.

### V2 (deferred): live switch

Daemon listens on its mgmt socket for a "switch profile" command:
graceful-shutdown all current supervisors, swap active source, run
reconciler against the new source. ~30s of agent downtime. Doable but
not urgent.

---

## TUI surface

The mc TUI gains a Profiles tab:

```
┌─ Profiles ──────────────────────────────────────────────────┐
│                                                             │
│  ★ work       https://mc.work.example.com   12 agents       │
│    personal   https://mc.merlin.dev          3 agents       │
│    homelab    http://missioncontrol:8008     5 agents       │
│    [standalone]                              2 agents (local)│
│                                                             │
│  Press <Enter> to switch. Active profile shown with ★.      │
│  Press 'a' to add, 'r' to remove. Restart required (V1).    │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

Agent counts come from local SQLite (the cache or local). Switch from
the TUI = same as `mcd use <name>`.

---

## Implementation phases

### 5a — Local SQLite registry; standalone mode functional
- [ ] `mcd-local-registry` module: open SQLite, run migrations,
      CRUD on `agent` table
- [ ] Daemon reads from SQLite when no active profile
- [ ] `mcd agent enroll/reassign/unenroll/list` write to SQLite in
      standalone mode
- [ ] Reconciler diff source becomes "agents in SQLite WHERE source = …"
- [ ] Tests: SQLite round-trip, migration, reconciler on local source

### 5b — Profile-aware state.json (schema v2)
- [ ] State.json gains `profiles` map + `active_profile`
- [ ] Migration v1 → v2: existing `{node_id, attach_secret}` becomes a
      profile named `default`. Daemon writes once + warns operator.
- [ ] Daemon resolves active profile on startup; if standalone, skips
      controlplane sync; if federated, uses that profile's identity
- [ ] Tests: migration paths, missing-active-profile, missing-named-profile

### 5c — Profile management CLI (in `mc`, not `mcd`)
- [ ] Extend `integrations/mc/src/mesh.rs::MeshCommand` with:
      `Profile(MeshProfileCommand)` and `Use(MeshUseArgs)`.
- [ ] `MeshProfileCommand::{Add, List, Remove, Rename}` implementations
      that read/write `~/.mc/mcd.state.json` directly.
- [ ] `MeshUseArgs` writes `active_profile`. Prints daemon-restart hint.
- [ ] Wire `mc daemon agent {enroll,reassign,unenroll,ls}` to branch on
      "active profile present?" — standalone path writes SQLite,
      federated path POSTs to controlplane.
- [ ] **Remove** the `node-register` subcommand from the `mcd`
      binary added in Phase 4b. Its body moves into `mc daemon profile add`'s
      handler in `integrations/mc/src/`. Update `Phase 4b`'s state-file
      module so the mc binary can call it (move it from
      `crates/mcd/src/state.rs` to a shared crate, e.g.
      `mcd-core::state` or a new `mcd-state` crate, so both `mc`
      and `mcd` link against it).
- [ ] Best-effort mgmt-socket "reconcile" hint after local SQLite
      mutations (silently no-ops if daemon isn't running).
- [ ] Tests: profile lifecycle, standalone-vs-federated dispatch in
      `mc daemon agent enroll`.

### 5d — Reconciler reads SQLite, controlplane sync upserts
- [ ] In federated mode, the WS + poll loops (Phase 4d) write to SQLite
      instead of feeding the reconciler directly
- [ ] Reconciler input becomes "diff of SQLite WHERE source = current vs
      running map"
- [ ] Both standalone and federated paths now share the same
      reconciler-input-source
- [ ] Tests: federated sync upserts SQLite correctly; reconciler
      reacts to SQLite changes

### 5e — TUI surface
- [ ] mc TUI Profiles tab
- [ ] Switch / add / remove
- [ ] Per-profile agent count + last-sync timestamp

### 5f (deferred) — Live profile switching
- [ ] mgmt socket command: switch profile
- [ ] Graceful drain of current supervisors, re-spawn under new source

---

## How this builds on Phase 4

| Phase 4 piece | Phase 5 effect |
|---|---|
| **4a** controlplane GET /agents + WS | unchanged. Used only when a profile is active. |
| **4b** state file for node identity | becomes per-profile (schema v2). Migration from v1 is mechanical. The `mcd node-register` subcommand introduced there is **removed** in Phase 5c — it was the wrong split and never should have lived on the daemon binary. Its logic moves into `mc daemon profile add`. |
| **4c** pull agents from controlplane | retargets: now writes the result into SQLite instead of feeding the reconciler directly. Reconciler always reads SQLite. |
| **4d** WS + poll reconciler | unchanged in shape. Only the input source moves (SQLite, populated by sync). |
| **4d-b** controlplane reassign/unenroll endpoints | still needed — federated mutations go through them. |
| **4e** capability discovery | unchanged. Discovered capabilities upserted into SQLite + (federated) the controlplane. |
| **4f** drop yaml fields | finalized. yaml is back to 5 lines. SQLite + state.json carry everything else. |

No prior phase is undone. The shape we built is the federated-mode
path; Phase 5 lifts SQLite up as the universal substrate underneath.

---

## Open questions for sign-off

1. **Migration default for old `state.json` v1 → v2.** The v1 file holds a
   `{node_id, attach_secret}` from one controlplane. v2 needs that to
   become a named profile. Default the name to `default`, or prompt the
   operator? I lean **default to `default`** with a warning suggesting a
   rename via `mcd profile rename default <better>`.
2. **Standalone agent_id format.** Should locally-enrolled agents get a
   UUID (matches controlplane shape) or a friendly name? UUIDs are
   safer (no collisions if you ever switch to federated and re-enroll);
   friendly names are nicer in the TUI. I lean **UUID with an optional
   `display_name` column**.
3. **What happens to local agents when a profile is active?** Two
   options: (a) hidden — only the active profile's agents run; (b)
   merged — local + active profile both run.
   - (a) matches "one source at a time" cleanly.
   - (b) lets you keep local helpers running while syncing work agents.

   I lean **(a) for V1** — predictable, matches the "switch profiles"
   metaphor users have from VS Code, kubectl context, etc. (b) is
   addable as a per-profile flag later if anyone wants it.
4. **Scope of secrets across profiles.** The mcd-secrets broker
   today serves any agent that asks via the local socket. Should
   secrets be partitioned by profile? Locally-enrolled agents have
   access to local secrets only; federated agents have access to
   controlplane-pushed secrets only? V1 simple answer: **secrets are
   per-agent-record, regardless of source.** The broker doesn't need to
   know about profile membership; the credential resolution path
   already keys by agent identity.
5. **Multi-host federated mode** (e.g. two laptops both federated to the
   same work controlplane): unchanged from Phase 4 design. Each
   daemon registers independently; controlplane sees them as separate
   runtimenodes. Out of scope for this plan.

---

## What's NOT in scope

- Web UI for profile management (mc TUI is enough for V1)
- Cross-profile peer messaging (explicitly disallowed — sphere of trust)
- Live profile switching (V2; this plan ships V1 with restart-required)
- Cloning a controlplane's full state to local (not the use case)
- Sync conflict resolution beyond last-write-wins (the controlplane is
  authoritative when active, period)

---

## Sign-off: DONE (2026-05-10)

All 5 open questions resolved. Phase 5a shipped (commit aaf10d1).
5b → 5c → 5d in progress.

---

## Roadmap: Self-heal / doctor capabilities

Track separately. Before considering any phase "production-ready", verify:

- **`mc daemon health`** (exists, shallow) — extend to cover local registry
  integrity: can open SQLite, schema version matches, all enrolled agents
  still have valid runtime binaries on PATH.
- **`mc doctor`** — new top-level command (or `mc daemon doctor`). Checks:
  1. State file readable + schema_version understood
  2. Local registry accessible + not corrupt
  3. Active profile reachable (ping controlplane if federated)
  4. mcd daemon running + responding to mgmt socket
  5. All enrolled agents have their runtime binary available
  6. No orphaned PIDs / stale sockets in `~/.mc/`
- **Self-heal on startup**: daemon logs actionable errors for each check;
  `--repair` flag that can delete corrupt DB, reset stale sockets, etc.
- Tie to `mc daemon status` output so health problems surface without extra commands.

Assign to a phase after 5e (TUI). Implementation is straightforward once
the full profile + registry stack lands.
