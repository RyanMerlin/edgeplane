---
title: Component Reference
description: What each EdgePlane binary owns, how to configure it, and where its trust boundary sits.
---

EdgePlane is composed of five components. Three are core binaries that must cooperate; two are optional extensions for specific environments.

---

## edgeplane (CLI / TUI)

**Role:** Primary operator and scripted interface. Stateless — reads a session token from disk and delegates all mutations to edgeplane-tower.

**Responsibilities:**
- `edgeplane tui` — full-screen terminal UI (agents, domains, missions, feed, approvals, secrets, config)
- `edgeplane run <runtime>` — unified agent launcher (claude, codex, goose, gemini, custom)
- `edgeplane auth login / logout / whoami` — OIDC session token lifecycle
- `edgeplane capabilities` — capability pack discovery and dispatch
- `edgeplane domain`, `edgeplane mission`, `edgeplane task`, `edgeplane agent` — entity CRUD
- `edgeplane health` — connectivity check and server status

**Trust boundary:** Runs with operator-level OS privileges. Holds a session token in `~/.edgeplane/session.json` (should be chmod 600). Has no privileged access to the database or secrets beyond what the session token grants.

**Key config:**

| Variable / file | Purpose | Default |
|-----------------|---------|---------|
| `EP_BASE_URL` | edgeplane-tower base URL | `http://localhost:8008` |
| `~/.edgeplane/session.json` | Session token (written by `edgeplane auth login`) | — |
| `EP_HOME` | Override base data dir | `~/.edgeplane` |

---

## edgeplane-tower (Control Plane)

**Role:** The Axum HTTP server that owns all shared state. Single source of truth for domains, missions, tasks, artifacts, governance, and agent registrations.

**Responsibilities:**
- REST API under `/api/*` — full CRUD for all entities
- Server-Sent Events (SSE) at `/api/ai/sessions/{id}/stream` — real-time event streaming per session
- WebSocket attach proxy at `/api/runtime/nodes/{node_id}/agents/{agent_id}/attach` — browser → tower → edgeplaned PTY
- OIDC authentication (`/api/auth/oidc/login`, `/api/auth/oidc/callback`)
- Governance enforcement — policy lifecycle, approval tokens, domain membership checks
- Artifact ledger — S3 streaming, vector indexing of artifact content, SHA-256 provenance
- Automatic schema migrations on startup
- Serving the React web dashboard (static files bundled into the binary)

**Trust boundary:** Authoritative. All writes route through here. OIDC JWTs and node JWTs are validated here. No agent writes to the database directly.

**Key config:**

| Variable | Purpose |
|----------|---------|
| `DATABASE_URL` | Postgres connection string |
| `S3_ENDPOINT` / `S3_BUCKET` / `S3_ACCESS_KEY` / `S3_SECRET_KEY` | Artifact storage |
| OIDC client credentials (provider-specific) | Browser auth flow |
| `--bind` flag | Bind address (default `0.0.0.0:8008`) |

```bash
edgeplane-tower --bind 0.0.0.0:8008
```

---

## edgeplaned (Node Daemon)

**Role:** Headless executor on each operator node. Manages agent processes and brokers secrets. Operators never interact with it directly — use `edgeplane daemon …` CLI subcommands or the TUI.

Think of the relationship as: `edgeplane` = kubectl (operator interface), `edgeplaned` = kubelet (node executor).

**Responsibilities:**
- Agent subprocess lifecycle — spawn, restart, crash recovery (watchdog per supervisor)
- Secrets broker — agents receive `EP_SECRETS_SOCKET` and `EP_SECRETS_SESSION` env vars; raw credentials never leave the daemon
- PTY bridge for Zellij-hosted agents — gateway between edgeplane-tower attach WebSocket and the agent's terminal
- Cron dispatch — reads `~/.edgeplane/edgeplaned/cron.toml`, fires scheduled jobs
- Node registration — persists node JWT to `/etc/edgeplane/node.json` via `edgeplane agent node register`
- Task worker loops — claim loop and triage loop for distributed mesh task execution

**Trust boundary:** Holds a machine-identity node JWT (`/etc/edgeplane/node.json`). Acts on behalf of the agents it manages. Cannot escalate privileges beyond what edgeplane-tower's node identity grants.

**Socket paths** (under `~/.edgeplane/edgeplaned/`):

| Socket | Protocol | Purpose |
|--------|----------|---------|
| `mgmt.sock` | JSON-RPC 2.0 | Management gateway (CLI ↔ daemon) |
| `secrets.sock` | Line-delimited JSON | Secrets broker (agent subprocesses only) |
| `edgeplaned.sock` | WebSocket / PTY | PTY attach gateway |

**Key config:**

| Variable / file | Purpose |
|-----------------|---------|
| `EP_BACKEND_URL` | edgeplane-tower base URL |
| `/etc/edgeplane/node.json` | Node JWT (written by `edgeplane agent node register`) |
| `~/.edgeplane/edgeplaned/config.yaml` | Daemon config (work dir, agent profiles) |
| `~/.edgeplane/edgeplaned/cron.toml` | Scheduled job definitions |

```bash
# Register this machine as an EdgePlane node
edgeplane agent node register --join-token <TOKEN> --endpoint https://edgeplane.example.com

# Run the daemon
edgeplaned run --backend-url https://edgeplane.example.com
```

---

## Web Dashboard

**Role:** React SPA served by edgeplane-tower. Communicates exclusively through tower REST, SSE, and WebSocket APIs — no direct database access.

**Tabs and capabilities:**
- **Fleet** — live agent grid, status, ACP terminal attach sessions
- **Domains** — domain → mission → task tree, task create/edit/close
- **Governance** — policy lifecycle, pending approvals, approval actions
- **AI Console** — direct AI conversation within a session
- **Feed** — live SSE event stream across all entities

**Auth:** OIDC browser flow → session cookie (`ep_session_token`, HttpOnly). Display name is taken from the `preferred_username` OIDC claim (falls back to `name`); single-word names render as one initial, multi-part names as first + last.

**Trust boundary:** Same as any authenticated user session. WebSocket attach is owner-scoped — a user can only attach to agents they own or have been granted access to.

---

## edgeplane-zrpc (Optional Zellij Plugin)

**Role:** WASM plugin for the Zellij terminal multiplexer. Optional and feature-flagged — if `EDGEPLANE_ZRPC_PLUGIN_PATH` is not set, nothing changes.

**What it adds for Zellij-hosted agents:**
- Focus-free PTY injection — send input to a pane without stealing focus
- Scrollback reads — read pane output without a PTY race
- Pane lifecycle events — spawn/close notifications over the `zrpc-events` Zellij pipe
- Cancel signal delivery

**Feature flag:** Set `EDGEPLANE_ZRPC_PLUGIN_PATH` to the path of the compiled `.wasm` file. edgeplaned installs the plugin config and pre-seeds `permissions.kdl` on startup.

**Build note:** The plugin must be compiled as a Rust **bin crate** (not `cdylib`). Zellij 0.44.x uses the WASI command model and requires a `_start` export; a `cdylib` on `wasm32-wasip1` compiles to a WASI reactor with no `_start`, loads silently, but fails to instantiate — every `zellij pipe` call to it hangs with no error output.

---

## What's Next

- [Data Flow](/architecture/data-flow/) — how a task moves from submission to artifact publication
- [Security Model](/architecture/security/) — authentication, authorization, and audit trail
- [Guides: Zellij Integration](/guides/zellij-integration/) — enable and configure edgeplane-zrpc
