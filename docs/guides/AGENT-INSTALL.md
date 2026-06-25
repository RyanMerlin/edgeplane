# Edgeplane Agent Setup

## The Quick Way (recommended)

**Step 1 — Install edgeplane:**

**Linux / macOS** (downloads prebuilt binary, falls back to source build):
```bash
bash <(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/install-edgeplane.sh)
```

**Windows** (PowerShell):
```powershell
irm https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/bootstrap-edgeplane.ps1 | iex
```

**Step 2 — Authenticate:**

With a static token (simplest):
```bash
export EP_AGENT_TOKEN="<your-token>"
export EP_BASE_URL="https://your-edgeplane.example.com"
```

Or create a session token (recommended — see [Session tokens](#session-tokens)):
```bash
export EP_BASE_URL="https://your-edgeplane.example.com"
EP_AGENT_TOKEN="<your-token>" edgeplane auth login   # saves ~/.edgeplane/session.json
# EP_AGENT_TOKEN no longer needed in env after this
```

**Step 3 — Launch your agent:**

```bash
edgeplane run claude           # Claude Code
edgeplane run codex            # OpenAI Codex CLI
edgeplane run gemini           # Google Gemini CLI
edgeplane run openclaw         # OpenClaw (ACP)
edgeplane run custom           # Custom ACP agent
```

That's it. `edgeplane run <runtime>` is the single launch path for every agent.
(The old `edgeplane launch` command has been removed — use `edgeplane run`.)

Codex quick checks:
```bash
edgeplane run codex status             # read-only quick status (human)
edgeplane run codex status --json      # read-only quick status (machine)
edgeplane run codex doctor --json      # detailed readiness diagnostics (machine)
```

---

## What `edgeplane run <driver agent>` does

For the driver-managed agents (gemini, openclaw, custom), `edgeplane run`:

1. Checks the agent binary is on PATH (with install hint if not)
2. Validates profile/session context and pin policy (when configured)
3. Validates auth against the Edgeplane API
4. Fetches agent config from the onboarding manifest
5. Writes config to an instance-local runtime home by default (token not embedded if using a session)
6. Injects `EP_AGENT_TOKEN` into the agent's process environment
7. exec's the agent

(claude and codex are native runtimes with their own profile-scoped
homes and `doctor`/`exec`/`status` actions; see `edgeplane run <runtime> --help`.)

## Agent Config Locations (driver agents, default)

| Agent | Config written by `edgeplane run` |
|---|---|
| Gemini CLI | `~/.edgeplane/instances/<runtime_session_id>/home/.gemini/settings.json` |
| OpenClaw | `~/.edgeplane/instances/<runtime_session_id>/edgeplane/config/openclaw.acp.json` |
| Custom ACP agent | `~/.edgeplane/instances/<runtime_session_id>/edgeplane/config/custom.acp.json` |

---

## Session tokens

`edgeplane auth login` exchanges your current credentials for a server-issued session token
(`ep_*` prefix) stored at `~/.edgeplane/session.json` (chmod 600).

Session tokens are:
- **Revocable** — `edgeplane auth logout` revokes server-side instantly
- **Never written to agent config files** — injected into the agent process at exec time only
- **Auto-loaded** — `edgeplane` reads `session.json` automatically when `EP_AGENT_TOKEN` is not set
- **Expiring** — default 8h TTL, configurable with `--ttl-hours` (max 720h / 30 days)

### Login / logout / whoami

```bash
# Create a session (exchange any valid credential for an ep_ token)
edgeplane auth login                      # default 8h TTL
edgeplane auth login --ttl-hours 24       # longer TTL
edgeplane auth login --print-token        # print token to stdout (for scripting)

# Check identity and session expiry
edgeplane auth whoami

# Revoke session server-side and clear local file
edgeplane auth logout
edgeplane auth logout --local-only        # clear local file only (no server call)
```

### Session token workflow

```bash
export EP_BASE_URL="https://your-edgeplane.example.com"

# One-time: bootstrap a session from a static token
EP_AGENT_TOKEN="<static-token>" edgeplane auth login

# From now on — no EP_AGENT_TOKEN needed in env
edgeplane run claude   # session loaded from ~/.edgeplane/session.json
edgeplane run codex    # token injected into agent process at exec, not written to config
edgeplane auth whoami          # verify identity
edgeplane auth logout          # revoke when done
```

### OIDC / short-lived JWTs

When you authenticate via OIDC (Authentik, SSO), your `EP_AGENT_TOKEN` is a short-lived JWT.
The recommended pattern is to exchange it for a session token immediately:

```bash
# Exchange OIDC JWT for a longer-lived edgeplane session token
EP_AGENT_TOKEN="$(get-oidc-token)" edgeplane auth login --ttl-hours 8
edgeplane run claude
```

Or run Claude directly with an env token:

```bash
export EP_AGENT_TOKEN="$(get-oidc-token)"
edgeplane run claude
```

**Token embedding rules for driver agents (`edgeplane run` gemini/openclaw/custom):**
- Session tokens (`ep_*`) → never embedded, always injected at exec time
- `--no-embed-token` flag → never embedded
- `EP_AGENT_TOKEN` absent → never embedded (auto-implied, notice printed)
- Static token present → embedded by default (can override with `--no-embed-token`)

---

## Manual Setup (alternative)

Use this path when you need explicit control over config or are integrating into CI.

### 1) Install edgeplane

Download prebuilt binary (recommended):
```bash
bash scripts/install-edgeplane.sh
```

Or build from source (requires Rust/cargo):
```bash
cd crates/edgeplane && cargo build --release && cp target/release/edgeplane ~/.local/bin/edgeplane
```

### 2) Set Edgeplane Endpoint

```bash
export EP_BASE_URL="https://edgeplane.example.com"
export EP_AGENT_TOKEN="<your-token>"
```

### 3) Install edgeplane (one-time per update)

```bash
bash scripts/install-edgeplane.sh
```

By default installs to `~/.local/bin/edgeplane`. Ensure `~/.local/bin` is on `PATH`.

### 4) Start Rust Daemon (every session)

```bash
edgeplane daemon --shim-host 127.0.0.1 --shim-port 8765
```

Or via the convenience script:

```bash
bash scripts/start-edgeplane-daemon.sh
```

### 5) Add MCP Server to Your Agent

Default shim-mode config (works for Claude Code, Gemini CLI, and others supporting `mcpServers`):

```json
{
  "edgeplane": {
    "command": "edgeplane",
    "args": ["serve"],
    "env": {
      "EP_BASE_URL": "https://edgeplane.example.com",
      "EP_AGENT_TOKEN": "<your-token>"
    }
  }
}
```

Codex TOML format (`~/.codex/config.toml`):

```toml
[mcp_servers.edgeplane]
command = "edgeplane"
args = ["serve"]
startup_timeout_sec = 45
tool_timeout_sec = 60
env = { EP_BASE_URL = "https://edgeplane.example.com", EP_AGENT_TOKEN = "<your-token>" }
```

Gemini CLI (`~/.gemini/settings.json`):

```json
{
  "mcpServers": {
    "edgeplane": {
      "command": "edgeplane",
      "args": ["serve"],
      "env": {
        "EP_BASE_URL": "https://edgeplane.example.com",
        "EP_AGENT_TOKEN": "<your-token>"
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

## Skill Sync (Domain/Mission Scope)

Resolve and materialize effective skills for an active domain/mission:

```bash
edgeplane data sync status --domain-id <domain-id> --mission-id <optional-mission-id>
```

---

## Auth Reference

| Auth type | How it works | Recommended for |
|---|---|---|
| Static `EP_AGENT_TOKEN` | Shared secret, never expires | Local dev, CI |
| Session token (`ep_*`) | DB-backed, revocable, expiring | Interactive use, OIDC users |
| OIDC JWT | Short-lived, identity-bound | SSO/Authentik environments |

All auth types work with `edgeplane run` for gemini/openclaw/custom. Claude/codex are native runtimes under the same `edgeplane run` command.

## Troubleshooting: Startup Timeout

If Codex shows `MCP startup incomplete (failed: edgeplane)`:

- Ensure `edgeplane daemon` is running on `127.0.0.1:8765`.
- Use shim defaults (`EP_MCP_MODE=shim`, `EP_STARTUP_PREFLIGHT=none`).
- Ensure your MCP env vars use the `EP_*` prefix.
- Run `edgeplane auth whoami` to verify auth is working before launching an agent.
