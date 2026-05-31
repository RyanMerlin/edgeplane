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

Interactive OIDC (recommended):
```bash
export EP_BASE_URL="https://your-edgeplane.example.com"
edgeplane auth login       # browser flow — issues a session token
```

Or with a service-account token (CI/non-interactive):
```bash
export EP_BASE_URL="https://your-edgeplane.example.com"
EP_AGENT_TOKEN="mcs_sa_..." edgeplane auth login --non-interactive
```

**Step 3 — Launch your agent:**

```bash
edgeplane run claude           # Claude Code
edgeplane run codex            # OpenAI Codex CLI
edgeplane run gemini           # Google Gemini CLI
edgeplane launch openclaw      # OpenClaw
edgeplane launch custom        # Custom ACP agent
```

That's it. `edgeplane run <runtime>` is the unified launch path. `edgeplane launch` remains for openclaw/custom and legacy compatibility.

Codex quick checks:
```bash
edgeplane run codex status             # read-only quick status (human)
edgeplane run codex status --json      # read-only quick status (machine)
edgeplane run codex doctor --json      # detailed readiness diagnostics (machine)
```

---

## What `edgeplane launch` does

1. Checks agent binary is on PATH (with install hint if not)
2. Validates profile/session context and pin policy (when configured)
3. Validates auth against the MC API
4. Fetches agent config from the onboarding manifest
5. Writes config to an instance-local runtime home by default (token not embedded if using session)
6. Injects `EP_AGENT_TOKEN` into the agent's process environment
7. exec's the agent

## `edgeplane launch` flags (non-Claude/Codex agents)

| Flag | Effect |
|---|---|
| `--preflight-only` | Validate env + auth without launching (CI-safe) |
| `--no-daemon` | Skip daemon management (daemon externally managed) |
| `--skip-config-gen` | Use existing config, skip manifest fetch |
| `--no-embed-token` | Omit `EP_AGENT_TOKEN` from written config file (auto-implied for session tokens) |
| `--legacy-global-config` | Write config to global agent paths (`~/.codex`, `~/.gemini`) for compatibility |
| `--daemon-timeout N` | Seconds to wait for daemon ready (default: 15) |
| `-- <args>` | Pass remaining args verbatim to the agent |

## Agent Config Locations (default)

| Agent | Config written by `edgeplane launch` |
|---|---|
| Gemini CLI | `~/.edgeplane/instances/<runtime_session_id>/home/.gemini/settings.json` |
| OpenClaw | `~/.edgeplane/instances/<runtime_session_id>/edgeplane/config/openclaw.acp.json` |
| Custom ACP agent | `~/.edgeplane/instances/<runtime_session_id>/edgeplane/config/custom.acp.json` |

Use `--legacy-global-config` only when you explicitly need legacy global config writes.

---

## Session tokens

`edgeplane auth login` exchanges your current credentials for a server-issued session token
(`mcs_*` prefix) stored at `~/.edgeplane/session.json` (chmod 600).

Session tokens are:
- **Revocable** — `edgeplane auth logout` revokes server-side instantly
- **Never written to agent config files** — injected into the agent process at exec time only
- **Auto-loaded** — `edgeplane` reads `session.json` automatically when `EP_AGENT_TOKEN` is not set
- **Expiring** — default 8h TTL, configurable with `--ttl-hours` (max 720h / 30 days)

### Login / logout / whoami

```bash
# Create a session (exchange any valid credential for an mcs_ token)
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

# Interactive OIDC (recommended)
edgeplane auth login

# Non-interactive / CI: bootstrap a session from a service-account token
EP_AGENT_TOKEN="mcs_sa_..." edgeplane auth login --non-interactive

# From now on — no EP_AGENT_TOKEN needed in env
edgeplane run claude   # session loaded from ~/.edgeplane/session.json
edgeplane run codex    # token injected into agent process at exec, not written to config
edgeplane auth whoami          # verify identity
edgeplane auth logout          # revoke when done
```

### OIDC / short-lived JWTs

When you authenticate via OIDC (Authentik, SSO), the OIDC flow issues a session token directly.
For scripted flows, set `EP_AGENT_TOKEN` to an `mcs_sa_*` service-account token:

```bash
# Exchange service-account token for a longer-lived edgeplane session token
EP_AGENT_TOKEN="mcs_sa_..." edgeplane auth login --non-interactive --ttl-hours 8
edgeplane run claude
```

**Token embedding rules in `edgeplane launch` (non-Claude/Codex agents):**
- Session tokens (`mcs_*`) → never embedded, always injected at exec time
- `--no-embed-token` flag → never embedded
- `EP_AGENT_TOKEN` absent → never embedded (auto-implied, notice printed)
- Service-account token present → embedded by default (can override with `--no-embed-token`)

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
edgeplane auth login       # OIDC browser flow, or --with-token for mcs_sa_* service-account token
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
      "EP_BASE_URL": "https://edgeplane.example.com"
    }
  }
}
```

Session token is injected automatically by `edgeplane launch`. For static service-account tokens, add `EP_AGENT_TOKEN`.

Codex TOML format (`~/.codex/config.toml`):

```toml
[mcp_servers.edgeplane]
command = "edgeplane"
args = ["serve"]
startup_timeout_sec = 45
tool_timeout_sec = 60
env = { EP_BASE_URL = "https://edgeplane.example.com" }
```

Gemini CLI (`~/.gemini/settings.json`):

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
| OIDC session | `edgeplane auth login` browser flow, DB-backed revocable token | Interactive use, operators |
| Service-account token (`mcs_sa_*`) | Long-lived, programmatic; set `EP_AGENT_TOKEN` | CI/CD, daemons |
| Session token (`mcs_*`) | Issued after login; injected at exec time, never written to config | Default after any login |

All auth types work with `edgeplane launch` for gemini/openclaw/custom. Codex/Claude use the dedicated command families.

## Troubleshooting: Startup Timeout

If Codex shows `MCP startup incomplete (failed: edgeplane)`:

- Ensure `edgeplane daemon` is running on `127.0.0.1:8765`.
- Use shim defaults (`EP_MCP_MODE=shim`, `EP_STARTUP_PREFLIGHT=none`).
- Ensure your MCP env vars use the `EP_*` prefix.
- Run `edgeplane auth whoami` to verify auth is working before launching an agent.
