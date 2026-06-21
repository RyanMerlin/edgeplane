<p align="center">
  <img src="edgeplane/assets/edgeplane-git-hero-img.png" alt="Edgeplane" width="100%">
</p>

# EdgePlane

> **Kubernetes orchestrates containers. EdgePlane orchestrates agents.**

AI agents can write code, run tools, and reason over architecture. What they cannot do is coordinate. Without a shared system of record, parallel agents duplicate effort, diverge on state, and collide on artifacts with no resolution path. The capability compounds; the coordination does not.

EdgePlane is the coordination layer: a control plane for AI agents and the humans working alongside them. It is not a workflow runner, not a pipeline framework, and not a chatbot UI. It gives agents structured domains, durable task ownership, overlap detection before mutations, HMAC-signed governance, and a three-tier persistence model spanning Postgres, S3, and Git.

**The MCP server is the control-plane boundary.** Agents interact through standard MCP stdio tools, with no sidecar, no custom SDK, and no separate auth token. The `edgeplane` CLI is a single compiled Rust binary, and `edgeplaned` keeps long-running agent sessions alive across crashes and reconnects through ACP (Agent Client Protocol). It works with Claude, Codex, Gemini, or any MCP-compatible runtime.

## Core Capabilities

- **Domains & Missions** — organizational units that scope knowledge, tools, permissions, and governance. Agents and humans switch profiles without losing context or integrity.
- **Overlap Detection** — fuzzy and vector similarity run before task and artifact creation. Collisions surface as `overlap_suggestions` in the API response, before damage occurs.
- **Governance & Approvals** — versioned policy lifecycle (draft → active → rollback), role-based access (Admin / Contributor / Viewer), and HMAC-signed approval tokens on sensitive mutations.
- **Three-Tier Persistence** — structured state in Postgres with pgvector for semantic and hybrid search, artifact content in S3-compatible object storage, and a Git-committed artifact ledger carrying full provenance on publish.
- **Persistent Sessions** — `edgeplaned` manages agent processes per node via ACP. Remote attach through the web UI renders structured conversation (assistant turns, tool calls, permission prompts), not raw terminal output.

See the [documentation](https://edgeplane.ai/concepts/overview/) for the full capability set, including chat integration, semantic search, and the entity model.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│              AI Agents  (Claude, Codex, custom)              │
└──────────────────────────────┬───────────────────────────────┘
                               │
               ┌───────────────▼─────────────────┐
               │               edgeplane                │
               │     MCP stdio bridge · Rust     │
               │        cargo install edgeplane         │
               │  tools/list · tools/call · CLI  │
               └───────────────┬─────────────────┘
                               │  HTTP
┌──────────────────────────────▼───────────────────────────────┐
│                     EdgePlane API                       │
│                       Axum  ·  MQTT                          │
├─────────────────┬──────────────────────┬─────────────────────┤
│  Domains &      │  Tasks · Overlap     │  Governance &       │
│  Missions       │  Detection · Semantic│  Approvals          │
│                 │  Search              │  Slack / ChatOps    │
└─────────────────┴──────────────────────┴─────────────────────┘
         │                    │                      │
         ▼                    ▼                      ▼
┌─────────────────┐  ┌─────────────────┐  ┌──────────────────┐
│   PostgreSQL    │  │   S3 / RustFS   │  │      Git         │
│   + pgvector    │  │  Object Store   │  │                  │
│                 │  │                 │  │  Artifact ledger │
│  Structured     │  │  Artifact       │  │  long-term       │
│  state · roles  │  │  content ·      │  │  memory of       │
│  vector index   │  │  large objects  │  │  record          │
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
- API docs (Swagger): `http://localhost:8008/api/docs`
- Web UI: `http://localhost:8008/ui/`

## Quick Links

| | |
|---|---|
| Docker full stack | `bash scripts/dev-up.sh` |
| Install edgeplane CLI | `bash scripts/install-edgeplane.sh` or `.\scripts\install-edgeplane.ps1` |
| Bootstrap edgeplane (curl) | `curl -fsSL https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/bootstrap-edgeplane.sh \| bash` |
| Philosophy & vision | [EDGEPLANE_PHILOSOPHY.md](PHILOSOPHY.md) |
| API reference | `/api/docs` (Swagger UI) |
| Agent install guide | [docs/guides/AGENT-INSTALL.md](docs/guides/AGENT-INSTALL.md) |
| Web UI (React 19 + Vite) | [web/README.md](web/README.md) |
| Persistent sessions | [docs/plans/edgeplaned-persistent-session-architecture.md](docs/plans/edgeplaned-persistent-session-architecture.md) |

## Running with Docker (Recommended)

Full stack (Postgres + pgvector + MQTT + RustFS):

```bash
bash scripts/dev-up.sh        # start
bash scripts/smoke.sh --profile full   # validate
bash scripts/dev-down.sh      # stop
```

Quickstart (SQLite + Chroma — no external deps):

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

`edgeplane auth login` issues a server-managed session token (`mcs_*`) — revocable, never stored in agent config files, auto-loaded on next run:

```bash
edgeplane auth login       # interactive OIDC (default)
edgeplane auth login --with-token   # prompt for an mcs_sa_* service-account token
edgeplane run claude       # session auto-loaded
edgeplane auth whoami
edgeplane auth logout
```

For CI/non-interactive environments, set `EP_AGENT_TOKEN` to an `mcs_sa_*` service-account token and pass `--non-interactive`:

```bash
EP_AGENT_TOKEN="mcs_sa_..." edgeplane auth login --non-interactive
```

Pass `--preflight-only` to validate connectivity without launching.

See [docs/guides/AGENT-INSTALL.md](docs/guides/AGENT-INSTALL.md) for session tokens, Codex swarm workflows, and skill sync.

## MCP

```bash
# List available tools
curl http://localhost:8008/mcp/tools

# Search tasks
curl -X POST http://localhost:8008/mcp/call \
  -H "Content-Type: application/json" \
  -d '{"tool":"search_tasks","args":{"query":"overlap detection","limit":5}}'

# Load mission workspace
curl -X POST http://localhost:8008/mcp/call \
  -H "Content-Type: application/json" \
  -d '{"tool":"load_mission_workspace","args":{"mission_id":"<mission-id>"}}'
```

## Persistence Model

Three layers, each with a distinct role:

| Layer | Role | When written |
|-------|------|-------------|
| PostgreSQL + pgvector | Structured state, ownership, vector index | Every mutation |
| S3 / RustFS | Artifact content, large objects, working files | On create/update |
| Git | Memory of record, provenance, audit trail | On publish/approval |

Publication is policy-routed. Configure repository targets via `/persistence/connections` and `/persistence/bindings`. Resolve targets before publish with MCP `resolve_publish_plan`. All responses include `x-request-id` for correlation.

## Governance

Policy is DB-backed and versioned (`draft` → `active` → rollback). The Admin UI tab at `/ui` supports viewing, editing, and publishing policy. Conservative preset: `EP_GOV_PROFILE=production`.

See the [governance & approvals guide](https://edgeplane.ai/guides/governance-and-approvals/) for the full env var reference.

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

## Contributing

- [CONTRIBUTING.md](CONTRIBUTING.md)
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- [SECURITY.md](SECURITY.md)
- [GOVERNANCE.md](GOVERNANCE.md)
- [LICENSE](LICENSE) — Apache-2.0
