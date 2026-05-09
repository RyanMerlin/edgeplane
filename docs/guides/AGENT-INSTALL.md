# MissionControl Agent Setup

## The Quick Way (recommended)

**Step 1 — Install mc:**

**Linux / macOS** (downloads prebuilt binary, falls back to source build):
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/missioncontrol-ai/missioncontrol/main/scripts/install-mc.sh)
```

**Windows** (PowerShell):
```powershell
irm https://raw.githubusercontent.com/missioncontrol-ai/missioncontrol/main/scripts/bootstrap-mc.ps1 | iex
```

**Step 2 — Authenticate:**

With a static token (simplest):
```bash
export MC_TOKEN="<your-token>"
export MC_BASE_URL="https://your-mc.example.com"
```

Or create a session token (recommended — see [Session tokens](#session-tokens)):
```bash
export MC_BASE_URL="https://your-mc.example.com"
MC_TOKEN="<your-token>" mc auth login   # saves ~/.missioncontrol/session.json
# MC_TOKEN no longer needed in env after this
```

**Step 3 — Launch your agent:**

```bash
mc run claude           # Claude Code
mc run codex            # OpenAI Codex CLI
mc run gemini           # Google Gemini CLI
mc launch openclaw      # OpenClaw
mc launch custom        # Custom ACP agent
```

That's it. `mc run <runtime>` is the unified launch path. `mc launch` remains for openclaw/custom and legacy compatibility.

Codex quick checks:
```bash
mc run codex status             # read-only quick status (human)
mc run codex status --json      # read-only quick status (machine)
mc run codex doctor --json      # detailed readiness diagnostics (machine)
```

---

## What `mc launch` does

1. Checks agent binary is on PATH (with install hint if not)
2. Validates profile/session context and pin policy (when configured)
3. Validates auth against the MC API
4. Fetches agent config from the onboarding manifest
5. Writes config to an instance-local runtime home by default (token not embedded if using session)
6. Injects `MC_TOKEN` into the agent's process environment
7. exec's the agent

## `mc launch` flags (non-Claude/Codex agents)

| Flag | Effect |
|---|---|
| `--preflight-only` | Validate env + auth without launching (CI-safe) |
| `--no-daemon` | Skip daemon management (daemon externally managed) |
| `--skip-config-gen` | Use existing config, skip manifest fetch |
| `--no-embed-token` | Omit `MC_TOKEN` from written config file (auto-implied for session tokens) |
| `--legacy-global-config` | Write config to global agent paths (`~/.codex`, `~/.gemini`) for compatibility |
| `--daemon-timeout N` | Seconds to wait for daemon ready (default: 15) |
| `-- <args>` | Pass remaining args verbatim to the agent |

## Agent Config Locations (default)

| Agent | Config written by `mc launch` |
|---|---|
| Gemini CLI | `~/.missioncontrol/instances/<runtime_session_id>/home/.gemini/settings.json` |
| OpenClaw | `~/.missioncontrol/instances/<runtime_session_id>/mc/config/openclaw.acp.json` |
| Custom ACP agent | `~/.missioncontrol/instances/<runtime_session_id>/mc/config/custom.acp.json` |

Use `--legacy-global-config` only when you explicitly need legacy global config writes.

---

## Session tokens

`mc auth login` exchanges your current credentials for a server-issued session token
(`mcs_*` prefix) stored at `~/.missioncontrol/session.json` (chmod 600).

Session tokens are:
- **Revocable** — `mc auth logout` revokes server-side instantly
- **Never written to agent config files** — injected into the agent process at exec time only
- **Auto-loaded** — `mc` reads `session.json` automatically when `MC_TOKEN` is not set
- **Expiring** — default 8h TTL, configurable with `--ttl-hours` (max 720h / 30 days)

### Login / logout / whoami

```bash
# Create a session (exchange any valid credential for an mcs_ token)
mc auth login                      # default 8h TTL
mc auth login --ttl-hours 24       # longer TTL
mc auth login --print-token        # print token to stdout (for scripting)

# Check identity and session expiry
mc auth whoami

# Revoke session server-side and clear local file
mc auth logout
mc auth logout --local-only        # clear local file only (no server call)
```

### Session token workflow

```bash
export MC_BASE_URL="https://your-mc.example.com"

# One-time: bootstrap a session from a static token
MC_TOKEN="<static-token>" mc auth login

# From now on — no MC_TOKEN needed in env
mc run claude   # session loaded from ~/.missioncontrol/session.json
mc run codex    # token injected into agent process at exec, not written to config
mc auth whoami          # verify identity
mc auth logout          # revoke when done
```

### OIDC / short-lived JWTs

When you authenticate via OIDC (Authentik, SSO), your `MC_TOKEN` is a short-lived JWT.
The recommended pattern is to exchange it for a session token immediately:

```bash
# Exchange OIDC JWT for a longer-lived mc session token
MC_TOKEN="$(get-oidc-token)" mc auth login --ttl-hours 8
mc run claude
```

Or run Claude directly with an env token:

```bash
export MC_TOKEN="$(get-oidc-token)"
mc run claude
```

**Token embedding rules in `mc launch` (non-Claude/Codex agents):**
- Session tokens (`mcs_*`) → never embedded, always injected at exec time
- `--no-embed-token` flag → never embedded
- `MC_TOKEN` absent → never embedded (auto-implied, notice printed)
- Static token present → embedded by default (can override with `--no-embed-token`)

---

## Manual Setup (alternative)

Use this path when you need explicit control over config or are integrating into CI.

### 1) Install mc

Download prebuilt binary (recommended):
```bash
bash scripts/install-mc.sh
```

Or build from source (requires Rust/cargo):
```bash
cd integrations/mc && cargo build --release && cp target/release/mc ~/.local/bin/mc
```

### 2) Set MissionControl Endpoint

```bash
export MC_BASE_URL="https://mc.example.com"
export MC_TOKEN="<your-token>"
```

### 3) Install mc (one-time per update)

```bash
bash scripts/install-mc.sh
```

By default installs to `~/.local/bin/mc`. Ensure `~/.local/bin` is on `PATH`.

### 4) Start Rust Daemon (every session)

```bash
mc daemon --shim-host 127.0.0.1 --shim-port 8765
```

Or via the convenience script:

```bash
bash scripts/start-mc-daemon.sh
```

### 5) Add MCP Server to Your Agent

Default shim-mode config (works for Claude Code, Gemini CLI, and others supporting `mcpServers`):

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

Codex TOML format (`~/.codex/config.toml`):

```toml
[mcp_servers.missioncontrol]
command = "mc"
args = ["serve"]
startup_timeout_sec = 45
tool_timeout_sec = 60
env = { MC_BASE_URL = "https://mc.example.com", MC_TOKEN = "<your-token>" }
```

Gemini CLI (`~/.gemini/settings.json`):

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

### 6) Validate In Agent

Ask agent to list tools and call one:

- list tools
- create a task in cluster 1
- list tasks in cluster 1

---

## Codex Swarm Workflow

For first-class Codex multi-session collaboration (without nested `codex exec`), follow:

- `docs/CODEX-SWARM-WORKFLOW.md`

## Skill Sync (Mission/Kluster Scope)

Resolve and materialize effective skills for an active mission/kluster:

```bash
mc data sync status --mission-id <mission-id> --kluster-id <optional-kluster-id>
```

---

## Auth Reference

| Auth type | How it works | Recommended for |
|---|---|---|
| Static `MC_TOKEN` | Shared secret, never expires | Local dev, CI |
| Session token (`mcs_*`) | DB-backed, revocable, expiring | Interactive use, OIDC users |
| OIDC JWT | Short-lived, identity-bound | SSO/Authentik environments |

All auth types work with `mc launch` for gemini/openclaw/custom. Codex/Claude use the dedicated command families.

## Troubleshooting: Startup Timeout

If Codex shows `MCP startup incomplete (failed: missioncontrol)`:

- Ensure `mc daemon` is running on `127.0.0.1:8765`.
- Use shim defaults (`MC_MCP_MODE=shim`, `MC_STARTUP_PREFLIGHT=none`).
- Ensure your MCP env vars are `MC_*` (not `MISSIONCONTROL_*`).
- Run `mc auth whoami` to verify auth is working before launching an agent.
