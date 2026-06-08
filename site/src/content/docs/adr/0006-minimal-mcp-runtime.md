---
title: "ADR-0006: Minimal MCP Surface — Runtime Protocol + Discovery Meta-Tools"
description: Reduce the tower MCP catalogue from 80 tools to ~19 by moving management operations to CLI and adding discover/exec meta-tools for agent access.
---

**Status:** Accepted (2026-06-08)

## Context

ADR 0005 established that CLI is the primary operator interface and MCP is the agent runtime
protocol. At the time of writing, the tower MCP catalogue advertised **80 tools** — consuming
significant agent context window before any work begins.

An audit of the tower route implementation revealed that MCP tool logic was **inline SQL forked
from the REST routes** with no shared service layer. This means a CLI command can hit the
existing REST routes directly and the corresponding MCP arm can simply be deleted.

## Decision

The tower MCP surface is limited to **operations an agent calls mid-execution while holding an
execution lease** — the claim/heartbeat/progress/complete loop, the workspace lease cycle, and
the message bus.

Two meta-tools bridge the gap for agents that need CLI access without a shell:

- `discover(path?, deep=false)` — walks the clap command tree; lets agents discover available
  commands without a shell
- `exec(args[])` — runs the local `edgeplane` binary as a subprocess; full CLI passthrough

These run locally in the `edgeplane serve` process — no tower round-trip.

## Target State: 80 → 19 tools

| Category | Count |
|----------|-------|
| Runtime tools (core keep set) | 14 |
| Borderline keep (re-evaluate at next audit) | 3 |
| Meta-tools in gateway (`discover`, `exec`) | 2 |
| **Total advertised** | **19** |

### Core keep set (14)

`submit_mesh_task`, `list_mesh_tasks`, `claim_mesh_task`, `heartbeat_mesh_task`,
`progress_mesh_task`, `complete_mesh_task`, `fail_mesh_task`, `block_mesh_task`,
`load_mission_workspace`, `heartbeat_workspace_lease`, `commit_mission_workspace`,
`release_mission_workspace`, `send_mesh_message`, `list_mesh_messages`

### Borderline keep (3)

`get_overlap_suggestions`, `fetch_workspace_artifact`, `get_mesh_task`

### Extract-first (7) — need REST routes before deletion

`get_artifact_download_url`, `export_domain_pack`, `install_domain_pack`,
`publish_pending_ledger_events`, `provision_domain_persistence`,
`resolve_publish_plan`, `get_publication_status`

### Move to CLI + delete arm (56)

All domain/mission/task CRUD, doc/artifact CRUD, agent management, mesh admin ops,
ledger, profiles, skill-sync, domain packs, remote launch.

See `docs/adr/0006-minimal-mcp-runtime-plus-discovery.md` in the repository for the
complete per-tool table with move rationale.

## Implementation Rules

1. Never delete an MCP arm whose logic has no REST/CLI equivalent — extract first.
2. Add a catalogue↔dispatch parity test so drift cannot silently recur.
3. The `exec` meta-tool is the migration safety net — hard cutover is safe because agents
   can reach any CLI path via `exec(["domain", "create", ...])`.

## Consequences

- Agent context windows shrink by ~60 tool schema definitions at session start
- All removed tools are reachable via `exec` — no capability regression
- New CLI commands are automatically MCP-accessible via `exec`; no new MCP arms ever required
  for management operations

## See Also

- [ADR-0005: CLI-First](/adr/0005-unified-agent-launcher/) — the founding principle
- [Command Map](/reference/command-map/) — full CLI surface auto-generated from the clap tree
