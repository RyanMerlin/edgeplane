# ADR 0006: Minimal MCP Surface — Runtime Protocol + Discovery Meta-Tools

## Status
Accepted (2026-06-08)

## Context

ADR 0005 established that CLI is the primary operator interface and MCP is the agent runtime
protocol. At the time of writing, the tower MCP catalogue advertises **80 tools** — a surface
so large that every agent's context window is materially consumed by the tool list before any
work begins.

An audit of `crates/edgeplane-tower/src/routes/mcp.rs` revealed a structural problem: the MCP
tool implementations are **inline SQL forked from the REST routes** — there is no shared service
layer. This means a CLI command can simply hit the existing REST routes directly, and the
corresponding MCP arm can be **deleted** rather than re-plumbed. The duplication is a liability,
not a feature.

The goals of this ADR:

1. Define the authoritative keep / extract-first / move-and-delete table for all 80 tools.
2. Specify the two meta-tools (`discover`, `exec`) that give agents CLI access without a shell.
3. Target surface after reduction: **~16 tools** (14 runtime + `discover` + `exec`).

## Decision

### Principle

**The tower MCP surface is the agent runtime protocol.** It carries only operations an agent
calls *mid-execution* while holding an execution lease — the claim/heartbeat/progress/complete
loop, the workspace lease cycle, and the message bus. Everything else is CLI.

Two meta-tools bridge the gap for agents that need to reach the full CLI surface:

- `discover(path?, deep=false)` — walks the clap command tree (same `build_node` walker as
  `edgeplane discover` CLI), returns capability nodes. Lets agents discover available commands
  without a shell.
- `exec(args[])` — runs the local `edgeplane` binary as a subprocess, returns
  stdout/stderr/exit. Full CLI passthrough for agents in shell-less environments.

These live in the stdio gateway (`edgeplane serve`) — they run locally in the `edgeplane`
process and never round-trip to the tower.

### Keep set — 14 typed runtime tools + 3 borderline

Operations an agent calls mid-execution while holding a lease or working the mesh cycle:

| Tool | Reason to keep |
|------|---------------|
| `submit_mesh_task` | Creates work in-flight; called by agent, not operator |
| `list_mesh_tasks` | Scheduler loop — agent polls for claimable tasks |
| `claim_mesh_task` | Core lease acquisition |
| `heartbeat_mesh_task` | Lease renewal — sub-second, in flight |
| `progress_mesh_task` | Typed progress event emission |
| `complete_mesh_task` | Lease-bound task completion |
| `fail_mesh_task` | Lease-bound task failure |
| `block_mesh_task` | Lease-bound dependency block |
| `load_mission_workspace` | Workspace lease acquisition |
| `heartbeat_workspace_lease` | Workspace lease renewal |
| `commit_mission_workspace` | Workspace commit — requires active lease |
| `release_mission_workspace` | Workspace lease release |
| `send_mesh_message` | Agent-to-agent messaging |
| `list_mesh_messages` | Agent inbox poll |

Borderline — keep typed for now (re-evaluate at next audit):

| Tool | Notes |
|------|-------|
| `get_overlap_suggestions` | Called pre-task by agents to detect collisions — genuinely runtime |
| `fetch_workspace_artifact` | Requires active workspace lease; in-flight fetch |
| `get_mesh_task` | Agents use this to hydrate a task they just claimed |

### Extract-first set — 7 tools

These have non-trivial logic that lives only in the MCP arm (no REST equivalent). Build the
REST route + CLI command *before* deleting the arm.

| Tool | Why extract-first | Location in mcp.rs |
|------|------------------|-------------------|
| `get_artifact_download_url` | Inline SigV4 presign — no `/artifacts/{id}/download-url` REST route | ~2409 |
| `export_domain_pack` | In-memory tar.gz assembly — no REST equivalent | ~1840 |
| `install_domain_pack` | In-memory tar.gz unpack — no REST equivalent | ~2116 |
| `publish_pending_ledger_events` | Shells `git` — requires extraction + REST wrapper | ~2336 |
| `provision_domain_persistence` | Creates connection/binding/policy in one call — no REST | ~2120 |
| `resolve_publish_plan` | Resolves binding/repo/branch/path — no REST route | ~2267 |
| `get_publication_status` | Lists publication records — no REST route | ~2200 |

Do not delete these arms until the REST route + CLI command exists and is verified.

### Move-and-delete set — 56 tools

REST route already exists; CLI command (added in Phase 1) replaces the MCP arm.
Delete the dispatch arm in `mcp.rs` after verifying the CLI command works.

**Domain CRUD** (CLI: `edgeplane domain {create,list,update,delete}`):
`create_domain`, `list_domains`, `update_domain`, `delete_domain`

**Mission CRUD** (CLI: `edgeplane mission {create,list,update,delete}`):
`create_mission`, `search_missions`, `update_mission`, `delete_mission`

**Task CRUD** (CLI: `edgeplane task {create,list,update,delete,claim,release}`):
`create_task`, `list_tasks`, `update_task`, `delete_task`, `search_tasks`,
`claim_task`, `release_task`, `list_task_assignments`

**Doc CRUD** (CLI: `edgeplane doc {read,create,update}`):
`read_doc`, `create_doc`, `update_doc`

**Artifact CRUD** (CLI: `edgeplane artifact {create,update}`):
`create_artifact`, `update_artifact`

**Agent management** (CLI: `edgeplane agent {register,list,show,status,session}`):
`register_agent`, `list_agents`, `get_agent`, `update_agent_status`,
`start_agent_session`, `end_agent_session`

**Mesh management — admin ops, not runtime**
(CLI: `edgeplane mesh {unblock,cancel,retry,enroll,list-agents}`):
`unblock_mesh_task`, `cancel_mesh_task`, `retry_mesh_task`,
`enroll_mesh_agent`, `list_mesh_agents`

**Ledger** (CLI: `edgeplane history`, `edgeplane ledger`):
`list_pending_ledger_events`, `list_repo_bindings`, `get_entity_history`

**Profiles** (CLI: `edgeplane profile {list,get,publish,download,activate,delete,status,pin}`):
`list_profiles`, `get_profile`, `publish_profile`, `download_profile`,
`activate_profile`, `delete_profile`, `profile_status`, `pin_profile_version`

**Skill-sync** (CLI: `edgeplane skill-sync {resolve,download,status,ack,promote}`):
`resolve_skill_snapshot`, `download_skill_snapshot`, `get_skill_sync_status`,
`ack_skill_sync`, `promote_local_skill_overlay`

**Domain packs** (CLI: `edgeplane pack list`):
`list_domain_packs`

**Remote launch** (CLI: `edgeplane remote {register,list-targets,delete-target,launch,list-launches,status,kill}`):
`register_remote_target`, `list_remote_targets`, `delete_remote_target`,
`create_remote_launch`, `list_remote_launches`, `get_remote_launch`, `kill_remote_launch`

## Implementation Rule

1. **Never delete an MCP arm whose logic has no REST/CLI equivalent** — that is the
   extract-first set. Extract → verify → delete. In that order.
2. **Add a catalogue↔dispatch parity test** to `edgeplane-tower`: every tool advertised by
   `list_tools()` must have a dispatch arm; every dispatch arm must be advertised. This prevents
   silent drift (which is how the surface grew from ~40 to 80 tools unnoticed).
3. **The `exec` meta-tool is the migration safety net.** Any CLI path that replaces an MCP tool
   is immediately reachable by agents via `exec`. Hard-cutover is safe because of this.

## Target State After Phase 2

| Category | Count |
|----------|-------|
| Runtime tools (keep set) | 14 |
| Borderline keep | 3 |
| Meta-tools in gateway | 2 |
| **Total advertised** | **19** |

80 → 19. Every removed tool is reachable via `exec(["<noun>", "<verb>", ...])`.

## Consequences

- Agent context windows shrink by ~60 tools worth of schema definitions at session start
- The `exec` meta-tool gives agents the same capability surface as operators; no regression
- New CLI commands added in Phase 1 are immediately MCP-accessible via `exec` — no MCP arm
  required, ever again
- Extract-first tools (7) require REST routes before deletion; scope estimate: 2-4 days

## References

- ADR 0005: CLI-First — MCP Is the Agent Runtime Protocol
- ADR 0003: EdgePlane CLI Hierarchy Hard Cutover
- `crates/edgeplane-tower/src/routes/mcp.rs`: current catalogue + dispatch
- `crates/edgeplane/src/mcp_server.rs`: stdio gateway (discover + exec land here)

## Addendum (2026-07-26)

The Task CRUD and mesh (`submit`/`claim`/`heartbeat`/.../`complete_mesh_task` etc.) tool families
above are described as separate surfaces over separate tables (`task` vs. `meshtask`). As of migration
`0014_unify_task_meshtask.sql`, both are backed by one Postgres table (`public.task`), split by a `kind`
column (`'assigned'` | `'claimable'`) instead of by table. The keep-set / move-and-delete reasoning in
this ADR is unaffected — the mesh tools stay in the "keep" (runtime protocol) set because they operate
mid-execution on `kind='claimable'` rows under an active lease, and Task CRUD tools stay in the
"move-and-delete" set because they operate on `kind='assigned'` rows with no lease — but this ADR's
original table-per-surface framing is now inaccurate. See `docs/architecture/entities.md` § Task.
