---
title: Agent Setup
description: Connect Claude Code, Codex, Gemini, or a custom agent to EdgePlane.
---

`edgeplane run` is the unified agent launcher. It validates your environment, fetches the agent's onboarding manifest, and injects EdgePlane as an MCP server before handing off to the agent binary.

## Prerequisites

- `edgeplane` installed and on `PATH` — see [Installation](/getting-started/installation/)
- Authenticated via `edgeplane auth login` (OIDC) or a service account token (`mcs_sa_*`) for CI

## Launching an Agent

```bash
edgeplane run claude           # Claude Code
edgeplane run codex            # OpenAI Codex CLI
edgeplane run gemini           # Google Gemini CLI
edgeplane launch openclaw      # OpenClaw
edgeplane launch custom        # Custom ACP agent
```

### What `edgeplane run` does

1. Checks that the agent binary is on PATH (prints an install hint if not)
2. Validates profile/session context against the server
3. Fetches agent config from the onboarding manifest
4. Writes runtime config to `~/.edgeplane/instances/<session-id>/`
5. Injects auth context into the agent's process environment (`EP_SECRETS_SOCKET` / `EP_SECRETS_SESSION` for session tokens)
6. `exec`s the agent

## Session Tokens

Session tokens (`mcs_*` prefix) are the recommended auth mechanism for interactive use. They are revocable, expiring, and never written to agent config files on disk.

```bash
# Exchange any valid credential for a session token
edgeplane auth login                       # default 8h TTL
edgeplane auth login --ttl-hours 24        # longer TTL
edgeplane auth login --print-token         # print token (for scripting)

edgeplane auth whoami                      # verify identity and expiry
edgeplane auth logout                      # revoke server-side and clear local file
edgeplane auth logout --local-only         # clear local file only
```

### OIDC / Short-lived JWTs

For SSO environments, run the browser-based OIDC flow to get a session token:

```bash
edgeplane auth login --ttl-hours 8
edgeplane run claude
```

| Auth type | Recommended for |
|-----------|----------------|
| OIDC (interactive) | Interactive use, SSO environments |
| Session token (`mcs_*`) | Any interactive session after `edgeplane auth login` |
| Service account (`mcs_sa_*`) | CI, headless pipelines |
| Node JWT | Daemons and machines (`edgeplaned`) |

## Manual MCP Server Setup

If you prefer to wire EdgePlane into an existing agent config manually:

**Claude Code (`.claude.json` or `mcpServers` block):**

```json
{
  "edgeplane": {
    "command": "edgeplane",
    "args": ["serve"],
    "env": {
      "EP_BASE_URL": "https://edgeplane.example.com"
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

## Diagnosing Issues

```bash
edgeplane run codex doctor --json       # detailed readiness diagnostics
edgeplane auth whoami                   # verify auth before launching
edgeplane health --json                 # verify server connectivity
```

If an agent shows `MCP startup incomplete (failed: edgeplane)`:

- Confirm `edgeplane auth whoami` succeeds before launching
- Use shim defaults (`EP_MCP_MODE=shim`, `EP_STARTUP_PREFLIGHT=none`)

## `edgeplane launch` Flags (non-Claude/Codex agents)

| Flag | Effect |
|------|--------|
| `--preflight-only` | Validate env and auth without launching (CI-safe) |
| `--no-daemon` | Skip daemon management (when daemon is externally managed) |
| `--skip-config-gen` | Use existing config, skip manifest fetch |
| `--no-embed-token` | Omit token from written config file (auto-implied for session tokens) |
| `--legacy-global-config` | Write config to global agent paths for compatibility |
| `--daemon-timeout N` | Seconds to wait for daemon ready (default: 15) |
| `-- <args>` | Pass remaining args verbatim to the agent |

## Skill Sync

To resolve and materialize effective skills for an active domain/mission:

```bash
edgeplane data sync status --domain-id <id> --mission-id <optional-id>
```

## Next Steps

- [Concepts: Domains, Missions & Tasks](/concepts/domains-missions-tasks/) — the organizational model
- [Reference: CLI](/reference/cli/) — full command surface
- [Reference: edgeplaned Daemon](/reference/edgeplaned-daemon/) — secrets brokering and daemon internals
