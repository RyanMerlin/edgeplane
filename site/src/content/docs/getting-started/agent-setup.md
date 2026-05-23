---
title: Agent Setup
description: Connect Claude Code, Codex, Gemini, or a custom agent to MissionControl.
---

`mc run` is the unified agent launcher. It validates your environment, fetches the agent's onboarding manifest, and injects MissionControl as an MCP server before handing off to the agent binary.

## Prerequisites

- `mc` installed and on `PATH` — see [Installation](/missioncontrol/getting-started/installation/)
- `MC_BASE_URL` and `MC_TOKEN` set (or a session token from `mc auth login`)

## Launching an Agent

```bash
mc run claude           # Claude Code
mc run codex            # OpenAI Codex CLI
mc run gemini           # Google Gemini CLI
mc launch openclaw      # OpenClaw
mc launch custom        # Custom ACP agent
```

### What `mc run` does

1. Checks that the agent binary is on PATH (prints an install hint if not)
2. Validates profile/session context against the server
3. Fetches agent config from the onboarding manifest
4. Writes runtime config to `~/.missioncontrol/instances/<session-id>/`
5. Injects `MC_TOKEN` into the agent's process environment (session tokens are never written to config files)
6. `exec`s the agent

## Session Tokens

Session tokens (`mcs_*` prefix) are the recommended auth mechanism for interactive use. They are revocable, expiring, and never written to agent config files on disk.

```bash
# Exchange any valid credential for a session token
mc auth login                       # default 8h TTL
mc auth login --ttl-hours 24        # longer TTL
mc auth login --print-token         # print token (for scripting)

mc auth whoami                      # verify identity and expiry
mc auth logout                      # revoke server-side and clear local file
mc auth logout --local-only         # clear local file only
```

### OIDC / Short-lived JWTs

When your `MC_TOKEN` is an OIDC JWT (from SSO), exchange it for a session token at the start of each session:

```bash
MC_TOKEN="$(get-oidc-token)" mc auth login --ttl-hours 8
mc run claude
```

| Auth type | Recommended for |
|-----------|----------------|
| Static `MC_TOKEN` | Local dev, CI |
| Session token (`mcs_*`) | Interactive use, OIDC users |
| OIDC JWT | SSO environments (exchange for session token) |

## Manual MCP Server Setup

If you prefer to wire MissionControl into an existing agent config manually:

**Claude Code (`.claude.json` or `mcpServers` block):**

```json
{
  "missioncontrol": {
    "command": "mc",
    "args": ["serve"],
    "env": {
      "MC_BASE_URL": "https://mc.example.com",
      "MC_TOKEN": "<your-token>"
    }
  }
}
```

**Codex (`~/.codex/config.toml`):**

```toml
[mcp_servers.missioncontrol]
command = "mc"
args = ["serve"]
startup_timeout_sec = 45
tool_timeout_sec = 60
env = { MC_BASE_URL = "https://mc.example.com", MC_TOKEN = "<your-token>" }
```

**Gemini CLI (`~/.gemini/settings.json`):**

```json
{
  "mcpServers": {
    "missioncontrol": {
      "command": "mc",
      "args": ["serve"],
      "env": {
        "MC_BASE_URL": "https://mc.example.com",
        "MC_TOKEN": "<your-token>"
      }
    }
  }
}
```

## Diagnosing Issues

```bash
mc run codex doctor --json       # detailed readiness diagnostics
mc auth whoami                   # verify auth before launching
mc health --json                 # verify server connectivity
```

If an agent shows `MCP startup incomplete (failed: missioncontrol)`:

- Confirm `mc auth whoami` succeeds before launching
- Use shim defaults (`MC_MCP_MODE=shim`, `MC_STARTUP_PREFLIGHT=none`)
- Ensure env vars use the `MC_*` prefix, not `MISSIONCONTROL_*`

## `mc launch` Flags (non-Claude/Codex agents)

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
mc data sync status --domain-id <id> --mission-id <optional-id>
```

## Next Steps

- [Concepts: Domains, Missions & Tasks](/missioncontrol/concepts/domains-missions-tasks/) — the organizational model
- [Reference: CLI](/missioncontrol/reference/cli/) — full command surface
- [Reference: mcd Daemon](/missioncontrol/reference/mcd-daemon/) — secrets brokering and daemon internals
