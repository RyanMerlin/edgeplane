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
│     Domains, missions, tasks, artifacts, ledger          │
│     Authorization (membership-based), SSE, OIDC auth     │
└──────┬─────────────────────────────────┬─────────────────┘
       │                                 │
  Postgres                          Git repos
  (structured state +               (memory of record)
   artifact content today)

┌─────────────────────────────────────────────────────────┐
│                    edgeplaned (daemon)                          │
│  Agent lifecycle, secrets brokering, task worker,        │
│  cron dispatch, profile management                       │
│  (connects to edgeplane-tower via HTTP)                  │
└─────────────────────────────────────────────────────────┘

Agents (Claude Code, Codex, Gemini, custom ACP agents)
connect to edgeplane-tower via MCP stdio (edgeplane serve)
```

![System architecture diagram](/diagrams/system-architecture.svg)

## edgeplane — CLI and TUI

The primary operator interface. All interactivity: fleet views, agent launch, capability dispatch, and the full-screen TUI.

Key capabilities:
- `edgeplane tui` — full-screen terminal UI (agents, domains, feed, secrets, config)
- `edgeplane run <runtime>` — unified agent launcher
- `edgeplane auth` — session token management
- `edgeplane capabilities` — capability pack dispatch
- `edgeplane domain, edgeplane mission list, edgeplane task list, edgeplane agent list` — entity management
- `edgeplane health` — connectivity and server status

## edgeplane-tower — API Server

The Axum HTTP server backing the REST/SSE API. Runs independently from the CLI. Handles:

- Domain, mission, task, and artifact CRUD
- Agent registration and status tracking
- Authorization enforcement — membership-based, default-deny (per-domain `owners`/`contributors` plus an `EP_ADMIN_EMAILS` admin allowlist). A versioned governance policy engine with approval tokens existed early on but was dropped (migration `0009_drop_governance.sql`) and is on the roadmap, not current behavior
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

- **Web Dashboard** — React SPA served by edgeplane-tower. Provides a browser-based fleet view, ACP terminal sessions, live event feed, and domain/task drill-down. Communicates exclusively through the tower REST/SSE/WebSocket API.
- **edgeplane-zrpc** — Optional Zellij WASM plugin. Adds focus-free PTY injection, scrollback reads, pane lifecycle events, and cancel signals for Zellij-hosted agents. Activated by setting `EDGEPLANE_ZRPC_PLUGIN_PATH`; unset means no behavior change.

## Persistence Layers

See [Persistence Model](/architecture/persistence/) for the full breakdown. Summary:

| Layer | What lives here | Authority |
|-------|----------------|-----------|
| **Postgres** | All structured state — domains, missions, tasks, artifact content, domain ownership, ledger | Source of truth for coordination |
| **S3-compatible storage** (planned) | Artifact bytes, workspace files, document content — not implemented yet; content is inline in Postgres today | Working store (target design) |
| **Git** | Published mutations | Memory of record |

`pgvector` is not in use — there's no embedding generation or vector search anywhere in the stack today.

## MCP Interface

Agents connect to EdgePlane two ways: standard MCP stdio, served by `edgeplane serve`, and an HTTP MCP surface at `/api/mcp/tools` (catalogue) and `/api/mcp/call` (dispatch). Both work with any MCP-compatible runtime — Claude Code, Codex, Gemini CLI, custom ACP agents — with no sidecar or custom SDK required.

Available MCP tools include: `submit_mesh_task`, `claim_mesh_task`, `load_mission_workspace`, `commit_mission_workspace`, `publish_pending_ledger_events`, `resolve_publish_plan`, `get_overlap_suggestions`, `send_mesh_message`, and more. See [Reference: CLI](/reference/cli/) for the full surface.

## Request Lifecycle

A typical agent mutation (creating a task) flows:

1. Agent calls MCP tool → `edgeplane serve` (or the HTTP MCP surface) → `edgeplane-tower` REST endpoint
2. Authorization check — caller must be a domain owner/contributor or an admin (`EP_ADMIN_EMAILS`); there is no separate approval-workflow gate on ordinary mutations
3. If authorized: mutation recorded in Postgres immediately
4. If the mutation is publish-eligible: it can be routed to Git — route resolver picks repo/branch/path (`resolve_publish_plan`) → Git commit → provenance written back to Postgres, and the ledger entry (initially `pending`) is marked published

## See Also

- [Persistence Model](/architecture/persistence/) — three-tier storage model in detail
- [Ephemeral Task Agents](/architecture/ephemeral-agents/) — distributed agent execution via mesh tasks
- [Component Reference](/architecture/components/) — per-component roles, config, and trust boundaries
- [Data Flow](/architecture/data-flow/) — how a task moves from creation to execution to artifact publication
- [Security Model](/architecture/security/) — authentication, authorization, and audit trail
- [Reference: edgeplaned Daemon](/reference/edgeplaned-daemon/) — daemon internals and secrets brokering
