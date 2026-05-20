<p align="center">
  <img src="assets/mc-hero-image-2.png" alt="MissionControl" width="100%">
</p>

# MissionControl

> Kubernetes orchestrates containers. MissionControl orchestrates agents, missions, and knowledge.

AI agents can write code, run tools, and reason over architecture. What they can't do is coordinate. Without a shared system of record, parallel agents duplicate effort, diverge on state, and collide on artifacts with no resolution path.

MissionControl is a control plane for AI agents and human collaborators. It provides structured missions, durable task ownership, overlap detection before mutations, HMAC-signed governance, and a three-tier persistence model (Postgres + S3 + Git). The `mc` CLI is a compiled Rust binary. Agents interact via standard MCP stdio — no custom SDK required.

## Core Capabilities

- **Missions & Klusters** — organizational units that scope knowledge, tools, permissions, and governance. Agents and humans switch profiles without losing context or integrity.
- **Overlap Detection** — fuzzy + vector similarity runs before task and artifact creation. Collisions surface as `overlap_suggestions` in the API response before damage occurs.
- **Artifact Ledger** — every mutation recorded in Postgres, vector-indexed for search, and committed to Git with full provenance metadata on publish.
- **MCP-Native Interface** — standard MCP stdio tools: `search_tasks`, `get_overlap_suggestions`, `load_kluster_workspace`, `publish_pending_ledger_events`. Works with any MCP-compatible agent.
- **Governance & Approvals** — versioned policy lifecycle (draft → active → rollback), role-based access (Admin / Contributor / Viewer), HMAC-signed approval tokens on sensitive mutations.
- **Persistent Agent Sessions** — `mcd` manages long-running agent processes on each node via ACP (Agent Client Protocol). Sessions survive crashes and reconnects. Remote attach via the web UI renders structured conversation — assistant turns, tool calls, permission prompts — not raw terminal output.
- **Semantic Search** — tasks, docs, and klusters are vector-indexed (pgvector) for similarity and hybrid search.
- **S3-Backed File Persistence** — artifact content stored in S3-compatible object storage. RustFS is bundled in the Docker Compose stack. Swap in AWS S3 or MinIO with env vars — no code changes.
- **Chat Integration** — Slack-native notifications, task creation from threads, approval workflows, and in-channel search. Teams and Google Chat provider skeletons included.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│              AI Agents  (Claude, Codex, custom)              │
└──────────────────────────────┬───────────────────────────────┘
                               │
               ┌───────────────▼─────────────────┐
               │               mc                │
               │     MCP stdio bridge · Rust     │
               │        cargo install mc         │
               │  tools/list · tools/call · CLI  │
               └───────────────┬─────────────────┘
                               │  HTTP
┌──────────────────────────────▼───────────────────────────────┐
│                     MissionControl API                       │
│                       Axum  ·  MQTT                          │
├─────────────────┬──────────────────────┬─────────────────────┤
│  Missions &     │  Tasks · Overlap     │  Governance &       │
│  Klusters       │  Detection · Semantic│  Approvals          │
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
│  vector index   │  │  skill bundles  │  │  record          │
│  status · collab│  │  file persist.  │  │                  │
└─────────────────┘  └─────────────────┘  └──────────────────┘
                               │
                 ┌─────────────▼──────────────┐
                 │        Human Team          │
                 │    Slack · Teams · Web UI  │
                 └────────────────────────────┘
```

**mcd** is the node daemon — analogous to kubelet. It runs on every node, registers with mc-controlplane, and manages all agent processes on that node. `mc` is kubectl: the CLI surface for humans and agents.

## Quick Start

Install the `mc` CLI:

```bash
curl -fsSL https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.sh | bash
```

Windows:
```powershell
irm https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.ps1 | iex
```

Then bring up the full stack locally:

```bash
bash scripts/dev-up.sh
MC_TOKEN="TopSecret" mc system doctor
```

Then open:
- API docs (Swagger): `http://localhost:8008/api/docs`
- Web UI: `http://localhost:8008/ui/`

## Quick Links

| | |
|---|---|
| Docker full stack | `bash scripts/dev-up.sh` |
| Install mc CLI | `bash scripts/install-mc.sh` or `.\scripts\install-mc.ps1` |
| Bootstrap mc (curl) | `curl -fsSL https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.sh \| bash` |
| Philosophy & vision | [MISSIONCONTROL_PHILOSOPHY.md](MISSIONCONTROL_PHILOSOPHY.md) |
| API reference | `/api/docs` (Swagger UI) |
| Agent install guide | [docs/guides/AGENT-INSTALL.md](docs/guides/AGENT-INSTALL.md) |
| Web UI | [web/README.md](web/README.md) |
| Persistent sessions | [docs/plans/mcd-persistent-session-architecture.md](docs/plans/mcd-persistent-session-architecture.md) |

## Running with Docker (Recommended)

Full stack (Postgres + pgvector + MQTT + RustFS):

```bash
bash scripts/dev-up.sh        # start
bash scripts/smoke.sh --profile full   # validate
bash scripts/dev-down.sh      # stop
```

Quickstart (SQLite + Chroma — no external deps):

```bash
MC_STACK_PROFILE=quickstart bash scripts/dev-up.sh
```

Object storage is available locally at `http://localhost:9000` (S3 API) and `http://localhost:9001` (console). To use an external backend instead, set `MC_OBJECT_STORAGE_*` env vars — see `.env.example`.

## Running Natively (Rust)

```bash
cp .env.example .env
cd integrations/mc-controlplane
cargo build --release
set -a; source .env; set +a
./target/release/mc-controlplane
```

Frontend (SvelteKit): `cd web && npm install && npm run dev -- --host 0.0.0.0 --port 5173`

## Agent Integration

### Install mc

```bash
# Linux / macOS
bash <(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.sh)

# Windows
irm https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.ps1 | iex
```

### Launch an agent

```bash
export MC_TOKEN="<your-token>"
export MC_BASE_URL="https://your-mc.example.com"

mc run claude       # Claude Code
mc run codex        # OpenAI Codex CLI
mc run gemini       # Google Gemini CLI
```

### Auth

`mc auth login` exchanges a static token for a server-issued session token (`mcs_*`) — revocable, never stored in agent config files, auto-loaded on next run:

```bash
MC_TOKEN="<static-token>" mc auth login
mc run claude       # session auto-loaded
mc auth whoami
mc auth logout
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

# Load kluster workspace
curl -X POST http://localhost:8008/mcp/call \
  -H "Content-Type: application/json" \
  -d '{"tool":"load_kluster_workspace","args":{"kluster_id":"<kluster-id>"}}'
```

## Persistence Model

Three layers, each with a distinct role:

| Layer | Role | When written |
|-------|------|-------------|
| PostgreSQL + pgvector | Structured state, ownership, vector index | Every mutation |
| S3 / RustFS | Artifact content, skill bundles, working files | On create/update |
| Git | Memory of record, provenance, audit trail | On publish/approval |

Publication is policy-routed. Configure repository targets via `/persistence/connections` and `/persistence/bindings`. Resolve targets before publish with MCP `resolve_publish_plan`. All responses include `x-request-id` for correlation.

## Governance

Policy is DB-backed and versioned (`draft` → `active` → rollback). The Admin UI tab at `/ui` supports viewing, editing, and publishing policy. Conservative preset: `MC_GOV_PROFILE=production`.

See [docs/reference/GOVERNANCE.md](docs/reference/GOVERNANCE.md) for the full env var reference.

## Migrations

Migrations run automatically on startup via sqlx. To run manually:

```bash
cd integrations/mc-controlplane && sqlx migrate run
```

Migration files: `integrations/mc-controlplane/migrations/`

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
