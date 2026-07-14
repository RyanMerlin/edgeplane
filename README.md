<p align="center">
  <img src="[edgeplane/assets/edgeplane-git-hero-img.png](https://github.com/RyanMerlin/edgeplane/blob/1c7b17682c8b1f16095e7545c264af4bbf9eb568/assets/edgeplane-git-hero-img.png)" alt="Edgeplane" width="100%">
</p>

# EdgePlane

> **Kubernetes orchestrates containers. EdgePlane orchestrates agents.**

> **Status — alpha (0.x).** Under active development; the API surface changes between minor versions. This README describes what runs **today** — see [Status & Roadmap](#status--roadmap) for what's built vs. planned.

AI agents can write code, run tools, and reason over architecture. What they cannot do is coordinate. Without a shared system of record, parallel agents duplicate effort, diverge on state, and collide on artifacts with no resolution path. The capability compounds; the coordination does not.

EdgePlane is the coordination layer: a control plane for AI agents and the humans working alongside them. It is not a workflow runner, not a pipeline framework, and not a chatbot UI. It gives agents structured domains, durable task ownership, membership-based authorization, and a Git-committed artifact ledger that carries full provenance on publish — all reachable through a standard MCP interface.

**The MCP server is the control-plane boundary.** Agents interact through standard MCP stdio tools, with no sidecar, no custom SDK, and no separate auth token. The `edgeplane` CLI is a single compiled Rust binary, and `edgeplaned` supervises long-running agent sessions per node — with mid-session remote attach and event replay through ACP (Agent Client Protocol). It works with Claude, Codex, Gemini, or any MCP-compatible runtime.

## Core Capabilities

- **Domains, Missions & Tasks** — organizational units that scope knowledge, tools, and membership. Agents and humans switch profiles without losing context, and every task carries durable ownership with claim coordination.
- **Membership-based authorization** — default-deny access keyed on per-domain `owners` / `contributors`, plus an admin allowlist (`EP_ADMIN_EMAILS` / `EP_ADMIN_GROUPS`). Enforced on the MCP and HTTP surfaces alike. (A versioned policy/approvals engine is on the [roadmap](#status--roadmap).)
- **Git artifact ledger** — on publish, artifacts are committed to a configured Git target with full provenance (`published_by`, `published_at`, publication metadata), routed via versioned persistence bindings.
- **Persistent sessions** — `edgeplaned` supervises agent processes per node via ACP. Remote attach through the web UI renders structured conversation (assistant turns, tool calls, permission prompts), not raw terminal output — with a replay buffer so mid-session attachers catch up.
- **MCP-native** — everything above is reachable through standard MCP stdio tools: no sidecar, no custom SDK, no per-agent token.

See the [documentation](https://edgeplane.ai/concepts/overview/) for the full capability set, including chat integration, the persistence model, and the entity model.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│              AI Agents  (Claude, Codex, custom)              │
└──────────────────────────────┬───────────────────────────────┘
                               │
               ┌───────────────▼─────────────────┐
               │           edgeplane             │
               │     MCP stdio bridge · Rust     │
               │     cargo install edgeplane     │
               │  tools/list · tools/call · CLI  │
               └───────────────┬─────────────────┘
                               │  HTTP
┌──────────────────────────────▼───────────────────────────────┐
│                       EdgePlane API                          │
│                       Axum HTTP API                          │
├─────────────────┬──────────────────────┬─────────────────────┤
│  Domains &      │  Tasks · Claims      │  Authorization      │
│  Missions       │  Publish · Ledger    │  Slack / ChatOps    │
│                 │                      │                     │
└─────────────────┴──────────────────────┴─────────────────────┘
         │                    │                      │
         ▼                    ▼                      ▼
┌─────────────────┐  ┌─────────────────┐  ┌──────────────────┐
│   PostgreSQL    │  │   S3 / RustFS   │  │      Git         │
│   + pgvector    │  │  Object Store   │  │                  │
│                 │  │                 │  │  Artifact ledger │
│  Structured     │  │  Artifact       │  │  long-term       │
│  state · roles  │  │  content ·      │  │  memory of       │
│  artifacts      │  │  large objects  │  │  record          │
│  status · collab│  │  file persist.  │  │                  │
└─────────────────┘  └─────────────────┘  └──────────────────┘
                               │
                 ┌─────────────▼──────────────┐
                 │        Human Team          │
                 │    Slack · Teams · Web UI  │
                 └────────────────────────────┘
```

**edgeplaned** is the node daemon — analogous to kubelet. It runs on every node, registers with edgeplane-tower, and manages all agent processes on that node. `edgeplane` is kubectl: the CLI surface for humans and agents.

## Quick Start

Install the `edgeplane` CLI:

```bash
curl -fsSL https://edgeplane.ai/install.sh | bash
```

Windows:
```powershell
irm https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/bootstrap-edgeplane.ps1 | iex
```

Then bring up the full stack locally:

```bash
bash scripts/dev-up.sh
edgeplane auth login          # OIDC or token prompt
edgeplane system doctor
```

Then open:
- Web UI: `http://localhost:8008/`
- OpenAPI spec: committed at `web/openapi.json` (used for typed client generation)

## Quick Links

| | |
|---|---|
| Docker full stack | `bash scripts/dev-up.sh` |
| Install edgeplane CLI | `bash scripts/install-edgeplane.sh` or `.\scripts\install-edgeplane.ps1` |
| Bootstrap edgeplane (curl) | `curl -fsSL https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/bootstrap-edgeplane.sh \| bash` |
| Philosophy & vision | [EDGEPLANE_PHILOSOPHY.md](PHILOSOPHY.md) |
| API reference | OpenAPI spec at `web/openapi.json` |
| Agent install guide | [docs/guides/AGENT-INSTALL.md](docs/guides/AGENT-INSTALL.md) |
| Web UI (React 19 + Vite) | [web/README.md](web/README.md) |
| Persistent sessions | [docs/plans/edgeplaned-persistent-session-architecture.md](docs/plans/edgeplaned-persistent-session-architecture.md) |

## Running with Docker (Recommended)

Full stack (Postgres/pgvector + RustFS object storage + tower API + MCP daemon):

```bash
bash scripts/dev-up.sh        # start
bash scripts/smoke.sh --profile full   # validate
bash scripts/dev-down.sh      # stop
```

Quickstart (self-contained — Postgres + RustFS with baked-in dev credentials, no `.env` setup needed):

```bash
EP_STACK_PROFILE=quickstart bash scripts/dev-up.sh
```

Object storage is available locally at `http://localhost:9000` (S3 API) and `http://localhost:9001` (console). To use an external backend instead, set `EP_OBJECT_STORAGE_*` env vars — see `.env.example`.

## Running Natively (Rust)

```bash
cp .env.example .env
cd crates/edgeplane-tower
cargo build --release
set -a; source .env; set +a
./target/release/edgeplane-tower
```

Frontend (React 19 + Vite, in `web/`): `cd web && npm install && npm run dev` (Vite dev server on :5173, proxies `/api` → :8008).

## Agent Integration

### Install edgeplane

```bash
# Linux / macOS
bash <(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/bootstrap-edgeplane.sh)

# Windows
irm https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/bootstrap-edgeplane.ps1 | iex
```

### Launch an agent

```bash
export EP_BASE_URL="https://your-edgeplane.example.com"
edgeplane auth login       # OIDC browser flow (or --with-token for API token prompt)

edgeplane run claude       # Claude Code
edgeplane run codex        # OpenAI Codex CLI
edgeplane run gemini       # Google Gemini CLI
```

### Auth

`edgeplane auth login` issues a server-managed session token (`ep_*`) — revocable, never stored in agent config files, auto-loaded on next run:

```bash
edgeplane auth login       # interactive OIDC (default)
edgeplane auth login --with-token   # prompt for an ep_sa_* service-account token
edgeplane run claude       # session auto-loaded
edgeplane auth whoami
edgeplane auth logout
```

For CI/non-interactive environments, set `EP_AGENT_TOKEN` to an `ep_sa_*` service-account token and pass `--non-interactive`:

```bash
EP_AGENT_TOKEN="ep_sa_..." edgeplane auth login --non-interactive
```

Pass `--preflight-only` to validate connectivity without launching.

See [docs/guides/AGENT-INSTALL.md](docs/guides/AGENT-INSTALL.md) for session tokens, Codex swarm workflows, and skill sync.

## MCP

```bash
# List available tools
curl http://localhost:8008/api/mcp/tools

# Search tasks
curl -X POST http://localhost:8008/api/mcp/call \
  -H "Content-Type: application/json" \
  -d '{"tool":"search_tasks","args":{"query":"authorization","limit":5}}'

# Load mission workspace
curl -X POST http://localhost:8008/api/mcp/call \
  -H "Content-Type: application/json" \
  -d '{"tool":"load_mission_workspace","args":{"mission_id":"<mission-id>"}}'
```

## Persistence Model

Three layers, each with a distinct role:

| Layer | Role | When written |
|-------|------|-------------|
| PostgreSQL | Structured state, ownership, artifact content | Every mutation |
| Git | Memory of record, provenance, audit trail | On publish |
| S3 / RustFS | Object storage for large artifacts (*roadmap — see [Status & Roadmap](#status--roadmap)*) | — |

Publication is policy-routed. Configure repository targets via `/persistence/connections` and `/persistence/bindings`, and resolve targets before publish with MCP `resolve_publish_plan`.

## Authorization

Access control is membership-based and default-deny: each domain has `owners` and `contributors`, plus an admin allowlist (`EP_ADMIN_EMAILS` / `EP_ADMIN_GROUPS`). Checks are enforced on both the HTTP and MCP surfaces. There is currently **no** separate policy/approval engine — a versioned governance lifecycle is on the [roadmap](#status--roadmap).

See the [authorization guide](https://edgeplane.ai/guides/governance-and-approvals/) for details.

## Migrations

Migrations run automatically on startup via sqlx. To run manually:

```bash
cd crates/edgeplane-tower && sqlx migrate run
```

Migration files: `crates/edgeplane-tower/migrations/`

## Tests

```bash
bash scripts/dev-up.sh
bash scripts/smoke.sh --profile full
```

## Status & Roadmap

EdgePlane is **alpha (0.x)** and under active development. The honest split:

**Working today**
- MCP-native control plane — standard stdio tools, no sidecar / SDK / per-agent token
- Domains, Missions & Tasks with durable task ownership and claim coordination
- Membership-based, default-deny authorization (owners/contributors + admin allowlist)
- Persistent agent sessions via `edgeplaned` + ACP: supervise, remote-attach, event replay
- Git-committed artifact ledger with provenance; policy-routed publish targets
- Automatic sqlx migrations on startup; React web UI; Slack / Teams / Google Chat integration hooks

**Planned / not yet implemented**
- **Overlap detection** — the schema exists, but similarity analysis is not yet wired; treat as advisory-only for now
- **Governance & approvals engine** — versioned policy lifecycle and signed approval tokens (removed pending redesign)
- **Role model** beyond owners/contributors (e.g. Admin / Contributor / Viewer)
- **Semantic / hybrid (vector) search** over structured state
- **Object-storage (S3) artifact content** — artifact bodies currently live in Postgres
- **Session context recovery across a daemon crash** — reconnect + event replay work; full context restore does not

We'd rather ship a smaller true surface than a larger claimed one. If you hit a gap between the docs and the code, that's a bug — please file it.

## Contributing

- [CONTRIBUTING.md](CONTRIBUTING.md)
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- [SECURITY.md](SECURITY.md)
- [GOVERNANCE.md](GOVERNANCE.md)
- [LICENSE](LICENSE) — Apache-2.0
