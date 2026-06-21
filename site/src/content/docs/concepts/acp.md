---
title: ACP — Agent Client Protocol
description: How EdgePlane manages persistent agent sessions — the transport model, session lifecycle, and supported runtimes.
---

ACP is EdgePlane's mechanism for maintaining persistent, bidirectional communication with long-running AI agents. Unlike subprocess execution that fires and forgets, ACP sessions survive crashes and reconnect automatically. The agent keeps working; EdgePlane keeps the session record and the attach path alive.

## What ACP Is

When you run `edgeplane run claude`, EdgePlane does more than spawn a process. It:

1. Fetches your active profile from the server
2. Injects EdgePlane as an MCP server into the agent's configuration
3. Registers an `AgentSession` on the control plane
4. Monitors the process and reconnects on failure

The agent communicates back through EdgePlane's MCP tools — creating tasks, publishing artifacts, reporting governance actions — using the same JSON-RPC stdio transport that MCP defines. No sidecar, no separate auth token: the MCP server is the control plane boundary.

## Transport Model

There are three communication paths:

**MCP tools over JSON-RPC stdio** — the primary path. The agent calls EdgePlane MCP tools freely: creating tasks, fetching domain context, publishing artifacts, requesting governance approvals. This is standard MCP — works with any MCP-compatible runtime.

**WebSocket attach (`/api/runtime/nodes/{node_id}/agents/{agent_id}/attach`)** — operators and the web dashboard attach for real-time I/O. The attach endpoint proxies frames to the running agent process via the per-node daemon. Owner-scope is enforced — you can only attach to sessions you own.

**SSE streams** — scoped per session at `/api/ai/sessions/{id}/stream` for AI Console conversations. Session lifecycle events (tool calls, heartbeats, termination) stream here; the web dashboard's conversation pane reads this path.

## Session Lifecycle

```
edgeplane run claude
  → edgeplaned spawns Claude Code with EdgePlane in mcpServers
  → EdgePlane MCP server registers the AgentSession
  → Session status: active
  → Agent works; calls EdgePlane MCP tools freely
  → Crash / network drop → edgeplaned restarts; session reattaches
  → Clean exit → session marked terminated
```

The `AgentSession` record persists across restarts. On reconnect, the same session identity is reused, so conversation continuity (and any `AgentRun` bindings) survive a process crash.

## Attach Secret

Each node gets a short-lived `attach_secret` at registration time. This secret is:

- Written into the agent's environment at process startup
- Used by the web dashboard and `edgeplane attach` to authenticate WebSocket connections
- Self-healing: regenerated on node re-registration and propagated to running agents on reconnect
- Scoped to the node's owner — the tower enforces this at the `/api/runtime` boundary

The attach secret is not a long-lived credential. If you rotate it (by re-registering the node), active attach connections are invalidated and must reconnect.

## Supported Runtimes

| Runtime | Kind | Session model |
|---------|------|---------------|
| Claude Code | `claude_agent_acp` | ACP persistent — EdgePlane injected as MCP server at startup |
| Zellij pane | `zellij_hosted` | PTY bridge — EdgePlane bridges via PTY; optional `edgeplane-zrpc` plugin for focus-free injection |
| Codex | `codex` | Driver subprocess with instance isolation |
| Gemini | `gemini` | Driver subprocess with instance isolation |
| Goose | `goose` | Driver subprocess with instance isolation |
| OpenClaw | `openclaw` | Driver subprocess |
| Custom | `custom` | Driver subprocess |

All runtimes are launched through `edgeplane run <runtime>` — the single entry point for starting any agent.

## The zellij_hosted Runtime

For agents running in a Zellij pane, EdgePlane bridges via PTY rather than MCP stdio. Input frames arriving over the WebSocket attach path are converted to PTY stdin bytes; output streams back as SSE frames.

The optional `edgeplane-zrpc` plugin extends this with focus-free injection: it replaces the paste→delay→Enter chain with a Zellij-pipe–based request/response path that does not require the pane to have keyboard focus. The plugin ships feature-flagged — set `EDGEPLANE_ZRPC_PLUGIN_PATH` and `EDGEPLANE_ZRPC_SESSIONS` to enable it.

See [Zellij Integration](/guides/zellij-integration/) for setup details.

## What's Next

- [MeshTask System](/concepts/mesh-tasks/) — how agents claim and execute distributed work
- [Architecture: Data Flow](/architecture/data-flow/) — full request path from agent to control plane
- [Guides: Web Dashboard](/guides/web-dashboard/) — attaching to live sessions via the UI
