---
title: Advanced Agent Configuration
description: Multi-instance setup, CI integration, custom ACP agents, and skill sync.
---

This guide covers advanced agent configuration scenarios. For the basic setup walkthrough, see [Getting Started: Agent Setup](/edgeplane/getting-started/agent-setup/).

## Multi-Profile Workflows

Operators can create multiple profiles — one per work context — and switch between them instantly.

```bash
# List profiles
edgeplane profiles list

# Switch active profile
edgeplane profiles activate research

# Each profile carries its own:
# - Tool and integration settings
# - Mission context and permissions
# - Instruction files for the agent
# - Governance posture
```

Profile files live in `~/.edgeplane/profiles/<name>/` and are synced from the server on activation.

## CI / Headless Environments

For CI pipelines that need to validate the environment without launching an agent:

```bash
# Validate env and auth without launching
edgeplane run codex --preflight-only

# Or for other agents
edgeplane launch gemini --preflight-only
```

Verify readiness before running an agent:

```bash
edgeplane auth whoami
edgeplane health --json
```

For CI, use a long-lived static `EP_TOKEN` rather than a session token (session tokens have an 8h default TTL):

```bash
export EP_BASE_URL="https://edgeplane.example.com"
export EP_TOKEN="<ci-service-token>"
edgeplane run codex doctor --json
```

## Custom ACP Agents

Any agent that implements the ACP protocol can connect to Edgeplane.

**Generate the config:**

```bash
edgeplane launch custom \
  --agent-binary /path/to/your-agent \
  --mission-id <id> \
  --preflight-only   # validate first
```

**Manual config (`~/.edgeplane/instances/<id>/edgeplane/config/custom.acp.json`):**

```json
{
  "ep_base_url": "https://edgeplane.example.com",
  "domain_id": "<id>",
  "mission_id": "<id>",
  "mcp_servers": {
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

Session tokens are injected at exec time — never embed them in config files.

## Skill Sync

Resolve and materialize the effective skill set for a domain/mission before launching agents:

```bash
edgeplane data sync status --domain-id <id>
edgeplane data sync status --domain-id <id> --mission-id <id>
```

Skills are versioned capability bundles — code, tools, and configuration an agent should have available. Skill sync ensures the agent's local state matches what's published for the mission.

## Daemon Management

`edgeplaned` manages agent subprocess lifecycle. Most of the time you don't interact with it directly, but useful commands:

```bash
# Check daemon status
edgeplaned version

# Retrieve a secret from inside an agent subprocess
edgeplaned get-secret MY_API_KEY
```

Socket paths (`~/.ep/`):
- `edgeplaned-mgmt.sock` — JSON-RPC management
- `edgeplaned-secrets.sock` — secrets broker (agents only)
- `edgeplaned.sock` — PTY attach

## Agent Config Locations

| Agent | Config path |
|-------|-------------|
| Gemini CLI | `~/.edgeplane/instances/<id>/home/.gemini/settings.json` |
| OpenClaw | `~/.edgeplane/instances/<id>/edgeplane/config/openclaw.acp.json` |
| Custom ACP | `~/.edgeplane/instances/<id>/edgeplane/config/custom.acp.json` |

Pass `--legacy-global-config` to write config to global agent paths (`~/.gemini`, etc.) for compatibility with tools that only read from a fixed location.

## Fleet-Level Agent Setup

For running multiple concurrent agents across missions:

1. Register agents explicitly for fleet visibility:

```bash
# Via MCP tool
register_agent(
  name = "my-agent",
  capabilities = "code-editing,research",
  metadata = '{"runtime":"claude-code","node_id":"my-host"}'
)
```

2. Update status on startup:

```bash
update_agent_status(agent_id = <id>, status = "online")
```

3. Anchor to a home domain:

```bash
edgeplane agent update --agent-id <id> --home-domain-id <domain-id>
```

## Troubleshooting

| Symptom | Check |
|---------|-------|
| `MCP startup incomplete (failed: edgeplane)` | `edgeplane auth whoami` — auth must succeed before launch |
| Token written to config file | Session tokens (`mcs_*`) are never written to disk — if you see a token in a config file, it's a static token |
| Agent can't reach the server | `edgeplane health --json` — verify `EP_BASE_URL` is set correctly |
| `connection refused` on MCP tools | Ensure `edgeplane serve` can start — check `EP_TOKEN` / `EP_BASE_URL` in the agent's environment |

## See Also

- [Getting Started: Agent Setup](/edgeplane/getting-started/agent-setup/) — basic setup
- [Reference: CLI](/edgeplane/reference/cli/) — full command surface
- [Reference: edgeplaned Daemon](/edgeplane/reference/edgeplaned-daemon/) — daemon internals and secrets brokering
