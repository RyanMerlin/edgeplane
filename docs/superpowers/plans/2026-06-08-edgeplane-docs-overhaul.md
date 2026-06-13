# EdgePlane Docs Overhaul — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring edgeplane.ai documentation to full alignment with v0.13.x — fill the empty architecture section, add ACP/MeshTask/zRPC/Web UI coverage, update all stale Getting Started and Reference pages, and produce a top-tier OSS project doc site.

**Architecture:** Six parallel content agents each own a section (Getting Started, Concepts, Architecture, Guides, Reference+ADRs). A seventh agent does the consistency pass after all content is written. All files live in `site/src/content/docs/` inside `/home/merlin/code/edgeplane/`.

**Tech Stack:** Astro 5, `@astrojs/starlight` — Markdown/MDX pages, frontmatter `title`+`description` required on every page. Build with `cd site && bun run build`. Deploy target: edgeplane.ai (Cloudflare Pages, CI on push to main).

---

## Scrubbing Rules (every agent must apply these)

Before committing any page, verify:
- [ ] No homelab hostnames: `epyc`, `cloud0`, `kai`, `aria-memory-pg`, `*.ts.net`, `<your-tailnet>`
- [ ] No Tailscale addresses or VPN topology
- [ ] No personal Infisical paths: `/providers/`, `/aria-sa/`, `/infra/`
- [ ] No personal fleet profile names: `operator`, `research`, `merlinlabs`, `work`, `engineer`
- [ ] No vault/S3 bucket names
- [ ] No personal GitHub handle in code examples — use `<your-org>` or generic placeholder
- [ ] No `/workspace/cache/zellij` — replace with `$(zellij setup --check | grep 'CACHE DIR' | awk '{print $3}')`
- [ ] All code examples use generic hosts: `your-tower-host`, `https://edgeplane.example.com`, `<your-domain>`

---

## Source Material (all agents should read these before writing)

- `/home/merlin/code/edgeplane/CHANGELOG.md` — feature history, exact version numbers
- `/home/merlin/code/edgeplane/docs/architecture/entities.md` — canonical entity definitions (source of truth)
- `/home/merlin/code/edgeplane/docs/architecture/architecture.md` — system architecture overview
- `/home/merlin/code/edgeplane/docs/superpowers/specs/2026-06-08-edgeplane-docs-update-design.md` — the approved design spec
- Current pages in `site/src/content/docs/` (read before updating)

---

## File Map

### Modified
| File | Change |
|------|--------|
| `site/src/content/docs/getting-started/installation.md` | Add `/api` health check, minor v0.13.x notes |
| `site/src/content/docs/getting-started/quick-start.md` | Update auth flow, `edgeplane run`, current commands |
| `site/src/content/docs/getting-started/agent-setup.md` | Add ACP runtime, session token, profile loading |
| `site/src/content/docs/concepts/overview.md` | Add ACP/MeshTasks/Web UI to capabilities table |
| `site/src/content/docs/concepts/entity-reference.md` | Add AgentSession, AiSession entities |
| `site/src/content/docs/guides/deployment.md` | `/api` prefix, current tower flags |
| `site/src/content/docs/guides/oidc.md` | Correct callback URL, Authentik pattern |
| `site/src/content/docs/reference/cli.md` | Remove `launch`, add all `run` runtimes |
| `site/src/content/docs/reference/edgeplaned-daemon.md` | Phase 7 federated attach, node JWT self-heal |
| `site/src/content/docs/reference/command-map.md` | Remove retired commands, add new ones |
| `site/src/content/docs/index.mdx` | Update hero tagline and capability bullets |
| `site/astro.config.mjs` | Add `architecture` sidebar entry |

### Created
| File | What it covers |
|------|---------------|
| `site/src/content/docs/concepts/acp.md` | Agent Connection Protocol concept |
| `site/src/content/docs/concepts/mesh-tasks.md` | MeshTask distributed execution model |
| `site/src/content/docs/concepts/profiles.md` | Personal operator profiles |
| `site/src/content/docs/architecture/overview.md` | Component map |
| `site/src/content/docs/architecture/components.md` | Per-binary deep-dive |
| `site/src/content/docs/architecture/data-flow.md` | Task lifecycle end-to-end |
| `site/src/content/docs/architecture/security.md` | Auth, HMAC approvals, audit trail |
| `site/src/content/docs/guides/web-dashboard.md` | React UI walkthrough |
| `site/src/content/docs/guides/zellij-integration.md` | zRPC plugin setup and use |
| `site/src/content/docs/guides/multi-agent-fleet.md` | Running edgeplaned fleet |
| `site/src/content/docs/guides/fleet-profiles-advanced.md` | Advanced: self-hosted profiles |
| `site/src/content/docs/reference/zrpc-plugin.md` | zRPC plugin reference |
| `site/src/content/docs/adr/0005-unified-agent-launcher.md` | ADR: `edgeplane run` unification |

---

## Task 1: Pre-flight — verify build and capture baseline

**Files:**
- Read: `site/astro.config.mjs`
- Run: `cd /home/merlin/code/edgeplane/site && bun run build`

- [ ] **Step 1: Verify the site builds cleanly before any changes**

```bash
cd /home/merlin/code/edgeplane/site && bun run build 2>&1 | tail -20
```
Expected: build completes, no broken-link errors. Note any existing warnings.

- [ ] **Step 2: Confirm the architecture directory does not yet exist in site**

```bash
ls /home/merlin/code/edgeplane/site/src/content/docs/architecture/ 2>&1
```
Expected: `No such file or directory` — this confirms it needs to be created.

- [ ] **Step 3: Confirm current page count**

```bash
find /home/merlin/code/edgeplane/site/src/content/docs -name "*.md" -o -name "*.mdx" | wc -l
```
Note the number. After the overhaul it should be ~13 higher.

---

## Task 2: Getting Started — update 3 pages

**Files:**
- Modify: `site/src/content/docs/getting-started/installation.md`
- Modify: `site/src/content/docs/getting-started/quick-start.md`
- Modify: `site/src/content/docs/getting-started/agent-setup.md`

**Source material to read first:**
- `CHANGELOG.md` — v0.12.0 `/api` prefix change, v0.13.0 `launch` removal
- Current versions of all three pages

- [ ] **Step 1: Read source material**

```bash
cat /home/merlin/code/edgeplane/CHANGELOG.md | head -200
cat /home/merlin/code/edgeplane/site/src/content/docs/getting-started/installation.md
cat /home/merlin/code/edgeplane/site/src/content/docs/getting-started/quick-start.md
cat /home/merlin/code/edgeplane/site/src/content/docs/getting-started/agent-setup.md
```

- [ ] **Step 2: Update `installation.md`**

Key changes:
  - Health check URL: `curl http://localhost:8008/api/health` (was `/health`, changed in v0.12.0)
  - Version badge: note v0.13.x as current
  - Auth table: confirm node JWT path is `/etc/edgeplane/node.json` (not `~/.edgeplane/node.json`)
  - No other structural changes needed — page is solid

The health verify block should read:
```bash
edgeplane --version
edgeplane health --json   # uses EP_BASE_URL, reports backend + MCP health
```

- [ ] **Step 3: Rewrite `quick-start.md`**

Full structure:
```markdown
---
title: Quick Start
description: Get an agent running under EdgePlane in under 5 minutes.
---

## 1. Start the Control Plane
[Start edgeplane-tower, set EP_BASE_URL]

## 2. Authenticate
edgeplane auth login       # browser OIDC flow → writes ~/.edgeplane/session.json
edgeplane auth whoami      # confirm identity

## 3. Verify the Connection
edgeplane health --json
edgeplane status           # shows auth, runtime, workspace lease status

## 4. Launch an Agent

edgeplane run claude         # Claude Code with EdgePlane MCP wired in
edgeplane run codex          # OpenAI Codex CLI
edgeplane run gemini         # Google Gemini CLI

[Explain: `edgeplane run` validates env, fetches onboarding manifest, injects MCP server]
[Note: edgeplane launch is removed as of v0.13.0 — edgeplane run is the single entry point]

## 5. Explore the Fleet

edgeplane tui     # full-screen TUI — agents, domains, feed, approvals

## What's Next
- Agent Setup → per-agent config, ACP runtime, profile loading
- Concepts: Domains, Missions & Tasks → the organizational model
- Guides: Deployment → running EdgePlane in production
```

- [ ] **Step 4: Rewrite `agent-setup.md`**

Full structure:
```markdown
---
title: Agent Setup
description: Connect an AI agent to EdgePlane — runtimes, profiles, session tokens, and MCP configuration.
---

## Supported Runtimes

| Runtime | Launch command | Protocol |
|---------|---------------|----------|
| Claude Code | `edgeplane run claude` | ACP (persistent JSON-RPC) |
| OpenAI Codex CLI | `edgeplane run codex` | Driver (subprocess) |
| Google Gemini CLI | `edgeplane run gemini` | Driver (subprocess) |
| Goose | `edgeplane run goose` | Native (profile-scoped) |
| OpenClaw | `edgeplane run openclaw` | Driver (subprocess) |
| Custom | `edgeplane run custom` | Driver (subprocess) |

## ACP Runtimes (Claude)

[Explain ACP: persistent JSON-RPC session, why it's different from driver agents]
[EdgePlane injects itself as an MCP server via the `--mcp-server` flag]
[Session persists until `edgeplane run claude --stop` or natural exit]

## Session Tokens

[Never written to config files on disk — injected at exec time from ~/.edgeplane/session.json]
[Create: edgeplane auth login]
[Service accounts for CI: mcs_sa_* tokens created via API]

## Profiles

[Operators carry a personal profile — env config, tool settings, instruction files]
[Profile loads automatically on `edgeplane run`]
[Switch: edgeplane profile switch <name>]
[Push/pull: edgeplane profile push / edgeplane profile pull]

## MCP Configuration

[When running Claude Code manually (without edgeplane run), add EdgePlane as an MCP server]
[Show the claude_desktop_config.json / .mcp.json snippet for manual wiring]
[Note: edgeplane run claude does this automatically]

## Environment Variables

[Table of EP_BASE_URL, EP_OUTPUT — no EP_TOKEN (retired)]

## What's Next
- [Concepts: ACP](/concepts/acp/) — how agent sessions work under the hood
- [Concepts: Profiles](/concepts/profiles/) — personal operator profiles in depth
- [Guides: Multi-Agent Fleet](/guides/multi-agent-fleet/) — running edgeplaned
```

- [ ] **Step 5: Run scrubbing check on all three pages**

Verify no homelab hostnames, no personal fleet profile names, no `/workspace/cache/` paths, no `EP_TOKEN`.

- [ ] **Step 6: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/getting-started/
git commit -m "docs(site): update Getting Started for v0.13.x — edgeplane run, ACP, /api prefix

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 3: Concepts — update 2 pages, create 3 new

**Files:**
- Modify: `site/src/content/docs/concepts/overview.md`
- Modify: `site/src/content/docs/concepts/entity-reference.md`
- Create: `site/src/content/docs/concepts/acp.md`
- Create: `site/src/content/docs/concepts/mesh-tasks.md`
- Create: `site/src/content/docs/concepts/profiles.md`

**Source material to read first:**
- `/home/merlin/code/edgeplane/docs/architecture/entities.md` — canonical entity defs
- `/home/merlin/code/edgeplane/CHANGELOG.md` — v0.12.0–v0.13.1
- Current `overview.md` and `entity-reference.md`

- [ ] **Step 1: Read source material**

```bash
cat /home/merlin/code/edgeplane/docs/architecture/entities.md
cat /home/merlin/code/edgeplane/CHANGELOG.md | grep -A5 "ACP\|MeshTask\|profile\|AiSession\|AgentSession"
cat /home/merlin/code/edgeplane/site/src/content/docs/concepts/overview.md
cat /home/merlin/code/edgeplane/site/src/content/docs/concepts/entity-reference.md
```

- [ ] **Step 2: Update `overview.md` capability table**

Add three rows to the existing "What EdgePlane Provides" table:

| Capability | What it does |
|------------|-------------|
| **Agent Connection Protocol (ACP)** | Persistent JSON-RPC sessions for long-running agents — sessions survive crashes and reconnects |
| **Web Dashboard** | React UI for fleet monitoring, live event feed, domain/task drill-down, and ACP conversation panes |
| **Mesh Execution** | Agents claim and execute `MeshTask` units from a shared queue — distributed work without central scheduling |

Also add links to the three new concept pages in the "Next Steps" section.

- [ ] **Step 3: Update `entity-reference.md`**

Add two new entity sections after `Agent`:

**AgentSession** section:
```markdown
## AgentSession

**A live agent process attached to the control plane.** Created when `edgeplane run` starts an agent; destroyed on clean exit or timeout.

- Columns: `agent_id`, `runtime_kind`, `runtime_node_id`, `session_token` (hashed), `status`, `started_at`, `last_seen_at`
- `runtime_kind`: one of `claude_agent_acp`, `zellij_hosted`, `codex`, `gemini`, `goose`, `openclaw`, `custom`
- `runtime_node_id`: the `edgeplaned` node that owns this session
- Session status: `active` | `idle` | `terminated`
```

**AiSession** section:
```markdown
## AiSession

**A tracked conversation session in the AI Console.** Created when an operator opens a conversation in the web dashboard's AI Console tab.

- Columns: `agent_id` (FK), `turns` (JSON array), `status`
- Each turn carries `role`, `content`, `events` (tool calls, progress frames)
- Distinct from `AgentSession` — an `AiSession` is the conversation record; `AgentSession` is the live process
```

- [ ] **Step 4: Create `acp.md`**

Full content:
```markdown
---
title: ACP — Agent Connection Protocol
description: How EdgePlane manages persistent agent sessions — the transport model, session lifecycle, and supported runtimes.
---

## What ACP Is

The Agent Connection Protocol (ACP) is EdgePlane's mechanism for maintaining persistent, bidirectional communication with long-running AI agents. Unlike simple subprocess execution, ACP sessions survive crashes, reconnect automatically, and carry full context continuity across turns.

ACP is the protocol underlying `claude_agent_acp` — the runtime used when you `edgeplane run claude`.

## Transport Model

EdgePlane acts as an MCP server injected into the agent at startup. The agent communicates back through:

1. **JSON-RPC over stdio** — the agent's MCP client calls EdgePlane tools (task creation, artifact publication, governance actions)
2. **WebSocket attach** (`/api/attach-ws`) — operators and the web dashboard attach to a live agent session for real-time I/O
3. **SSE event feed** (`/api/events`) — the control plane broadcasts session events (progress, tool calls, errors) to subscribers

## Session Lifecycle

```
edgeplane run claude
  → edgeplaned spawns Claude Code with --mcp-server edgeplane
  → EdgePlane MCP server registers the AgentSession
  → Session status: active
  → Agent works; calls EdgePlane MCP tools freely
  → Crash / network drop → edgeplaned restarts; session reattaches
  → Clean exit → session marked terminated
```

The `claude_agent_acp` runtime tracks the agent's process lifetime, manages the attach-secret handshake, and clears stale sessions on restart.

## Attach Secret

When `edgeplaned` starts an ACP session, it writes a short-lived attach secret to the agent's environment. The web dashboard and `edgeplane attach` use this secret to establish the WebSocket connection. The secret is:
- One-time — consumed on first attach
- Self-healing — `edgeplaned` re-issues it if the session reconnects

The `/api/attach-ws` endpoint requires owner-scope authentication; agents outside the owner's session cannot attach.

## Supported Runtimes

| Runtime | Protocol | Notes |
|---------|----------|-------|
| `claude_agent_acp` | ACP (persistent JSON-RPC) | Primary; supports full attach and context injection |
| `zellij_hosted` | PTY bridge | Long-running agents in a Zellij pane; signals via `edgeplane agent signal` |
| `codex`, `gemini`, `goose` | Driver (subprocess) | One-shot or session-based; no bidirectional attach |
| `openclaw`, `custom` | Driver (subprocess) | Instance-isolated driver agents |

## The zellij_hosted Runtime

For agents running in a Zellij terminal pane, EdgePlane bridges via PTY. Input frames typed in the web dashboard chat UI are converted to PTY stdin bytes; pane output streams back as SSE. The `edgeplane-zrpc` Zellij plugin (optional, feature-flagged) extends this with focus-free injection, scrollback reads, and pane lifecycle events.

See [Zellij Integration](/guides/zellij-integration/) for setup.

## What's Next
- [MeshTask System](/concepts/mesh-tasks/) — distributed agent-to-agent task execution
- [Architecture: Data Flow](/architecture/data-flow/) — how ACP fits into the full task lifecycle
- [Guides: Web Dashboard](/guides/web-dashboard/) — attaching to a live session in the browser
```

- [ ] **Step 5: Create `mesh-tasks.md`**

Full content:
```markdown
---
title: MeshTask System
description: Distributed agent-to-agent task execution — how agents claim, execute, and complete work from a shared queue.
---

## What a MeshTask Is

A `MeshTask` is a unit of work in the EdgePlane mesh — designed to be claimed and executed by any agent with the right capabilities, without a central scheduler assigning work.

Think of it as a job queue built into the control plane: tasks are submitted by any agent or operator, agents pull from the queue, execute with a lease, and record results as artifacts.

## The Claim-Execute-Complete Lifecycle

```
Submit          → status: pending
Claim           → status: claimed (lease issued, expires_at set)
Heartbeat       → lease extended (agent signals it's still alive)
Complete/Fail   → status: completed | failed (result artifact recorded)
Lease expire    → status: pending again (unclaimed by timeout)
```

Agents use the `claim_mesh_task` MCP tool to atomically claim a task. The lease prevents two agents from executing the same task simultaneously.

## Capabilities and Routing

Each `MeshTask` carries a `required_capabilities` list. Agents declare their capabilities at registration (`edgeplane agent register --capabilities "..."`) and only see tasks they can handle.

This enables coarse-grained routing without a scheduler:
- A task requiring `"rust,code-review"` is only visible to agents declaring those capabilities
- A task with no requirements is visible to all agents in the domain

## Parent-Child Structure

Tasks can spawn subtasks (`parent_task_id`). A parent task typically stays in `claimed` state while its children execute, completing when the last child completes. This enables parallel fan-out patterns within a domain.

## Result Artifacts

When a task completes, the executing agent records its output as an artifact linked via `result_artifact_id`. The artifact is stored in the mission's S3 path:

```
domains/{domain_id}/missions/{mission_id}/artifacts/{filename}
```

The result artifact is queryable via the ledger — full provenance, SHA-256 content hash, and version history.

## MeshTask vs Task

| | Task | MeshTask |
|-|------|----------|
| Primary user | Human operators, UI | AI agents |
| Assignment | Explicit owner | Capability-based claim |
| Lease | No | Yes (TTL-based) |
| Result | Definition of done text | Result artifact |
| Parent/child | No | Yes |

Both live inside a Mission. Use `Task` for human-tracked work items; use `MeshTask` for agent-executable units.

## MCP Tools for Mesh Execution

Agents interact with the mesh via these MCP tools (available when EdgePlane is wired in as an MCP server):

| Tool | Purpose |
|------|---------|
| `submit_mesh_task` | Create a new MeshTask in a mission |
| `claim_mesh_task` | Atomically claim a pending task (returns lease) |
| `heartbeat_mesh_task` | Extend the lease; signal continued execution |
| `complete_mesh_task` | Mark done, record result artifact |
| `fail_mesh_task` | Mark failed with reason |
| `list_mesh_tasks` | Query tasks by mission, status, or capability |

## What's Next
- [Entity Reference: MeshTask](/concepts/entity-reference#meshtask) — canonical entity definition
- [Architecture: Data Flow](/architecture/data-flow/) — full lifecycle with artifacts
- [Concepts: ACP](/concepts/acp/) — how agents connect to claim tasks
```

- [ ] **Step 6: Create `profiles.md`**

Full content:
```markdown
---
title: Profiles
description: Personal operator profiles — what they carry, how they travel, and how to switch between them.
---

## What a Profile Is

An EdgePlane profile is a portable bundle of operator configuration:

- **Environment variables** — `EP_BASE_URL`, tool-specific config, API keys (injected at agent startup, never written to disk)
- **Instruction files** — CLAUDE.md, AGENTS.md, or similar agent-guidance files scoped to a context (coding, review, research)
- **Tool settings** — per-agent configuration for Claude Code, Codex, Gemini, or custom runtimes

Profiles are **stored server-side, scoped strictly to the owner** — no other operator can read or list your profiles.

## How Profiles Travel

On `edgeplane run`, the CLI:
1. Fetches the active profile from the control plane
2. Writes it to `~/.edgeplane/profiles/<name>/` via atomic symlink swap
3. Injects the profile's environment into the agent subprocess

This means your profile follows you: switch machines, run `edgeplane auth login`, and your next `edgeplane run` picks up your profile automatically.

## Profile Commands

```bash
edgeplane profile list                    # list your profiles
edgeplane profile show [<name>]           # show the active or named profile
edgeplane profile switch <name>           # set the active profile
edgeplane profile push                    # push local profile state to the server
edgeplane profile pull [<name>]           # pull latest from server to local
edgeplane profile create <name>           # create a new empty profile
```

## Profile Contexts

You can maintain separate profiles for different types of work:

```bash
edgeplane profile switch coding           # coding-focused instruction files
edgeplane profile switch review           # code review posture + different tool settings
edgeplane profile switch research         # research-focused, different LLM config
```

Each profile has its own instruction files and environment, so switching profiles changes the agent's context immediately on next launch.

## What's in a Profile

```
~/.edgeplane/profiles/<name>/
├── env                    # key=value pairs injected at agent startup
├── CLAUDE.md              # Claude Code instruction file (if using Claude)
├── AGENTS.md              # Codex/Gemini instruction file
└── config.json            # Profile metadata (name, version, last_pushed)
```

The `env` file is never written to disk as plaintext beyond this local cache. Sensitive values should use the secrets broker rather than plain env vars.

## What's Next
- [Agent Setup](/getting-started/agent-setup/) — how profiles load on agent startup
- [Advanced: Personal Fleet Profiles](/guides/fleet-profiles-advanced/) — self-hosted profile sync and fleet-wide profile management
```

- [ ] **Step 7: Run scrubbing check on all five pages**

Check for homelab references, personal profile names, personal Infisical paths.

- [ ] **Step 8: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/concepts/
git commit -m "docs(site): Concepts — ACP, MeshTasks, Profiles (new); overview + entity-ref updates

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 4: Architecture — create 4 new pages

**Files:**
- Create: `site/src/content/docs/architecture/overview.md`
- Create: `site/src/content/docs/architecture/components.md`
- Create: `site/src/content/docs/architecture/data-flow.md`
- Create: `site/src/content/docs/architecture/security.md`

**Source material to read first:**
- `/home/merlin/code/edgeplane/docs/architecture/architecture.md`
- `/home/merlin/code/edgeplane/CHANGELOG.md`
- `ls /home/merlin/code/edgeplane/crates/` — crate names

- [ ] **Step 1: Read source material**

```bash
cat /home/merlin/code/edgeplane/docs/architecture/architecture.md
ls /home/merlin/code/edgeplane/crates/
cat /home/merlin/code/edgeplane/CHANGELOG.md | head -250
```

- [ ] **Step 2: Create `architecture/overview.md`**

Full content:
```markdown
---
title: Architecture Overview
description: How EdgePlane's components fit together — the CLI, control plane server, daemon, web UI, and Zellij plugin.
---

## Components at a Glance

EdgePlane is four binaries and an optional plugin, each with a clear responsibility:

| Component | Binary | Role |
|-----------|--------|------|
| CLI | `edgeplane` | Operator interface — launch agents, manage domains/tasks, TUI, auth |
| Control plane | `edgeplane-tower` | HTTP/SSE API server — state store, governance, artifact ledger |
| Daemon | `edgeplaned` | Node executor — agent lifecycle, secrets broker, cron dispatch |
| Web UI | (served by tower) | React dashboard — fleet view, ACP chat, event feed, governance |
| Zellij plugin | `edgeplane-zrpc` | Optional — focus-free terminal control for Zellij-hosted agents |

## Deployment Topology

```
Operator machine                      Server / cloud
┌─────────────────────────┐           ┌────────────────────────────┐
│  edgeplane (CLI / TUI)  │──────────▶│  edgeplane-tower           │
│  edgeplaned (daemon)    │──────────▶│    ├─ REST + SSE API        │
│  AI agents (Claude,     │           │    ├─ Postgres (state)      │
│   Codex, Gemini…)       │           │    ├─ pgvector (search)     │
│  Web browser (dashboard)│──────────▶│    └─ S3 (artifacts)       │
└─────────────────────────┘           └────────────────────────────┘
```

The server can run locally (single-node, Docker Compose) or remotely (Kubernetes, single VM). Multiple operator machines can connect to one server.

## Communication Paths

| From | To | Protocol |
|------|----|----------|
| `edgeplane` CLI | `edgeplane-tower` | HTTP REST (`/api/*`) |
| `edgeplaned` | `edgeplane-tower` | HTTP REST + WebSocket attach |
| Web dashboard | `edgeplane-tower` | HTTP REST + SSE (`/api/events`) + WebSocket attach (`/api/attach-ws`) |
| Agent (ACP) | `edgeplaned` (local) | Unix socket; `edgeplaned` proxies to tower |
| `edgeplane-zrpc` (plugin) | `edgeplaned` | Zellij pipes (named pipe IPC) |

## State Ownership

| State | Where |
|-------|-------|
| Domains, missions, tasks, agents, artifacts | Postgres via `edgeplane-tower` |
| Vector index (similarity search) | pgvector (co-located with Postgres) |
| Artifact files | S3-compatible object storage |
| Session tokens | `~/.edgeplane/session.json` (client) + DB (server) |
| Node identity | `/etc/edgeplane/node.json` (daemon node) |
| Agent profiles | DB (server) + `~/.edgeplane/profiles/` (local cache) |

## What's Next
- [Components](/architecture/components/) — per-binary responsibilities and configuration
- [Data Flow](/architecture/data-flow/) — how a task flows from creation to completion
- [Security Model](/architecture/security/) — authentication, authorization, and audit
```

- [ ] **Step 3: Create `architecture/components.md`**

Full content:
```markdown
---
title: Component Reference
description: What each EdgePlane binary owns, how to configure it, and where its trust boundary sits.
---

## edgeplane — The CLI

`edgeplane` is the primary human and scripted interface. It never holds persistent state — it reads session tokens and connects to `edgeplane-tower` for everything.

**Responsibilities:**
- Interactive TUI (`edgeplane tui`)
- Agent launch (`edgeplane run <runtime>`)
- Auth lifecycle (`edgeplane auth login/logout/whoami`)
- Domain/mission/task CRUD
- Profile management
- Status, health, and diagnostics

**Trust boundary:** Runs as the operator. Holds a session token from `edgeplane auth login`. Has no privileged access beyond what the session token permits.

**Config:**
- `EP_BASE_URL` — control plane URL (default `http://localhost:8008`)
- `~/.edgeplane/session.json` — session token (chmod 600, auto-managed)

---

## edgeplane-tower — The Control Plane

`edgeplane-tower` is the HTTP server that owns all shared state. It is the single source of truth.

**Responsibilities:**
- REST API under `/api/*` (domains, missions, tasks, agents, artifacts, governance)
- SSE event stream (`/api/events`) — real-time broadcast to all subscribers
- WebSocket attach (`/api/attach-ws`) — bidirectional PTY bridge for ACP agents
- OIDC authentication (login, callback, session issuance)
- Governance policy lifecycle and HMAC-signed approval tokens
- Artifact ledger (Postgres + pgvector + S3)
- Serving the React web dashboard

**Trust boundary:** Authoritative. All writes go through here. OIDC-issued JWTs and node JWTs are validated here. No agent writes directly to the database.

**Config:**
- `DATABASE_URL` — Postgres connection string
- `S3_*` — object storage config (endpoint, bucket, key, secret)
- OIDC client credentials (set via environment or secrets manager)
- `BIND_ADDR` — listen address (default `0.0.0.0:8008`)

---

## edgeplaned — The Node Daemon

`edgeplaned` is the headless executor that runs on each operator node. It manages the agent processes that do the actual work.

Think of it as the kubelet to `edgeplane`'s kubectl: `edgeplane` is the operator interface, `edgeplaned` is the node-side executor.

**Responsibilities:**
- Agent lifecycle — spawn, restart, watchdog for ACP and Zellij-hosted agents
- Secrets broker — injects credentials into agent subprocesses via Unix socket (`~/.edgeplane/edgeplaned/secrets.sock`)
- PTY bridge — connects Zellij-hosted agent panes to the tower's attach-ws endpoint
- Cron dispatch — periodic job runner reading `~/.edgeplane/edgeplaned/cron.toml`
- Node registration — holds the node JWT at `/etc/edgeplane/node.json`

**Trust boundary:** Holds a machine-identity JWT (not a user session). Permitted to act on behalf of the agents it manages. Cannot escalate privileges beyond what the tower grants to its node identity.

**Config:**
- `EP_BASE_URL` (or `--backend-url`) — tower URL
- `/etc/edgeplane/node.json` — node JWT (written by `edgeplane agent node register`)
- `~/.edgeplane/edgeplaned/cron.toml` — cron job definitions

---

## Web Dashboard

The React web dashboard is served by `edgeplane-tower` as a static SPA. It communicates exclusively through the tower's REST + SSE + WebSocket API — no direct database access.

**Tabs:**
| Tab | What it shows |
|-----|--------------|
| Fleet | Live agent grid — status, runtime, current task, ACP chat attach |
| Domains | Domain → Mission → Task drill-down tree |
| Governance | Policy lifecycle, pending approvals queue |
| AI Console | Direct AI session with any connected agent |
| Feed | Live SSE event stream (all domain events) |

**Auth:** OIDC via browser — login redirects to the configured identity provider, returns a session cookie. The avatar in the sidebar shows display name from the `preferred_username` OIDC claim.

---

## edgeplane-zrpc — The Zellij Plugin (Optional)

`edgeplane-zrpc` is a WebAssembly plugin for the Zellij terminal multiplexer. It is optional and feature-flagged — if `EDGEPLANE_ZRPC_PLUGIN_PATH` is unset, `edgeplaned` behaves exactly as without it.

**What it adds over PTY bridging:**
- Focus-free input injection — no need to focus the agent's pane to send a prompt
- Scrollback reads — `edgeplaned` can read pane history programmatically
- Pane lifecycle events — detect when a pane exits, splits, or changes
- Cancel signal — interrupt a running agent without killing the pane

**Build note:** The plugin is a Rust **bin crate** (not cdylib) — required for Zellij's `_start` WASI export. A cdylib on `wasm32-wasip1` produces a WASI reactor with no `_start`, which Zellij 0.44.x rejects at instantiation.

See [Zellij Integration](/guides/zellij-integration/) for setup and [zRPC Plugin Reference](/reference/zrpc-plugin/) for the full configuration reference.

## What's Next
- [Data Flow](/architecture/data-flow/) — how a task moves through all these components
- [Security Model](/architecture/security/) — who trusts what and how
```

- [ ] **Step 4: Create `architecture/data-flow.md`**

Full content:
```markdown
---
title: Data Flow
description: How a task moves through EdgePlane — from creation to agent execution to artifact publication.
---

This page traces the lifecycle of a `MeshTask` from creation to completion. It shows how each component participates and where state is recorded.

## Full Lifecycle

### 1. Task Submission

An agent or operator submits a MeshTask via the `submit_mesh_task` MCP tool (or the REST API):

```
Agent (Claude) → MCP tool call: submit_mesh_task(mission_id, description, capabilities)
  → edgeplane-tower: INSERT meshtask (status=pending)
  → tower broadcasts SSE event: {type: "meshtask.created", task_id: "..."}
```

All subscribers on `/api/events` receive the broadcast immediately.

### 2. Agent Discovery

Any `edgeplaned` node polling for work sees the new task:

```
edgeplaned (worker mode) → GET /api/missions/{id}/mesh-tasks?status=pending&capabilities=...
  ← edgeplane-tower returns matching tasks
```

Capability filtering happens server-side — agents only see tasks they can handle.

### 3. Claim (Atomic Lease)

The agent claims the task via `claim_mesh_task`:

```
Agent → MCP tool: claim_mesh_task(task_id)
  → edgeplane-tower: UPDATE meshtask SET status=claimed, claimed_by=<agent_id>,
                                         lease_expires_at=now()+TTL
  ← Returns lease token
```

The update is atomic — if two agents race, only one succeeds. The other gets a 409 and retries.

### 4. Execution with Heartbeat

While the agent executes, it extends its lease periodically:

```
Agent → MCP tool: heartbeat_mesh_task(task_id, lease_token)
  → edgeplane-tower: UPDATE lease_expires_at = now() + TTL
```

If heartbeats stop (agent crashes), the lease expires and the task returns to `pending` automatically.

### 5. Artifact Publication

On completion, the agent publishes its output:

```
Agent → MCP tool: create_artifact(mission_id, content, filename)
  → edgeplane-tower:
      1. Stream content to S3 at domains/{domain_id}/missions/{mission_id}/artifacts/{filename}
      2. INSERT artifact (uri, content_sha256, storage_backend)
      3. Vector-index the artifact content (pgvector)
      4. Record ledger entry (provenance, agent_id, timestamp)
```

### 6. Completion

```
Agent → MCP tool: complete_mesh_task(task_id, lease_token, result_artifact_id)
  → edgeplane-tower: UPDATE meshtask SET status=completed, result_artifact_id=<id>
  → tower broadcasts SSE event: {type: "meshtask.completed", task_id: "..."}
```

### 7. Overlap Detection

Before any task or artifact creation, EdgePlane runs overlap detection:

```
New task/artifact → tower: fuzzy + vector similarity against existing tasks/artifacts
  → if similarity > threshold: surface collision candidates
  → caller decides: proceed, merge, or cancel
```

This runs synchronously before the INSERT — collisions surface before damage.

## State at Each Stage

| Stage | Postgres | S3 | SSE Broadcast |
|-------|---------|-----|--------------|
| Submit | meshtask row (pending) | — | meshtask.created |
| Claim | status=claimed, lease | — | meshtask.claimed |
| Heartbeat | lease_expires_at updated | — | — |
| Artifact | artifact row + ledger | file uploaded | artifact.created |
| Complete | status=completed | — | meshtask.completed |

## ACP Session Data Flow

For an ACP session (Claude Code connected via `edgeplane run claude`):

```
User types prompt in web dashboard
  → WebSocket frame to /api/attach-ws
  → tower proxies PTY frame to edgeplaned
  → edgeplaned injects to agent's PTY stdin
  → Agent generates response + calls MCP tools
  → MCP tool results return via JSON-RPC
  → Agent output streams back to PTY
  → tower broadcasts SSE progress events
  → Web dashboard renders output in real-time
```

## What's Next
- [Security Model](/architecture/security/) — how each step is authorized
- [Concepts: MeshTask System](/concepts/mesh-tasks/) — the task model in depth
- [Concepts: ACP](/concepts/acp/) — the agent session protocol
```

- [ ] **Step 5: Create `architecture/security.md`**

Full content:
```markdown
---
title: Security Model
description: Authentication, authorization, audit trail, and trust boundaries in EdgePlane.
---

## Authentication Modes

EdgePlane has three authentication paths. No static API token env var (`EP_TOKEN`) — that was removed in v0.12.0.

| Mode | Identity | Issued by | Stored at |
|------|----------|-----------|-----------|
| OIDC session | Human operator | Identity provider → tower | `~/.edgeplane/session.json` (chmod 600) |
| Node JWT | Machine / daemon | `edgeplane agent node register` | `/etc/edgeplane/node.json` |
| Service account | CI / scripted | API (requires admin) | Caller-managed; `mcs_sa_*` prefix |

Session tokens are never written to agent config files — they are injected at exec time only.

## OIDC Flow

```
edgeplane auth login
  → opens browser to edgeplane-tower /api/auth/oidc/login
  → identity provider issues authorization code
  → tower exchanges code for userinfo
  → tower issues session token (mcs_* prefix)
  → token written to ~/.edgeplane/session.json
```

The OIDC callback URL is `https://<your-tower-host>/api/auth/oidc/callback`. Configure this exactly in your identity provider.

User display name is sourced from the `preferred_username` OIDC claim (falling back to `name`), stored in the session, and shown in the web dashboard avatar.

## Authorization Model

All writes to the tower go through the session or node JWT. The tower validates:
- **Owner scope** — operators can only attach to their own agent sessions (`/api/attach-ws` enforces this)
- **Domain membership** — tasks and artifacts require domain contributor or owner role
- **Governance policies** — operations marked as requiring approval are gated on HMAC-signed approval tokens

## HMAC Approval Tokens

For operations requiring governance approval (configurable per domain):

```
Agent requests governed action
  → tower creates pending approval record
  → approval notified to domain owners via SSE
  → approver reviews in web dashboard (Governance tab) or via CLI
  → tower issues HMAC-signed approval token (expires in TTL)
  → agent presents token with the action
  → tower verifies HMAC, executes action, records in audit ledger
```

The token is signed with a server-side secret. It binds: action type, resource ID, approver identity, and expiry. Replay and cross-resource use are both rejected.

## Audit Trail

Every mutation is recorded:
- **Ledger entries** — who, what, when, on which resource
- **Artifact provenance** — `artifact.provenance` JSON carries agent_id, session_id, timestamp, parent artifact
- **SHA-256 content hashes** — artifacts are content-addressed; tampering is detectable

The ledger is append-only at the application layer. Direct database modification is outside the threat model.

## Network Security

- All external traffic should be TLS-terminated (reverse proxy, load balancer, or Cloudflare)
- The `edgeplaned` management socket is Unix-only (`~/.edgeplane/edgeplaned/mgmt.sock`) — no network exposure
- The secrets broker socket (`secrets.sock`) is accessible only to agent subprocesses spawned by `edgeplaned`
- `edgeplane-tower` serves the attach-ws endpoint with owner-scope enforcement — unauthenticated attach attempts are rejected

## What's Next
- [Guides: OIDC Setup](/guides/oidc/) — configuring your identity provider
- [Guides: Deployment](/guides/deployment/) — TLS, reverse proxy, and production hardening
```

- [ ] **Step 6: Run scrubbing check on all four pages**

- [ ] **Step 7: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/architecture/
git commit -m "docs(site): Architecture section — overview, components, data flow, security (all new)

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 5: Guides — update 2 pages, create 4 new

**Files:**
- Modify: `site/src/content/docs/guides/deployment.md`
- Modify: `site/src/content/docs/guides/oidc.md`
- Create: `site/src/content/docs/guides/web-dashboard.md`
- Create: `site/src/content/docs/guides/zellij-integration.md`
- Create: `site/src/content/docs/guides/multi-agent-fleet.md`
- Create: `site/src/content/docs/guides/fleet-profiles-advanced.md`

**Source material to read first:**
- Current `deployment.md` and `oidc.md`
- CHANGELOG v0.12.0 (API prefix change) and v0.13.0–v0.13.1 (zRPC, run, federated attach)
- `docs/architecture/architecture.md`

- [ ] **Step 1: Read source material**

```bash
cat /home/merlin/code/edgeplane/site/src/content/docs/guides/deployment.md
cat /home/merlin/code/edgeplane/site/src/content/docs/guides/oidc.md
cat /home/merlin/code/edgeplane/CHANGELOG.md | grep -B2 -A10 "0.12.0\|/api\|attach\|federated\|zrpc"
```

- [ ] **Step 2: Update `deployment.md`**

Key changes:
- Health check URL: `curl https://your-tower-host/api/health` (not `/health`)
- OIDC callback URL: `https://your-tower-host/api/auth/oidc/callback`
- Remove any `EP_TOKEN` references
- Add section: "Reverse Proxy / TLS" (nginx/caddy snippet terminating TLS to tower:8008)
- Docker Compose snippet should use current env var names

- [ ] **Step 3: Update `oidc.md`**

Key changes:
- OIDC callback URL throughout: `https://your-tower-host/api/auth/oidc/callback` (the `/api` prefix was added in v0.12.0)
- Verify all env var names match current tower config
- Authentik example should use generic host names — no personal Authentik instance URLs

- [ ] **Step 4: Create `guides/web-dashboard.md`**

Full content:
```markdown
---
title: Using the Web Dashboard
description: A walkthrough of the EdgePlane React dashboard — fleet view, ACP sessions, event feed, and governance.
---

## Accessing the Dashboard

The dashboard is served by `edgeplane-tower` at your tower's root URL. Open it in a browser and log in via OIDC (your identity provider).

Your display name and initials appear in the sidebar avatar, sourced from the `preferred_username` claim in your OIDC token.

## Tabs

### Fleet

The Fleet tab is the homepage — a live grid of all connected agents:

- **Status indicator** — green (active), yellow (idle), grey (offline)
- **Runtime badge** — `claude_agent_acp`, `zellij_hosted`, `codex`, etc.
- **Node** — which `edgeplaned` node the agent is running on
- **Current task** — the task the agent is executing, if any

Click any agent row to open the ACP chat pane for that agent.

### ACP Chat (Fleet → Agent Row)

When you click an agent, a chat panel opens on the right. For `claude_agent_acp` and `zellij_hosted` agents:
- **Input** — type a prompt and press Enter; it is injected into the agent's PTY stdin
- **Output** — the agent's response streams back in real-time via the attach-ws WebSocket

The attach uses your session credentials — you can only attach to agents you own.

### Domains

A tree view: Domain → Mission → Task. Click any node to see details, create sub-items, or view artifacts. Missions show their `workstream_md` narrative; tasks show owner, definition of done, and status.

### Governance

Lists pending approval requests across all your domains. Each request shows:
- What action is being requested
- Which agent requested it
- The domain policy that triggered the approval requirement

Approve (`y`) or deny (`n`). Approved actions are HMAC-signed and returned to the requesting agent automatically.

### AI Console

A direct conversation interface backed by the `AiSession` model. Useful for one-off queries to a connected agent without opening a full attach session. The Console uses the REST `/api/ai/*` surface (not the PTY bridge).

### Feed

A live SSE event stream showing all domain events in real-time:
- `meshtask.*` — task lifecycle events
- `artifact.*` — artifact creation and updates
- `agent.*` — agent status changes
- `session.*` — ACP session lifecycle

Press `p` (or the Pause button) to freeze the feed for inspection.

## Theme

Toggle between light and dark mode via the avatar popup in the sidebar (Preferences → Theme).

## What's Next
- [Concepts: ACP](/concepts/acp/) — how the attach session works under the hood
- [Architecture: Components](/architecture/components/) — how the dashboard talks to the tower
```

- [ ] **Step 5: Create `guides/zellij-integration.md`**

Full content:
```markdown
---
title: Zellij Integration
description: How to set up the edgeplane-zrpc Zellij plugin for focus-free agent control.
---

## What the Zellij Plugin Adds

By default, EdgePlane communicates with `zellij_hosted` agents by paste-and-send — focus the pane, inject keystrokes, wait. This works, but has a race: if the agent is actively typing, the inject can land mid-output.

The `edgeplane-zrpc` Zellij plugin replaces this with a named-pipe control path: `edgeplaned` sends commands to the plugin over a Zellij pipe, and the plugin acts without the pane needing focus. This unlocks:

- **Focus-free injection** — send prompts to any pane without a focus race
- **Scrollback reads** — `edgeplaned` reads recent pane output programmatically
- **Pane lifecycle events** — detect when a pane exits, splits, or resizes
- **Cancel** — interrupt a running agent without killing the pane

## Feature Flag

The plugin is **dormant by default**. If `EDGEPLANE_ZRPC_PLUGIN_PATH` is unset, `edgeplaned` falls back to the paste path exactly as before — no behavior change.

## Prerequisites

- Zellij 0.44.x
- Rust toolchain (to build the plugin, or use the prebuilt binary from releases)
- `wasm32-wasip1` target: `rustup target add wasm32-wasip1`

## Building the Plugin

```bash
git clone https://github.com/RyanMerlin/edgeplane.git
cd edgeplane/crates/edgeplane-zrpc
cargo build --release --target wasm32-wasip1
# Output: target/wasm32-wasip1/release/edgeplane_zrpc.wasm
```

## Installation

The install tooling sets up the Zellij config and permissions automatically:

```bash
edgeplane zrpc install --plugin-path /path/to/edgeplane_zrpc.wasm
```

This writes:
1. The session config snippet (`plugins {}` + `load_plugins {}`) to `~/.config/zellij/configs/edgeplane.kdl`
2. A `permissions.kdl` file in the Zellij cache directory granting the plugin `ReadApplicationState`, `ReadCliPipes`, `WriteToStdin`, and `ReadPaneContents`

Find your Zellij cache directory:
```bash
zellij setup --check | grep 'CACHE DIR'
```

## Enabling on edgeplaned

Set two environment variables before starting `edgeplaned`:

```bash
export EDGEPLANE_ZRPC_PLUGIN_PATH=/path/to/edgeplane_zrpc.wasm
export EDGEPLANE_ZRPC_SESSIONS="my-session"   # comma-separated Zellij session names to watch
edgeplane daemon up
```

Or add to `~/.edgeplane/edgeplaned/env`:
```
EDGEPLANE_ZRPC_PLUGIN_PATH=/path/to/edgeplane_zrpc.wasm
EDGEPLANE_ZRPC_SESSIONS=my-session
```

## Verifying

Start a Zellij session and check the plugin loads:

```bash
zellij attach my-session
# In a pane, verify the plugin is running:
zellij pipe --name edgeplane-zrpc -- '{"kind":"ping"}'
# Expected response within 2 seconds: {"kind":"pong"}
```

If `zellij pipe` hangs, check the Zellij log:
```bash
# Default log path:
tail -f /tmp/zellij-$(id -u)/zellij-log/zellij.log | grep edgeplane
```
Common failure: permissions not pre-seeded — run `edgeplane zrpc install` again.

## Pipe Protocol

The plugin communicates over a named pipe (`edgeplane-zrpc`). All messages are JSON:

| Direction | Message | Payload |
|-----------|---------|---------|
| `edgeplaned` → plugin | `inject` | `{kind: "inject", pane_id: N, text: "..."}` |
| `edgeplaned` → plugin | `scrollback` | `{kind: "scrollback", pane_id: N, lines: N}` |
| plugin → `edgeplaned` | `pane_event` | `{kind: "pane_event", event: "exit"|"focus_gained", pane_id: N}` |
| plugin → `edgeplaned` | `scrollback_response` | `{kind: "scrollback_response", pane_id: N, content: "..."}` |

See [zRPC Plugin Reference](/reference/zrpc-plugin/) for the full protocol and all message types.

## What's Next
- [zRPC Plugin Reference](/reference/zrpc-plugin/) — full env var and protocol reference
- [Architecture: Components](/architecture/components/) — where the plugin fits in the stack
```

- [ ] **Step 6: Create `guides/multi-agent-fleet.md`**

Full content:
```markdown
---
title: Running a Multi-Agent Fleet
description: How to run edgeplaned for persistent agent management across one or more machines.
---

## Overview

A fleet is one or more `edgeplaned` daemons — each managing agents on a node — all connected to a shared `edgeplane-tower`. The web dashboard gives you a live view across all nodes.

## Setting Up a Node

### 1. Register the Node

Each machine running `edgeplaned` needs a node identity:

```bash
edgeplane agent node register --hostname my-node
# Writes JWT to /etc/edgeplane/node.json
# Requires sudo (writing to /etc/edgeplane/)
```

If you can't write to `/etc/`, specify an alternate path:

```bash
edgeplane agent node register --hostname my-node --output ~/.edgeplane/node.json
# Then: export EDGEPLANE_NODE_JWT_PATH=~/.edgeplane/node.json
```

### 2. Start the Daemon

```bash
edgeplane daemon up
# Or directly:
edgeplaned run --backend-url https://your-tower-host
```

Verify the node appears in the fleet:

```bash
edgeplane agent list
```

### 3. Run as a Persistent Service

For production, run `edgeplaned` as a systemd user service:

```ini
# ~/.config/systemd/user/edgeplaned.service
[Unit]
Description=EdgePlane Daemon
After=network-online.target

[Service]
ExecStart=/usr/local/bin/edgeplaned run --backend-url https://your-tower-host
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable --now edgeplaned
```

## Launching Agents

On any enrolled node:

```bash
edgeplane run claude [-p <profile>] [--mode interactive|headless]
edgeplane run codex  [-p <profile>]
```

`--mode headless` keeps the agent running without a terminal (for unattended execution). `--mode interactive` opens the ACP chat.

## Federated Attach

When an agent is running on a remote node, the web dashboard can still attach to it. The tower mediates the attach via the node's `edgeplaned` — you never need direct network access to the remote node.

```
Web browser → /api/attach-ws → edgeplane-tower → edgeplaned (remote node) → agent PTY
```

The agent's node must have `edgeplaned` running and registered. The attach-secret handshake is self-healing — if `edgeplaned` restarts, it re-registers and the next attach works automatically.

## Monitoring the Fleet

```bash
edgeplane agent list [--json]          # all registered agents + status
edgeplane agent status <agent-id>      # detailed status for one agent
edgeplane tui                          # full-screen real-time fleet view
```

The TUI Agents tab (`a`) shows each agent's node, runtime, and last-seen timestamp.

## Cron Dispatch

`edgeplaned` includes a cron dispatcher — useful for scheduled agent tasks:

```toml
# ~/.edgeplane/edgeplaned/cron.toml
[[job]]
name     = "nightly-review"
schedule = "0 2 * * *"          # 5-field cron, local time
session  = "default"
prompt   = "edgeplane run claude -- --task 'nightly code review'"
```

Reload after editing:
```bash
edgeplane agent cron reload
edgeplane agent cron list           # verify next fire times
```

## What's Next
- [Guides: Zellij Integration](/guides/zellij-integration/) — focus-free control for terminal agents
- [Architecture: Components](/architecture/components/) — how edgeplaned fits the stack
- [Reference: edgeplaned Daemon](/reference/edgeplaned-daemon/) — full daemon reference
```

- [ ] **Step 7: Create `guides/fleet-profiles-advanced.md`**

Full content:
```markdown
---
title: Personal Fleet Profiles (Advanced)
description: Managing operator profiles across multiple machines — push, pull, and switch profiles in a self-hosted EdgePlane setup.
---

This guide covers advanced profile workflows for operators running a self-hosted EdgePlane instance. If you just need to use profiles day-to-day, start with [Concepts: Profiles](/concepts/profiles/).

## Profile Storage Architecture

Profiles are stored server-side in `edgeplane-tower`'s database, scoped to the owning operator. The local machine caches a copy at `~/.edgeplane/profiles/<name>/`.

```
Server (edgeplane-tower)             Local cache
┌──────────────────────────┐         ┌─────────────────────────────────┐
│  profiles table          │◀───────▶│  ~/.edgeplane/profiles/         │
│    owner: operator-id    │  push/  │    coding/                      │
│    name: coding          │  pull   │      env                        │
│    env: {...}            │         │      CLAUDE.md                  │
│    instructions: {...}   │         │      config.json                │
└──────────────────────────┘         └─────────────────────────────────┘
```

`edgeplane run` always pulls the latest profile from the server before launching — the local cache is just a write-through layer.

## Creating a Profile from Scratch

```bash
edgeplane profile create research            # creates an empty profile
edgeplane profile edit research              # opens $EDITOR with the profile config

# Or build it manually:
mkdir -p ~/.edgeplane/profiles/research
cat > ~/.edgeplane/profiles/research/env <<EOF
EP_BASE_URL=https://your-tower-host
SOME_API_KEY_NAME=<your-key>
EOF
cat > ~/.edgeplane/profiles/research/CLAUDE.md <<EOF
You are in research mode. Focus on analysis and synthesis.
EOF
edgeplane profile push research              # upload to server
```

## Syncing Across Machines

On a second machine:

```bash
edgeplane auth login                         # authenticate to the same tower instance
edgeplane profile pull research              # download profile from server
edgeplane profile switch research
edgeplane run claude                         # launches with the research profile
```

The profile travels via the server — no direct machine-to-machine transfer needed.

## Versioning and Rollback

Each `push` creates a new version server-side. To see versions:

```bash
edgeplane profile versions research
```

To roll back:

```bash
edgeplane profile pull research --version 3
edgeplane profile push research             # re-push the old version as current
```

## Profile Secrets

For sensitive values (API keys, tokens), use the secrets broker rather than plain `env` entries:

```bash
# Register a secret with edgeplaned's secrets broker:
edgeplane secrets set MY_API_KEY "sk-..."

# Reference it in the profile env (value resolved at launch, never written to disk):
MY_API_KEY=secret:MY_API_KEY
```

The `secret:` prefix tells `edgeplaned` to resolve the value from the secrets broker at agent startup time.

## What's Next
- [Concepts: Profiles](/concepts/profiles/) — the profile model explained
- [Reference: CLI](/reference/cli/) — full `edgeplane profile` command reference
```

- [ ] **Step 8: Run scrubbing check on all six pages**

- [ ] **Step 9: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/guides/
git commit -m "docs(site): Guides — Web Dashboard, Zellij, Fleet, Profiles (new); Deployment + OIDC updated

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 6: Reference + ADRs — update 3 pages, create 2 new

**Files:**
- Modify: `site/src/content/docs/reference/cli.md`
- Modify: `site/src/content/docs/reference/edgeplaned-daemon.md`
- Modify: `site/src/content/docs/reference/command-map.md`
- Create: `site/src/content/docs/reference/zrpc-plugin.md`
- Create: `site/src/content/docs/adr/0005-unified-agent-launcher.md`

**Source material to read first:**
- Current versions of `cli.md`, `edgeplaned-daemon.md`, `command-map.md`
- CHANGELOG v0.13.0 (`launch` removal, all `run` runtimes, federated attach)

- [ ] **Step 1: Read source material**

```bash
cat /home/merlin/code/edgeplane/site/src/content/docs/reference/cli.md
cat /home/merlin/code/edgeplane/site/src/content/docs/reference/edgeplaned-daemon.md
cat /home/merlin/code/edgeplane/site/src/content/docs/reference/command-map.md
cat /home/merlin/code/edgeplane/CHANGELOG.md | head -300
```

- [ ] **Step 2: Update `cli.md`**

Key changes:
- Remove all `edgeplane launch` references — replaced by `edgeplane run` in v0.13.0
- Expand Agent Launch section to cover all `run` runtimes: `claude`, `codex`, `gemini`, `goose`, `openclaw`, `custom`
- Add `--mode` flag to `edgeplane run`: `interactive | headless | solo`
- Update Auth section: confirm `edgeplane auth login` writes to `~/.edgeplane/session.json`
- Add `edgeplane profile` commands section
- Add `edgeplane agent cron` commands section
- Add `edgeplane zrpc` commands section (install, status)
- Env var table: no `EP_TOKEN`, confirm `EP_BASE_URL` and `EP_OUTPUT`

The Agent Launch section should read:

```markdown
## Agent Launch

```bash
edgeplane run <runtime> [options]
```

| Runtime | Description |
|---------|-------------|
| `claude` | Claude Code with EdgePlane MCP wired in (ACP persistent session) |
| `codex` | OpenAI Codex CLI (driver agent) |
| `gemini` | Google Gemini CLI (driver agent) |
| `goose` | Goose (native, profile-scoped) |
| `openclaw` | OpenClaw (driver agent) |
| `custom` | Custom ACP agent (driver) |

Common flags:
```bash
edgeplane run claude [-p <profile>] [--mission <id>] [--mode interactive|headless|solo] [-- <extra-args>]
```

`edgeplane launch` was removed in v0.13.0. All agents now launch through `edgeplane run`.
```

- [ ] **Step 3: Update `edgeplaned-daemon.md`**

Key changes to add:
- **Federated Attach section**: explain that `edgeplaned` receives attach requests proxied from the tower — agents on remote nodes are reachable from any web dashboard
- **Self-heal attach-secret**: edgeplaned automatically re-registers and re-issues attach_secret on restart; the tower's `no-store` header ensures the secret is never cached
- **Phase 7 context** (without the "Phase N" internal numbering): "As of v0.13.x, `edgeplaned` supports mediated attach — the tower proxies WebSocket attach requests to the node daemon, eliminating the need for direct network access to agent nodes."
- Remove any references to EP_TOKEN

- [ ] **Step 4: Update `command-map.md`**

Remove:
- `edgeplane launch` and all its subcommands

Add:
- `edgeplane run <runtime>` and all runtimes
- `edgeplane profile list|show|switch|push|pull|create`
- `edgeplane agent cron list|describe|reload|history`
- `edgeplane zrpc install|status`

- [ ] **Step 5: Create `reference/zrpc-plugin.md`**

Full content:
```markdown
---
title: zRPC Plugin Reference
description: Configuration reference for the edgeplane-zrpc Zellij plugin — environment variables, pipe protocol, and permissions.
---

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `EDGEPLANE_ZRPC_PLUGIN_PATH` | Yes (to enable) | Absolute path to `edgeplane_zrpc.wasm`. If unset, `edgeplaned` uses the paste path (no behavior change). |
| `EDGEPLANE_ZRPC_SESSIONS` | Yes (to enable) | Comma-separated list of Zellij session names to watch. `edgeplaned` only loads the plugin for listed sessions. |

Both must be set for the plugin to activate. If either is unset, the feature is dormant.

## Permissions

The plugin requires these Zellij permissions (pre-seeded in `permissions.kdl` by `edgeplane zrpc install`):

| Permission | Why |
|------------|-----|
| `ReadApplicationState` | Read pane list and pane state |
| `ReadCliPipes` | Receive commands sent via `zellij pipe` |
| `WriteToStdin` | Inject text into pane PTY stdin |
| `ReadPaneContents` | Read scrollback buffer |
| `ChangeApplicationState` | Send focus events (optional, for focus-aware operations) |

The `permissions.kdl` file must exist in the Zellij cache directory **before the Zellij server starts**. Find the cache directory:

```bash
zellij setup --check | grep 'CACHE DIR'
```

The file format (note: raw absolute path, no `file:` prefix):

```kdl
"/absolute/path/to/edgeplane_zrpc.wasm" {
    ReadApplicationState
    ReadCliPipes
    WriteToStdin
    ReadPaneContents
}
```

## Pipe Protocol

The plugin communicates over the `edgeplane-zrpc` named Zellij pipe. All payloads are JSON.

### Commands (edgeplaned → plugin)

| `kind` | Fields | Description |
|--------|--------|-------------|
| `ping` | — | Health check |
| `inject` | `pane_id: number, text: string` | Inject text + newline into pane PTY stdin |
| `scrollback` | `pane_id: number, lines: number` | Request scrollback contents |
| `list_panes` | — | Request current pane manifest |

### Responses (plugin → edgeplaned)

| `kind` | Fields | Description |
|--------|--------|-------------|
| `pong` | — | Response to `ping` |
| `scrollback_response` | `pane_id: number, content: string` | Scrollback content |
| `pane_list` | `panes: [{id, title, is_focused}]` | Pane manifest |
| `pane_event` | `event: "exit"\|"focus_gained"\|"resize", pane_id: number` | Lifecycle event |

### Example: inject a prompt

```bash
# Sent by edgeplaned internally — shown here for debugging
zellij pipe --name edgeplane-zrpc -- '{"kind":"inject","pane_id":1,"text":"run tests"}'
```

## Build Details

The plugin is a Rust **bin crate** targeting `wasm32-wasip1`. This is required — a `cdylib` target on `wasm32-wasip1` with Rust 1.82+ produces a WASI reactor that exports `_initialize` but not `_start`. Zellij 0.44.x requires `_start` and rejects the plugin at instantiation with a silent hang on all `zellij pipe` calls.

Build:
```bash
cargo build --release --target wasm32-wasip1 -p edgeplane-zrpc
```

Verify `_start` is exported:
```bash
wasm-tools print target/wasm32-wasip1/release/edgeplane_zrpc.wasm | grep '(export "_start"'
```

## Logs

`edgeplaned` logs zRPC activity to its standard log stream. Filter:

```bash
journalctl --user -u edgeplaned -f | grep zrpc
```

Zellij's own log (for plugin instantiation failures):
```bash
tail -f "$(zellij setup --check | grep 'LOG DIR' | awk '{print $3}')/zellij.log" | grep edgeplane
```

## What's Next
- [Guides: Zellij Integration](/guides/zellij-integration/) — end-to-end setup walkthrough
- [Architecture: Components](/architecture/components/) — where the plugin fits
```

- [ ] **Step 6: Create `adr/0005-unified-agent-launcher.md`**

Full content:
```markdown
---
title: "ADR-0005: edgeplane run as the unified agent launcher"
description: Decision to retire edgeplane launch and unify all agent launch paths under edgeplane run.
---

**Status:** Accepted  
**Date:** 2026-05-29 (shipped in v0.13.0)

## Context

EdgePlane had two agent launch surfaces:
- `edgeplane run <runtime>` — the newer path for ACP-native runtimes (claude, codex, gemini)
- `edgeplane launch <agent>` — the older path for driver agents (openclaw, custom, and the original claude shim)

This split caused confusion:
- New users couldn't find all available runtimes in one place
- Gemini was a shim wrapped around `launch`; it worked inconsistently
- `openclaw` and `custom` were `launch`-only, not visible in `run --help`
- The generated lifecycle hooks invoked `edgeplane claude hook <event>` (a non-existent subcommand)

## Decision

Retire `edgeplane launch` entirely. All agents launch through `edgeplane run <runtime>`:

| Runtime | Was |
|---------|-----|
| `claude` | `edgeplane run claude` (unchanged) |
| `codex` | `edgeplane run codex` (unchanged) |
| `gemini` | `edgeplane run gemini` (was a shim over `launch`) |
| `goose` | `edgeplane run goose` (was `launch`-only) |
| `openclaw` | `edgeplane run openclaw` (was `launch`-only) |
| `custom` | `edgeplane run custom` (was `launch`-only) |

`edgeplane launch <anything>` now returns an unrecognized-subcommand error.

## Consequences

**Positive:**
- Single entry point — operators learn one command
- All runtimes visible in `edgeplane run --help`
- Claude lifecycle hooks now work (`edgeplane run claude hook <event>` is valid)
- Gemini, openclaw, custom get first-class `run` parity (mode flags, profile flags, mission flag)
- Internal code unified: one `RunDispatch` replaces two dispatch paths

**Negative:**
- Breaking change for any scripts using `edgeplane launch`
- Migration: replace `edgeplane launch <agent>` with `edgeplane run <agent>`

## Alternatives Considered

**Keep both surfaces:** Rejected — the confusion compounds as new runtimes are added. A single canonical surface is worth the one-time migration cost.

**Rename `launch` to `run`:** What was done.
```

- [ ] **Step 7: Update `reference/real-time.md` — add `progress` event type**

Read the current page, then add `progress` to the SSE event types table if missing. As of v0.13.x the feed emits `{type: "progress", ...}` frames during agent execution. The row to add:

| Event type | When |
|------------|------|
| `progress` | Agent execution progress frames — emitted during active ACP sessions |

- [ ] **Step 8: Run scrubbing check on all pages**

- [ ] **Step 9: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/reference/ site/src/content/docs/adr/
git commit -m "docs(site): Reference — zRPC plugin (new), CLI/daemon/command-map/real-time updated; ADR-0005

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

---

## Task 7: Consistency Pass — nav config, landing page, cross-links, build verify

**Files:**
- Modify: `site/astro.config.mjs`
- Modify: `site/src/content/docs/index.mdx`
- Read: all pages written in Tasks 2–6

**Depends on:** Tasks 2, 3, 4, 5, 6 all complete.

- [ ] **Step 1: Add architecture to the sidebar in `astro.config.mjs`**

Current sidebar:
```js
sidebar: [
  { label: 'Getting Started', autogenerate: { directory: 'getting-started' } },
  { label: 'Concepts', autogenerate: { directory: 'concepts' } },
  { label: 'Architecture', autogenerate: { directory: 'architecture' } },  // ← ADD THIS
  { label: 'Guides', autogenerate: { directory: 'guides' } },
  { label: 'Reference', autogenerate: { directory: 'reference' } },
  { label: 'ADRs', autogenerate: { directory: 'adr' } },
],
```

Check if `Architecture` entry already exists — if yes, verify the directory string is `'architecture'`.

- [ ] **Step 2: Update `index.mdx` landing page**

Update the hero tagline and bullet list to reflect current capabilities. The hero should read:

```mdx
---
title: EdgePlane
description: Control plane for AI agents and human collaborators.
template: splash
hero:
  tagline: The coordination layer for AI agents. Persistent sessions, governed artifacts, and distributed execution — without a scheduler.
  actions:
    - text: Quick Start
      link: /getting-started/quick-start/
      icon: right-arrow
      variant: primary
    - text: Architecture
      link: /architecture/overview/
      icon: right-arrow
```

Update the "What is EdgePlane?" bullet list to include ACP, MeshTask, and Web Dashboard. Current bullets are stale — replace with:

```
- **Domains & Missions** — organizational units that scope work, tools, and governance
- **ACP Sessions** — persistent agent processes that survive crashes and reconnect
- **Overlap Detection** — fuzzy + vector similarity before task and artifact creation
- **Artifact Ledger** — every output recorded, SHA-256 hashed, vector-indexed in Postgres
- **MeshTask Execution** — agents claim and complete work from a shared queue, no central scheduler
- **Web Dashboard** — React UI for fleet monitoring, live event feed, and ACP conversation
- **Governance & Approvals** — HMAC-signed approval tokens, versioned policy lifecycle
- **Zellij Integration** — focus-free terminal control via the edgeplane-zrpc plugin
```

- [ ] **Step 3: Audit cross-links in all new pages**

For each new page, verify every internal link (`[text](/path/)`) resolves to a page that actually exists:

```bash
# List all internal links across the new pages
grep -rh '\[.*\](/.*/)' /home/merlin/code/edgeplane/site/src/content/docs/ | sort -u
```

For each link, confirm the target file exists:
```bash
find /home/merlin/code/edgeplane/site/src/content/docs -name "*.md" | sort
```

Fix any broken links before building.

- [ ] **Step 4: Verify all new pages have required frontmatter**

```bash
for f in $(find /home/merlin/code/edgeplane/site/src/content/docs -name "*.md" -newer /home/merlin/code/edgeplane/site/src/content/docs/index.mdx); do
  if ! grep -q "^title:" "$f"; then
    echo "MISSING title: $f"
  fi
  if ! grep -q "^description:" "$f"; then
    echo "MISSING description: $f"
  fi
done
```

Fix any missing frontmatter.

- [ ] **Step 5: Build the site and verify no errors**

```bash
cd /home/merlin/code/edgeplane/site && bun run build 2>&1
```

Expected: build completes with no errors. Broken internal links cause build failures in Starlight — fix any that appear.

- [ ] **Step 6: Run final scrubbing check across all new/modified pages**

```bash
grep -r "epyc\|cloud0\|kai\|aria-memory-pg\|\.ts\.net\|EP_TOKEN\|edgeplane launch\|/workspace/cache/zellij" \
  /home/merlin/code/edgeplane/site/src/content/docs/ 2>/dev/null
```

Expected: no output. If any matches, fix them.

- [ ] **Step 7: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/astro.config.mjs site/src/content/docs/index.mdx
git commit -m "docs(site): consistency pass — add architecture to nav, update landing page, fix cross-links

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
```

- [ ] **Step 8: Final page count check**

```bash
find /home/merlin/code/edgeplane/site/src/content/docs -name "*.md" -o -name "*.mdx" | wc -l
```

Expected: ~13 more than the baseline from Task 1.

- [ ] **Step 9: Push to deploy**

```bash
cd /home/merlin/code/edgeplane && git push origin main
```

CI builds the Starlight site and deploys to Cloudflare Pages. Monitor the GitHub Actions run for any build failures.
