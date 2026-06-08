---
title: "ADR-0005: CLI-First — MCP Is the Agent Runtime Protocol"
description: Decision to use CLI as the primary operator interface and restrict MCP to the in-flight agent runtime protocol.
---

**Status:** Accepted (2026-06-08)

## Context

EdgePlane exposes approximately 65 MCP tools via `edgeplane serve`. Many of these are management
and CRUD operations (create domain, list missions, update task, search artifacts). MCP was the
path of least resistance to expose tower functionality to AI agents, but it became the *only* path
for many operations — including ones that human operators should be able to run from a shell.

This creates several problems:

- Operators cannot create domains, missions, or tasks without running an agent or calling MCP directly
- Shell scripts and CI pipelines have no reliable CLI surface for management operations
- The MCP surface grows unbounded as new features are added, because there is no CLI alternative to push back on it
- Documentation and discoverability suffer — `edgeplane --help` does not reflect what the system can actually do

## Decision

**CLI is the primary operator interface. MCP is the agent runtime protocol. These are different concerns.**

### What stays MCP-only

Operations that are genuinely in-flight agent protocol — called programmatically, sub-second, mid-task execution:

- `claim_mesh_task` / `complete_mesh_task` / `fail_mesh_task` / `retry_mesh_task` / `block_mesh_task` / `unblock_mesh_task`
- `heartbeat_mesh_task` / `heartbeat_workspace_lease`
- `progress_mesh_task`
- `submit_mesh_task`
- `send_mesh_message` / `list_mesh_messages`
- `load_mission_workspace` / `commit_mission_workspace` / `release_mission_workspace`
- `start_agent_session` / `end_agent_session`
- `update_agent_status`

These are the MCP surface. They are called by AI agents during task execution, not by humans managing infrastructure.

### What must have a CLI equivalent

Every management and CRUD operation must be reachable via `edgeplane` CLI. MCP may also expose it
(agents need it too), but the CLI command comes first — the MCP tool is a projection of the CLI,
not the other way around.

| CLI command | MCP equivalent |
|---|---|
| `edgeplane domain create` | `create_domain` |
| `edgeplane domain list` | `list_domains` |
| `edgeplane domain show <id>` | (read from list) |
| `edgeplane domain update <id>` | `update_domain` |
| `edgeplane domain delete <id>` | `delete_domain` |
| `edgeplane mission create` | `create_mission` |
| `edgeplane mission list` | `search_missions` |
| `edgeplane mission show <id>` | (read from search) |
| `edgeplane mission update <id>` | `update_mission` |
| `edgeplane mission delete <id>` | `delete_mission` |
| `edgeplane task create` | `create_task` |
| `edgeplane task list` | `list_tasks` / `search_tasks` |
| `edgeplane task show <id>` | (read from list) |
| `edgeplane task update <id>` | `update_task` |
| `edgeplane task delete <id>` | `delete_task` |
| `edgeplane artifact create` | `create_artifact` |
| `edgeplane artifact list` | `list_tasks` (artifact context) |
| `edgeplane history <entity-id>` | `get_entity_history` |

## Implementation Rule

When adding a new feature that needs to be accessible to operators:

1. **Write the CLI command first.** If you cannot describe it as an `edgeplane <noun> <verb>`
   command, the feature surface is not well-designed.
2. **Expose as MCP only if agents need to call it.** Many management operations do not need MCP
   at all.
3. **Never add an MCP tool without a documented reason it cannot be a CLI command.**

## Consequences

- Operators can manage domains, missions, and tasks entirely from the shell
- Shell scripts, CI pipelines, and runbooks can use `edgeplane` directly
- MCP surface shrinks over time as management tools are retired in favor of CLI equivalents
- New contributors see the full system capability in `edgeplane --help`
- Breaking: MCP tools that have CLI equivalents may be deprecated over time; callers should
  migrate to CLI for management operations

## Follow-up

- Update `COMMAND-MAP.md` as CLI commands are added
