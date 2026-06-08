---
title: System Overview
description: How the EdgePlane components fit together — control plane, daemon, CLI, and persistence layers.
---

## Component Map

EdgePlane has four core components that cooperate to provide coordination, governance, and durable state for AI agent fleets.

```
┌─────────────────────────────────────────────────────────┐
│                     edgeplane (CLI / TUI)                       │
│         Operator interface, agent launcher, TUI          │
└─────────────────────┬───────────────────────────────────┘
                      │ HTTP / REST / SSE
┌─────────────────────▼───────────────────────────────────┐
│                edgeplane-tower                           │
│     Domains, missions, tasks, artifacts, approvals       │
│     Governance enforcement, SSE telemetry, OIDC auth     │
└──────┬──────────────┬────────────────────────┬──────────┘
       │              │                        │
  Postgres        S3 Storage               Git repos
  + pgvector    (artifact bytes)        (memory of record)
  (structured state)

┌─────────────────────────────────────────────────────────┐
│                    edgeplaned (daemon)                          │
│  Agent lifecycle, secrets brokering, task worker,        │
│  cron dispatch, profile management                       │
│  (connects to edgeplane-tower via HTTP)                  │
└─────────────────────────────────────────────────────────┘

Agents (Claude Code, Codex, Gemini, custom ACP agents)
connect to edgeplane-tower via MCP stdio (edgeplane serve)
```

## edgeplane — CLI and TUI

The primary operator interface. All interactivity: fleet views, agent launch, capability dispatch, and the full-screen TUI.

Key capabilities:
- `edgeplane tui` — full-screen terminal UI (agents, missions, feed, approvals, secrets, config)
- `edgeplane run <runtime>` — unified agent launcher
- `edgeplane auth` — session token management
- `edgeplane capabilities` — capability pack dispatch
- `edgeplane domain, edgeplane daemon mission ls, edgeplane daemon task ls, edgeplane agent list` — entity management
- `edgeplane health` — connectivity and server status

## edgeplane-tower — API Server

The Axum HTTP server backing the REST/SSE API. Runs independently from the CLI. Handles:

- Domain, mission, task, and artifact CRUD
- Agent registration and status tracking
- Governance enforcement (policy lifecycle, approval tokens)
- SSE telemetry for real-time event streaming
- OIDC authentication
- Automatic database migrations on startup

```bash
edgeplane-tower --bind 0.0.0.0:8008
```

Everything agents interact with via MCP tools routes through this server.

## edgeplaned — Headless Daemon

The executor daemon. Agents communicate with it via Unix socket; operators never interact with it directly. Manages:

- Agent subprocess lifecycle (launch, restart, crash recovery)
- Secrets brokering — agents receive `EP_SECRETS_SOCKET` and `EP_SECRETS_SESSION` instead of raw credentials
- Task worker — ephemeral subagent spawning for distributed mesh execution
- Cron dispatch — durable recurring job scheduling
- Profile management — operator profile sync and activation

Socket paths (`~/.edgeplane/edgeplaned/`):
- `mgmt.sock` — JSON-RPC 2.0 management gateway
- `secrets.sock` — secrets broker (agent subprocesses only)
- `edgeplaned.sock` — PTY attach gateway

## Optional Components

Two additional components extend EdgePlane for specific environments:

- **Web Dashboard** — React SPA served by edgeplane-tower. Provides a browser-based fleet view, ACP terminal sessions, live event feed, and governance approvals. Communicates exclusively through the tower REST/SSE/WebSocket API.
- **edgeplane-zrpc** — Optional Zellij WASM plugin. Adds focus-free PTY injection, scrollback reads, pane lifecycle events, and cancel signals for Zellij-hosted agents. Activated by setting `EDGEPLANE_ZRPC_PLUGIN_PATH`; unset means no behavior change.

## Persistence Layers

See [Persistence Model](/architecture/persistence/) for the full breakdown. Summary:

| Layer | What lives here | Authority |
|-------|----------------|-----------|
| **Postgres + pgvector** | All structured state — domains, missions, tasks, approvals, roles, ledger | Source of truth for coordination |
| **S3-compatible storage** | Artifact bytes, workspace files, document content | Working store |
| **Git** | Published, approved mutations | Memory of record |

## MCP Interface

Agents connect to EdgePlane via standard MCP stdio, served by `edgeplane serve`. This works with any MCP-compatible runtime — Claude Code, Codex, Gemini CLI, custom ACP agents.

Available MCP tools include: `create_domain`, `create_mission`, `create_task`, `claim_mesh_task`, `publish_pending_ledger_events`, `search_tasks`, `search_missions`, `get_entity_history`, and more. See [Reference: CLI](/reference/cli/) for the full surface.

## Request Lifecycle

A typical agent mutation (creating a task) flows:

1. Agent calls MCP tool → `edgeplane serve` → `edgeplane-tower` REST endpoint
2. Policy check runs — role membership, governance policy, approval requirements
3. If approved immediately: mutation recorded in Postgres, S3 updated if applicable
4. If approval required: enters ledger as `pending`
5. Approval granted (human via TUI, or automated via policy) → mutation promoted
6. If publication policy configured: route resolver picks repo/branch/path → Git commit → provenance written back to Postgres

## See Also

- [Persistence Model](/architecture/persistence/) — three-tier storage model in detail
- [Ephemeral Task Agents](/architecture/ephemeral-agents/) — distributed agent execution via mesh tasks
- [Component Reference](/architecture/components/) — per-component roles, config, and trust boundaries
- [Data Flow](/architecture/data-flow/) — how a task moves from creation to execution to artifact publication
- [Security Model](/architecture/security/) — authentication, authorization, and audit trail
- [Reference: edgeplaned Daemon](/reference/edgeplaned-daemon/) — daemon internals and secrets brokering
