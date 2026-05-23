---
title: System Overview
description: How the MissionControl components fit together — control plane, daemon, CLI, and persistence layers.
---

## Component Map

MissionControl has four core components that cooperate to provide coordination, governance, and durable state for AI agent fleets.

```
┌─────────────────────────────────────────────────────────┐
│                     mc (CLI / TUI)                       │
│         Operator interface, agent launcher, TUI          │
└─────────────────────┬───────────────────────────────────┘
                      │ HTTP / REST / SSE
┌─────────────────────▼───────────────────────────────────┐
│                mc-controlplane                           │
│     Domains, missions, tasks, artifacts, approvals       │
│     Governance enforcement, SSE telemetry, OIDC auth     │
└──────┬──────────────┬────────────────────────┬──────────┘
       │              │                        │
  Postgres        S3 Storage               Git repos
  + pgvector    (artifact bytes)        (memory of record)
  (structured state)

┌─────────────────────────────────────────────────────────┐
│                    mcd (daemon)                          │
│  Agent lifecycle, secrets brokering, task worker,        │
│  cron dispatch, profile management                       │
│  (connects to mc-controlplane via HTTP)                  │
└─────────────────────────────────────────────────────────┘

Agents (Claude Code, Codex, Gemini, custom ACP agents)
connect to mc-controlplane via MCP stdio (mc serve)
```

## mc — CLI and TUI

The primary operator interface. All interactivity: fleet views, agent launch, capability dispatch, and the full-screen TUI.

Key capabilities:
- `mc tui` — full-screen terminal UI (agents, missions, feed, approvals, secrets, config)
- `mc run <runtime>` — unified agent launcher
- `mc auth` — session token management
- `mc capabilities` — capability pack dispatch
- `mc domains / missions / tasks / agents` — entity management
- `mc health` — connectivity and server status

## mc-controlplane — API Server

The Axum HTTP server backing the REST/SSE API. Runs independently from the CLI. Handles:

- Domain, mission, task, and artifact CRUD
- Agent registration and status tracking
- Governance enforcement (policy lifecycle, approval tokens)
- SSE telemetry for real-time event streaming
- OIDC authentication
- Automatic database migrations on startup

```bash
mc-controlplane --serve --bind 0.0.0.0:8008
```

Everything agents interact with via MCP tools routes through this server.

## mcd — Headless Daemon

The executor daemon. Agents communicate with it via Unix socket; operators never interact with it directly. Manages:

- Agent subprocess lifecycle (launch, restart, crash recovery)
- Secrets brokering — agents receive `MC_SECRETS_SOCKET` and `MC_SECRETS_SESSION` instead of raw credentials
- Task worker — ephemeral subagent spawning for distributed mesh execution
- Cron dispatch — durable recurring job scheduling
- Profile management — operator profile sync and activation

Socket paths (`~/.mc/`):
- `mcd-mgmt.sock` — JSON-RPC 2.0 management gateway
- `mcd-secrets.sock` — secrets broker (agent subprocesses only)
- `mcd.sock` — PTY attach gateway

## Persistence Layers

See [Persistence Model](/missioncontrol/architecture/persistence/) for the full breakdown. Summary:

| Layer | What lives here | Authority |
|-------|----------------|-----------|
| **Postgres + pgvector** | All structured state — domains, missions, tasks, approvals, roles, ledger | Source of truth for coordination |
| **S3-compatible storage** | Artifact bytes, workspace files, document content | Working store |
| **Git** | Published, approved mutations | Memory of record |

## MCP Interface

Agents connect to MissionControl via standard MCP stdio, served by `mc serve`. This works with any MCP-compatible runtime — Claude Code, Codex, Gemini CLI, custom ACP agents.

Available MCP tools include: `create_domain`, `create_mission`, `create_task`, `claim_mesh_task`, `publish_pending_ledger_events`, `search_tasks`, `search_missions`, `get_entity_history`, and more. See [Reference: CLI](/missioncontrol/reference/cli/) for the full surface.

## Request Lifecycle

A typical agent mutation (creating a task) flows:

1. Agent calls MCP tool → `mc serve` → `mc-controlplane` REST endpoint
2. Policy check runs — role membership, governance policy, approval requirements
3. If approved immediately: mutation recorded in Postgres, S3 updated if applicable
4. If approval required: enters ledger as `pending`
5. Approval granted (human via TUI, or automated via policy) → mutation promoted
6. If publication policy configured: route resolver picks repo/branch/path → Git commit → provenance written back to Postgres

## See Also

- [Persistence Model](/missioncontrol/architecture/persistence/) — three-tier storage model in detail
- [Ephemeral Task Agents](/missioncontrol/architecture/ephemeral-agents/) — distributed agent execution via mesh tasks
- [Reference: mcd Daemon](/missioncontrol/reference/mcd-daemon/) — daemon internals and secrets brokering
