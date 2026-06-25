---
title: Agent Setup
description: Connect an AI agent to EdgePlane — runtimes, profiles, session tokens, and MCP configuration.
---

`edgeplane run` is the unified agent launcher. It validates your environment, fetches the agent's onboarding manifest, and injects EdgePlane as an MCP server before handing off to the agent binary.

## Supported Runtimes

All runtimes launch through `edgeplane run <runtime>`:

| Runtime | Command | Agent type |
|---------|---------|-----------|
| `claude` | `edgeplane run claude` | Claude Code (ACP, persistent session) |
| `codex` | `edgeplane run codex` | OpenAI Codex CLI |
| `gemini` | `edgeplane run gemini` | Google Gemini CLI |
| `goose` | `edgeplane run goose` | Goose (Block) — profile-scoped home, `doctor`/`exec`/`status` |
| `openclaw` | `edgeplane run openclaw` | OpenClaw driver agent |
| `custom` | `edgeplane run custom` | Custom ACP agent with instance isolation |

## ACP Runtimes (Claude Code)

The `claude` runtime uses the **Agent Communication Protocol (ACP)**: a persistent JSON-RPC session over stdio. EdgePlane injects itself as an MCP server at launch, then maintains a live session until the agent exits or completes naturally.

Key behaviors:

- Session persists across compaction — EdgePlane re-injects context on compact
- Lifecycle hooks fire automatically: session registration, context injection, tool-audit, session-end
- `edgeplane run claude` resumes the previous session automatically; pass `--new` to force a fresh session instead

### What `edgeplane run` does

1. Checks that the agent binary is on `PATH` (prints an install hint if not)
2. Validates profile and session context against the tower
3. Fetches agent config from the onboarding manifest
4. Writes runtime config to `~/.edgeplane/instances/<session-id>/`
5. Injects EdgePlane by writing itself into the agent's `mcpServers` configuration before handoff — no manual MCP config required
6. `exec`s the agent

## Session Tokens

Session tokens are revocable, expiring, and **never written to agent config files on disk**. They are injected at exec time from `~/.edgeplane/session.json`.

```bash
edgeplane auth login                       # browser OIDC flow → ~/.edgeplane/session.json (8h TTL)
edgeplane auth login --ttl-hours 24        # longer TTL
edgeplane auth login --print-token         # print token value (for scripting)

edgeplane auth whoami                      # verify identity and expiry
edgeplane auth logout                      # revoke server-side and clear local file
edgeplane auth logout --local-only         # clear local file only
```

For CI and headless pipelines, use **service account tokens** (`ep_sa_*`) created via the API. There is no `EP_TOKEN` — static shared-secret auth was removed in v0.11.0. For non-interactive use (CI, automation), set `EP_AGENT_TOKEN` to a service account token (`ep_sa_*`) instead of using `edgeplane auth login`.

| Auth type | Recommended for |
|-----------|----------------|
| OIDC interactive (`edgeplane auth login`) | Interactive use, SSO environments |
| Service account (`ep_sa_*`) | CI, headless pipelines |
| Node JWT | Daemons and machines (`edgeplaned`) |

## Profiles

Profiles carry an operator's personal environment config, tool settings, and instruction files. The profile loads automatically when you run `edgeplane run`.

```bash
edgeplane profile activate --name <name>    # switch to a different profile
edgeplane profile publish --name <name>     # push local profile to the tower
edgeplane profile pull --name <name>        # pull profile from the tower
```

## MCP Configuration (manual wiring)

If you're running Claude Code manually without `edgeplane run`, add EdgePlane as an MCP server in your `.mcp.json` or `mcpServers` block:

**Claude Code (`.mcp.json`):**

```json
{
  "mcpServers": {
    "edgeplane": {
      "command": "edgeplane",
      "args": ["serve"],
      "env": {
        "EP_BASE_URL": "https://edgeplane.example.com"
      }
    }
  }
}
```

**Codex (`~/.codex/config.toml`):**

```toml
[mcp_servers.edgeplane]
command = "edgeplane"
args = ["serve"]
startup_timeout_sec = 45
tool_timeout_sec = 60
env = { EP_BASE_URL = "https://edgeplane.example.com" }
```

**Gemini CLI (`~/.gemini/settings.json`):**

```json
{
  "mcpServers": {
    "edgeplane": {
      "command": "edgeplane",
      "args": ["serve"],
      "env": {
        "EP_BASE_URL": "https://edgeplane.example.com"
      }
    }
  }
}
```

`edgeplane serve` reads auth automatically from `~/.edgeplane/session.json` (OIDC session) or the node JWT at `/etc/edgeplane/node.json`. No token in the config is needed.

:::tip
`edgeplane run claude` wires the MCP server automatically — manual `.mcp.json` config is only needed when launching Claude Code outside of `edgeplane run`.
:::

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `EP_BASE_URL` | Control plane HTTP base URL | `http://localhost:8008` |

Output format is controlled by the `--json` global flag on any command (e.g. `edgeplane health --json`).

There is no `EP_TOKEN`. Auth is handled via session file, node JWT, or service account token — never a static shared secret.

## Diagnosing Issues

```bash
edgeplane run codex doctor --json       # detailed readiness diagnostics
edgeplane auth whoami                   # verify auth before launching
edgeplane health --json                 # verify server connectivity
```

If an agent shows `MCP startup incomplete (failed: edgeplane)`:

- Confirm `edgeplane auth whoami` succeeds before launching
- Try shim defaults: `EP_MCP_MODE=shim EP_STARTUP_PREFLIGHT=none edgeplane run <runtime>`

## What's Next

- [Concepts: ACP](/concepts/acp/) — how persistent agent sessions work
- [Concepts: Domains, Missions & Tasks](/concepts/domains-missions-tasks/) — the organizational model
- [Guides: Multi-Agent Fleet](/guides/multi-agent-fleet/) — running multiple agents in coordination
