# GitHub Pages — Starlight Docs Site Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a public OSS documentation site for Edgeplane using Starlight (Astro), deployed to GitHub Pages via GitHub Actions, with all content migrated and scrubbed of personal/fleet-specific references.

**Architecture:** A standalone `site/` directory at the repo root houses the Astro + Starlight project. Content is authored in `site/src/content/docs/` as Markdown files adapted from the existing `docs/` tree. GitHub Actions builds and deploys to GitHub Pages on every push to `main` that touches `site/` or core docs.

**Tech Stack:** Astro 5, `@astrojs/starlight` 0.34+, Node 20, GitHub Actions (`actions/deploy-pages@v4`)

---

## File Map

### Created
| Path | Purpose |
|------|---------|
| `site/package.json` | Astro + Starlight deps |
| `site/tsconfig.json` | Astro TS config |
| `site/astro.config.mjs` | Starlight config, nav sidebar, site URL |
| `site/public/favicon.svg` | Site favicon (MC logo placeholder) |
| `site/public/CNAME` | Custom domain placeholder (empty until domain set) |
| `site/src/content/docs/index.mdx` | Landing page |
| `site/src/content/docs/getting-started/installation.md` | Install edgeplane + edgeplaned |
| `site/src/content/docs/getting-started/quick-start.md` | Docker Compose quickstart |
| `site/src/content/docs/getting-started/agent-setup.md` | Connect an agent (from AGENT-INSTALL.md) |
| `site/src/content/docs/concepts/overview.md` | What is MC (from README + philosophy) |
| `site/src/content/docs/concepts/missions-klusters-tasks.md` | Core entities explained (from entities.md) |
| `site/src/content/docs/concepts/entity-reference.md` | Full entity reference (from entities.md) |
| `site/src/content/docs/concepts/philosophy.md` | EDGEPLANE_PHILOSOPHY.md |
| `site/src/content/docs/architecture/overview.md` | System overview (from architecture.md) |
| `site/src/content/docs/architecture/persistence.md` | Persistence model (from architecture.md) |
| `site/src/content/docs/architecture/ephemeral-agents.md` | Ephemeral task agents (from design/ephemeral-task-subagents.md, scrubbed) |
| `site/src/content/docs/guides/deployment.md` | Linux VM + Compose deploy (from guides/DEPLOYMENT.md, scrubbed) |
| `site/src/content/docs/guides/oidc.md` | OIDC setup (from guides/OIDC.md, scrubbed title) |
| `site/src/content/docs/guides/agent-setup.md` | Full agent install guide (from guides/AGENT-INSTALL.md) |
| `site/src/content/docs/guides/upgrading.md` | Release upgrade checklist |
| `site/src/content/docs/reference/cli.md` | edgeplane + edgeplaned CLI reference (from reference/MC-RUST.md, scrubbed) |
| `site/src/content/docs/reference/command-map.md` | Full command map (from reference/COMMAND-MAP.md) |
| `site/src/content/docs/reference/edgeplaned-daemon.md` | edgeplaned daemon reference (from reference/MCD.md, scrubbed) |
| `site/src/content/docs/reference/real-time.md` | SSE real-time events (from reference/REAL-TIME.md) |
| `site/src/content/docs/reference/ai-console.md` | AI Console protocol (from reference/AI-CONSOLE.md) |
| `site/src/content/docs/adr/0001-project-catalog.md` | ADR 0001 |
| `site/src/content/docs/adr/0002-source-of-truth.md` | ADR 0002 |
| `site/src/content/docs/adr/0003-cli-hierarchy.md` | ADR 0003 |
| `.github/workflows/deploy-docs.yml` | GitHub Actions deploy workflow |

---

## Scrub Reference

Before migrating content, apply these find-and-replace rules to each file. Check each one — not all references appear in every file.

| Find | Replace with |
|------|-------------|
| `aria goose` / `Aria` (agent name) | remove or rephrase as "the configured triage command" / generic agent name |
| `Merlin` (person) | remove |
| `excalibur`, `epyc`, `cloud0`, `kai` (hostnames) | `your-node` or generic |
| `Infisical` | "your secrets manager" or remove |
| `Tailscale` (as prerequisite) | "network access to the MC backend" |
| `authentik` in prose | "your OIDC provider" |
| `rustfs` in prose | "S3-compatible object storage" |
| `Aria fleet` / `Aria-specific` | remove or rephrase |
| `zellij_hosted … Aria fleet` in MCD.md | remove parenthetical "Aria fleet" |
| `aria-rs` absorbed-responsibilities section in MCD.md | remove entire section (internal implementation history) |
| `Author: Aria (mc-engineer) with Merlin` in ephemeral-agents | remove author line |
| `leaked an Aria-specific operational pattern` in entities.md | `leaked a deployment-specific operational pattern` |

---

## Task 1: Scaffold Starlight site

**Files:**
- Create: `site/package.json`
- Create: `site/tsconfig.json`
- Create: `site/astro.config.mjs`
- Create: `site/public/favicon.svg`
- Create: `site/public/CNAME`
- Create: `site/src/content/docs/.gitkeep`

- [ ] **Step 1: Create `site/package.json`**

```json
{
  "name": "edgeplane-docs",
  "type": "module",
  "version": "0.1.0",
  "scripts": {
    "dev": "astro dev",
    "build": "astro build",
    "preview": "astro preview",
    "check": "astro check"
  },
  "dependencies": {
    "@astrojs/starlight": "^0.34.0",
    "astro": "^5.0.0"
  }
}
```

- [ ] **Step 2: Create `site/tsconfig.json`**

```json
{
  "extends": "astro/tsconfigs/strict"
}
```

- [ ] **Step 3: Install dependencies**

```bash
cd /home/merlin/code/edgeplane/site && npm install
```

Expected: `node_modules/` created, `package-lock.json` written.

- [ ] **Step 4: Create `site/astro.config.mjs`**

```js
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://ryanmerlin.github.io',
  base: '/edgeplane',
  integrations: [
    starlight({
      title: 'Edgeplane',
      description: 'Control plane for AI agents and human collaborators — structured missions, durable task ownership, and governed artifact publication.',
      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/RyanMerlin/edgeplane' },
      ],
      sidebar: [
        {
          label: 'Getting Started',
          items: [
            { label: 'Installation', slug: 'getting-started/installation' },
            { label: 'Quick Start', slug: 'getting-started/quick-start' },
            { label: 'Agent Setup', slug: 'getting-started/agent-setup' },
          ],
        },
        {
          label: 'Concepts',
          items: [
            { label: 'What is Edgeplane?', slug: 'concepts/overview' },
            { label: 'Missions, Klusters & Tasks', slug: 'concepts/missions-klusters-tasks' },
            { label: 'Entity Reference', slug: 'concepts/entity-reference' },
            { label: 'Philosophy', slug: 'concepts/philosophy' },
          ],
        },
        {
          label: 'Architecture',
          items: [
            { label: 'System Overview', slug: 'architecture/overview' },
            { label: 'Persistence Model', slug: 'architecture/persistence' },
            { label: 'Ephemeral Task Agents', slug: 'architecture/ephemeral-agents' },
          ],
        },
        {
          label: 'Guides',
          items: [
            { label: 'Deployment', slug: 'guides/deployment' },
            { label: 'OIDC Authentication', slug: 'guides/oidc' },
            { label: 'Agent Setup', slug: 'guides/agent-setup' },
            { label: 'Upgrading', slug: 'guides/upgrading' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'edgeplane & edgeplaned CLI', slug: 'reference/cli' },
            { label: 'Command Map', slug: 'reference/command-map' },
            { label: 'edgeplaned Daemon', slug: 'reference/edgeplaned-daemon' },
            { label: 'Real-Time Events', slug: 'reference/real-time' },
            { label: 'AI Console', slug: 'reference/ai-console' },
          ],
        },
        {
          label: 'ADRs',
          items: [
            { label: 'ADR-0001: Project Catalog', slug: 'adr/0001-project-catalog' },
            { label: 'ADR-0002: Source of Truth', slug: 'adr/0002-source-of-truth' },
            { label: 'ADR-0003: CLI Hierarchy', slug: 'adr/0003-cli-hierarchy' },
          ],
        },
      ],
    }),
  ],
});
```

- [ ] **Step 5: Create placeholder favicon**

Create `site/public/favicon.svg` with a minimal SVG:

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
  <text y=".9em" font-size="90">🛰️</text>
</svg>
```

- [ ] **Step 6: Create empty CNAME placeholder**

Create `site/public/CNAME` as an empty file. Fill in a domain later when one is set.

- [ ] **Step 6b: Create `site/.gitignore`**

```
node_modules/
dist/
.astro/
```

- [ ] **Step 7: Verify Astro config parses**

```bash
cd /home/merlin/code/edgeplane/site && npm run check 2>&1 | head -20
```

Expected: warns about missing content files (no pages yet) — that is fine at this stage.

- [ ] **Step 8: Commit scaffold**

```bash
cd /home/merlin/code/edgeplane
git add site/
git commit -m "feat(docs): scaffold Starlight site"
```

---

## Task 2: Landing page

**Files:**
- Create: `site/src/content/docs/index.mdx`

- [ ] **Step 1: Create the landing page**

```mdx
---
title: Edgeplane
description: Control plane for AI agents and human collaborators.
template: splash
hero:
  tagline: Kubernetes orchestrates containers. Edgeplane orchestrates agents, missions, and knowledge.
  actions:
    - text: Get Started
      link: /edgeplane/getting-started/installation/
      icon: right-arrow
      variant: primary
    - text: View on GitHub
      link: https://github.com/RyanMerlin/edgeplane
      icon: external
---

## What is Edgeplane?

AI agents can write code, run tools, and reason over architecture. What they can't do is **coordinate**.

Without a shared system of record, parallel agents duplicate effort, diverge on state, and collide on artifacts with no resolution path.

Edgeplane is a **control plane for AI agents and human collaborators**. It provides:

- **Missions & Klusters** — organizational units that scope knowledge, tools, permissions, and governance
- **Overlap Detection** — fuzzy + vector similarity runs before task and artifact creation; collisions surface before damage occurs
- **Artifact Ledger** — every mutation recorded in Postgres, vector-indexed for search, committed to Git with full provenance
- **MCP-Native Interface** — standard MCP stdio tools; works with any MCP-compatible agent
- **Governance & Approvals** — versioned policy lifecycle, role-based access, HMAC-signed approval tokens
- **Persistent Agent Sessions** — `edgeplaned` manages long-running agent processes; sessions survive crashes and reconnects
- **Semantic Search** — tasks, docs, and klusters are vector-indexed (pgvector) for similarity and hybrid search
```

- [ ] **Step 2: Verify build includes landing page**

```bash
cd /home/merlin/code/edgeplane/site && npm run build 2>&1 | tail -10
```

Expected: build succeeds, `dist/edgeplane/index.html` exists.

- [ ] **Step 3: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/index.mdx
git commit -m "feat(docs): add landing page"
```

---

## Task 3: Getting Started content

**Files:**
- Create: `site/src/content/docs/getting-started/installation.md`
- Create: `site/src/content/docs/getting-started/quick-start.md`
- Create: `site/src/content/docs/getting-started/agent-setup.md`

Source: `docs/guides/AGENT-INSTALL.md`, `docs/reference/MC-RUST.md` (Install section), `README.md` (Docker Compose quickstart), `docker-compose.quickstart.yml`.

- [ ] **Step 1: Create `installation.md`**

Adapt from `docs/reference/MC-RUST.md` install section. Content to include: install script (Linux/macOS), Windows PowerShell, build-from-source. Add Starlight frontmatter.

```markdown
---
title: Installation
description: Install the edgeplane CLI and edgeplaned daemon on your machine.
---

## Install edgeplane (recommended)

**Linux / macOS** — downloads a prebuilt binary, falls back to a source build:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/install-edgeplane.sh)
```

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/bootstrap-edgeplane.ps1 | iex
```

## Build from source

Requires the Rust toolchain (`rustup`):

```bash
git clone https://github.com/RyanMerlin/edgeplane.git
cd edgeplane

# edgeplane CLI
cd crates/edgeplane && cargo build --release
cp target/release/edgeplane ~/.local/bin/edgeplane

# edgeplaned daemon (optional — needed for agent supervision on a node)
cd ../edgeplaned && cargo build --release
cp target/release/edgeplaned ~/.local/bin/edgeplaned

# edgeplane-tower (optional — needed to self-host the backend)
cd ../edgeplane-tower && cargo build --release
cp target/release/edgeplane-tower ~/.local/bin/edgeplane-tower
```

## Environment

| Variable | Purpose | Default |
|----------|---------|---------|
| `EP_BASE_URL` | Backend HTTP base URL | `http://localhost:8008` |
| `EP_TOKEN` | Bearer auth token | unset |

Set these in your shell profile or pass them per-command.
```

- [ ] **Step 2: Create `quick-start.md`**

Adapt from README Docker Compose section and `docker-compose.quickstart.yml`.

```markdown
---
title: Quick Start
description: Run Edgeplane locally with Docker Compose in under five minutes.
---

The fastest way to run Edgeplane is with Docker Compose. This starts the API, Postgres, and S3-compatible storage as a single stack.

## Prerequisites

- Docker + Docker Compose v2
- `edgeplane` CLI installed (see [Installation](./installation))

## Start the stack

```bash
git clone https://github.com/RyanMerlin/edgeplane.git
cd edgeplane
docker compose -f docker-compose.quickstart.yml up -d
```

This starts:
- `edgeplane-api` — the FastAPI backend on port `8008`
- `postgres` — Postgres 16 with pgvector
- `rustfs` — S3-compatible object storage on port `9000`

## Verify it's running

```bash
curl http://localhost:8008/health
# {"status":"ok"}
```

## Connect edgeplane

```bash
export EP_BASE_URL=http://localhost:8008
export EP_TOKEN=changeme          # matches EP_TOKEN in docker-compose.quickstart.yml

edgeplane status
# Edgeplane vX.Y.Z — connected
```

## Next steps

- [Set up an agent](./agent-setup) to start claiming tasks
- [Full deployment guide](../guides/deployment) for production setups
```

- [ ] **Step 3: Create `agent-setup.md`**

Copy `docs/guides/AGENT-INSTALL.md` content, add Starlight frontmatter. This file is already clean — no scrubbing needed.

```markdown
---
title: Agent Setup
description: Connect an AI agent to Edgeplane in three steps.
---
```

Then paste the full content of `docs/guides/AGENT-INSTALL.md` below the frontmatter.

- [ ] **Step 4: Build and spot-check**

```bash
cd /home/merlin/code/edgeplane/site && npm run build 2>&1 | tail -5
```

Expected: build succeeds, three pages under `dist/edgeplane/getting-started/`.

- [ ] **Step 5: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/getting-started/
git commit -m "feat(docs): add Getting Started section"
```

---

## Task 4: Concepts content

**Files:**
- Create: `site/src/content/docs/concepts/overview.md`
- Create: `site/src/content/docs/concepts/missions-klusters-tasks.md`
- Create: `site/src/content/docs/concepts/entity-reference.md`
- Create: `site/src/content/docs/concepts/philosophy.md`

**Scrubs required:**
- `entity-reference.md`: replace `leaked an Aria-specific operational pattern` → `leaked a deployment-specific operational pattern`
- `philosophy.md`: no scrubs needed

- [ ] **Step 1: Create `overview.md`**

Adapt from `README.md` (Core Capabilities section) and philosophy intro.

```markdown
---
title: What is Edgeplane?
description: Edgeplane is a control plane for AI agents and human collaborators.
---

AI agents can write code, run tools, and reason over architecture.
What they cannot do is **coordinate**.

Without a shared system of record, parallel agents duplicate effort, diverge on state, and collide on artifacts. There is no overlap detection, no structured ownership, no audit trail, no governance boundary.

Edgeplane is the coordination layer. It is a control plane for AI agents and human collaborators operating against shared, durable, governed state.

> It is not a workflow runner. It is not a pipeline framework. It is not a chatbot UI.
>
> Kubernetes orchestrates containers. Edgeplane orchestrates agents, missions, and knowledge.

## Core capabilities

| Capability | What it does |
|-----------|-------------|
| **Missions & Klusters** | Organizational units that scope knowledge, tools, permissions, and governance. Agents and humans switch profiles without losing context or integrity. |
| **Overlap Detection** | Fuzzy + vector similarity runs before task and artifact creation. Collisions surface as `overlap_suggestions` in the API response before damage occurs. |
| **Artifact Ledger** | Every mutation recorded in Postgres, vector-indexed for search, and committed to Git with full provenance metadata on publish. |
| **MCP-Native Interface** | Standard MCP stdio tools: `search_tasks`, `get_overlap_suggestions`, `load_kluster_workspace`, `publish_pending_ledger_events`. Works with any MCP-compatible agent. |
| **Governance & Approvals** | Versioned policy lifecycle (draft → active → rollback), role-based access (Admin / Contributor / Viewer), HMAC-signed approval tokens on sensitive mutations. |
| **Persistent Agent Sessions** | `edgeplaned` manages long-running agent processes on each node via ACP. Sessions survive crashes and reconnects. |
| **Semantic Search** | Tasks, docs, and klusters are vector-indexed (pgvector) for similarity and hybrid search. |
| **S3-Backed File Persistence** | Artifact content stored in S3-compatible object storage. Swap in AWS S3 or MinIO with env vars — no code changes. |
| **Chat Integration** | Slack-native notifications, task creation from threads, approval workflows, and in-channel search. |

## Next steps

- [Missions, Klusters & Tasks](./missions-klusters-tasks) — the three core primitives
- [Entity Reference](./entity-reference) — full schema-level definitions
- [Philosophy](./philosophy) — the why behind the design
```

- [ ] **Step 2: Create `missions-klusters-tasks.md`**

Adapt Mission, Kluster, Task, and MeshTask sections from `docs/architecture/entities.md`. Keep the content, add frontmatter, apply the Aria scrub.

```markdown
---
title: Missions, Klusters & Tasks
description: The three core organizational primitives in Edgeplane.
---
```

Then include the Mission, Kluster, Task, and MeshTask sections from `docs/architecture/entities.md` — these four are the most important for new users to understand. Apply the scrub: `leaked an Aria-specific operational pattern` → `leaked a deployment-specific operational pattern`.

- [ ] **Step 3: Create `entity-reference.md`**

Copy the full content of `docs/architecture/entities.md` with frontmatter. Apply the single scrub noted above.

```markdown
---
title: Entity Reference
description: Canonical definitions for all Edgeplane entities — Mission, Kluster, Task, MeshTask, Artifact, Agent, and Session types.
---
```

- [ ] **Step 4: Create `philosophy.md`**

Copy `EDGEPLANE_PHILOSOPHY.md` with frontmatter. No scrubs needed.

```markdown
---
title: Philosophy
description: The design philosophy behind Edgeplane — why coordination is the missing layer for AI agents.
---
```

- [ ] **Step 5: Build and check**

```bash
cd /home/merlin/code/edgeplane/site && npm run build 2>&1 | tail -5
```

Expected: build succeeds, four pages under `dist/edgeplane/concepts/`.

- [ ] **Step 6: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/concepts/
git commit -m "feat(docs): add Concepts section"
```

---

## Task 5: Architecture content

**Files:**
- Create: `site/src/content/docs/architecture/overview.md`
- Create: `site/src/content/docs/architecture/persistence.md`
- Create: `site/src/content/docs/architecture/ephemeral-agents.md`

**Scrubs required:**
- `overview.md`: `Calls aria goose (local Qwen3.6-27B) for categorization` → `invokes the configured \`task_worker_surface_command\` for categorization`
- `ephemeral-agents.md`: remove `Author: Aria (mc-engineer) with Merlin` line; replace `aria-mc-engineer` example → `my-agent-profile`; replace `Aria (mc-engineer)` with generic agent name in table examples

- [ ] **Step 1: Create `overview.md`**

Adapt the Core Runtime, Ephemeral Task Subagents (claim/triage/capability/audit paragraphs), and Architecture diagram from `docs/architecture/architecture.md`. Apply the `aria goose` scrub.

```markdown
---
title: System Overview
description: How Edgeplane's runtime components fit together.
---
```

Include: Core Runtime section, Publish Flow section (numbered list), and the Ephemeral Task Subagents section (scrubbed). Omit the ASCII architecture diagram if it doesn't render cleanly in Starlight — or keep it in a code block.

- [ ] **Step 2: Create `persistence.md`**

Adapt the Persistence Model section from `docs/architecture/architecture.md`.

```markdown
---
title: Persistence Model
description: How Edgeplane uses Postgres, S3, and Git as a three-tier persistence stack.
---
```

Include: Persistence Model bullet list, Publish Flow, and the repo_connections/bindings/routes table.

- [ ] **Step 3: Create `ephemeral-agents.md`**

Adapt `docs/design/ephemeral-task-subagents.md`. Apply scrubs: remove author line, replace fleet-specific agent name examples with generic `my-agent-profile`.

```markdown
---
title: Ephemeral Task Agents
description: How edgeplaned spawns and supervises ephemeral agent processes for mesh task execution.
---
```

- [ ] **Step 4: Build and check**

```bash
cd /home/merlin/code/edgeplane/site && npm run build 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/architecture/
git commit -m "feat(docs): add Architecture section"
```

---

## Task 6: Guides content

**Files:**
- Create: `site/src/content/docs/guides/deployment.md`
- Create: `site/src/content/docs/guides/oidc.md`
- Create: `site/src/content/docs/guides/agent-setup.md`
- Create: `site/src/content/docs/guides/upgrading.md`

**Scrubs required:**
- `deployment.md`: `rustfs-init service` → `S3 init service`; `http://<rustfs-host>:<port>` placeholder is already generic — keep it; `authentik` env var prose → "your OIDC provider"; remove the sentence "Bucket bootstrap is handled by the one-shot `rustfs-init` service using Python/boto3 (no `minio/edgeplane` dependency)" and replace with "Bucket bootstrap is handled by the S3 init service included in the Compose stack."
- `oidc.md`: Change title from "Edgeplane OIDC (Authentik)" to "OIDC Authentication"; `authentik-host` placeholders are already generic angle-bracket vars — keep them; add a note that `authentik` is used in examples but any OIDC provider works.

- [ ] **Step 1: Create `deployment.md`**

Copy `docs/guides/DEPLOYMENT.md`, add frontmatter, apply scrubs.

```markdown
---
title: Deployment
description: Deploy Edgeplane on a Linux VM or with the Docker Compose stack.
---
```

Apply scrubs listed above.

- [ ] **Step 2: Create `oidc.md`**

Copy `docs/guides/OIDC.md`, add frontmatter, apply scrubs.

```markdown
---
title: OIDC Authentication
description: Configure OIDC JWT authentication alongside static token auth for MCP compatibility.
---

Edgeplane supports OIDC JWT validation while keeping static token auth for MCP compatibility. The examples below use Authentik as the identity provider, but any standards-compliant OIDC provider works.
```

Then paste the rest of the OIDC.md content below.

- [ ] **Step 3: Create `agent-setup.md`**

This is the same source as `getting-started/agent-setup.md` (AGENT-INSTALL.md) but in the Guides section for operators configuring agents. Add frontmatter:

```markdown
---
title: Agent Setup
description: Full guide to installing edgeplane and connecting AI agents to a Edgeplane instance.
---
```

Paste full AGENT-INSTALL.md content.

- [ ] **Step 4: Create `upgrading.md`**

Copy `docs/guides/RELEASE-UPGRADE-CHECKLIST.md`, add frontmatter.

```markdown
---
title: Upgrading
description: Release upgrade checklist for schema, auth, and deployment changes.
---
```

- [ ] **Step 5: Build and check**

```bash
cd /home/merlin/code/edgeplane/site && npm run build 2>&1 | tail -5
```

- [ ] **Step 6: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/guides/
git commit -m "feat(docs): add Guides section"
```

---

## Task 7: Reference content

**Files:**
- Create: `site/src/content/docs/reference/cli.md`
- Create: `site/src/content/docs/reference/command-map.md`
- Create: `site/src/content/docs/reference/edgeplaned-daemon.md`
- Create: `site/src/content/docs/reference/real-time.md`
- Create: `site/src/content/docs/reference/ai-console.md`

**Scrubs required:**
- `edgeplaned-daemon.md` (from `reference/MCD.md`):
  - In `AgentRuntime` list: `zellij_hosted (long-running agents hosted in a Zellij pane — Aria fleet; signals via edgeplane agent signal)` → `zellij_hosted (long-running agents hosted in a Zellij pane; signals via edgeplane agent signal)`
  - Remove the entire "Absorbed responsibilities (daemon-absorption plan)" section — it's internal implementation history with Aria-specific version notes.
  - Prerequisites: remove `Tailscale (or direct network access…)` bullet — replace with just `Network access to the MC backend`

- [ ] **Step 1: Create `cli.md`**

Copy `docs/reference/MC-RUST.md`, add frontmatter. No scrubs needed.

```markdown
---
title: edgeplane & edgeplaned CLI
description: Reference for the edgeplane CLI, edgeplaned daemon, and edgeplane-tower binary.
---
```

- [ ] **Step 2: Create `command-map.md`**

Copy `docs/reference/COMMAND-MAP.md`, add frontmatter.

```markdown
---
title: Command Map
description: Complete edgeplane CLI command hierarchy.
---
```

- [ ] **Step 3: Create `edgeplaned-daemon.md`**

Copy `docs/reference/MCD.md`, add frontmatter, apply scrubs listed above.

```markdown
---
title: edgeplaned Daemon
description: Reference for the edgeplaned agent work loop daemon — install, configure, and operate.
---
```

Scrubs to apply:
1. Remove the line `— Aria fleet` from the `zellij_hosted` runtime description.
2. Delete the entire section starting with `### Absorbed responsibilities (daemon-absorption plan)` through the end of that section (ending before `---`).
3. Replace the Tailscale prerequisite bullet with `- Network access to the MC backend`.

- [ ] **Step 4: Create `real-time.md`**

Copy `docs/reference/REAL-TIME.md`, add frontmatter. No scrubs needed.

```markdown
---
title: Real-Time Events
description: SSE event stream schema, rate limiting, and backoff semantics.
---
```

- [ ] **Step 5: Create `ai-console.md`**

Copy `docs/reference/AI-CONSOLE.md`, add frontmatter. No scrubs needed.

```markdown
---
title: AI Console
description: AI Console protocol — session creation, turn submission, and pending action approval.
---
```

- [ ] **Step 6: Build and check**

```bash
cd /home/merlin/code/edgeplane/site && npm run build 2>&1 | tail -5
```

- [ ] **Step 7: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/reference/
git commit -m "feat(docs): add Reference section"
```

---

## Task 8: ADRs content

**Files:**
- Create: `site/src/content/docs/adr/0001-project-catalog.md`
- Create: `site/src/content/docs/adr/0002-source-of-truth.md`
- Create: `site/src/content/docs/adr/0003-cli-hierarchy.md`

No scrubs required — all three ADRs are already generic.

- [ ] **Step 1: Create `0001-project-catalog.md`**

Copy `docs/adr/0001-project-catalog.md`, add frontmatter.

```markdown
---
title: "ADR-0001: Project Catalog"
description: Introduce machine-readable YAML catalog as a navigational layer for docs and agents.
---
```

- [ ] **Step 2: Create `0002-source-of-truth.md`**

Copy `docs/adr/0002-source-of-truth-boundaries.md`, add frontmatter.

```markdown
---
title: "ADR-0002: Source-of-Truth Boundaries"
description: Define which layer owns which class of information — catalog, Markdown, OpenAPI, code.
---
```

- [ ] **Step 3: Create `0003-cli-hierarchy.md`**

Copy `docs/adr/0003-edgeplane-cli-hierarchy-hard-cutover.md`, add frontmatter.

```markdown
---
title: "ADR-0003: CLI Hierarchy Hard Cutover"
description: Adopt a structured top-level command hierarchy for edgeplane and remove organic top-level sprawl.
---
```

- [ ] **Step 4: Build and check**

```bash
cd /home/merlin/code/edgeplane/site && npm run build 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
cd /home/merlin/code/edgeplane
git add site/src/content/docs/adr/
git commit -m "feat(docs): add ADRs section"
```

---

## Task 9: GitHub Actions deploy workflow

**Files:**
- Create: `.github/workflows/deploy-docs.yml`

- [ ] **Step 1: Enable GitHub Pages in repo settings (manual)**

In the GitHub repo UI: Settings → Pages → Source → **GitHub Actions**. This must be done before the workflow can deploy.

- [ ] **Step 2: Create `.github/workflows/deploy-docs.yml`**

```yaml
name: Deploy Docs to GitHub Pages

on:
  push:
    branches:
      - main
    paths:
      - 'site/**'
      - 'docs/**'
      - '.github/workflows/deploy-docs.yml'
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages
  cancel-in-progress: false

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm
          cache-dependency-path: site/package-lock.json

      - name: Install dependencies
        run: cd site && npm ci

      - name: Build site
        run: cd site && npm run build

      - name: Upload Pages artifact
        uses: actions/upload-pages-artifact@v3
        with:
          path: site/dist

  deploy:
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - name: Deploy to GitHub Pages
        id: deployment
        uses: actions/deploy-pages@v4
```

- [ ] **Step 3: Verify workflow file is valid YAML**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/deploy-docs.yml'))" && echo "valid"
```

Expected: `valid`

- [ ] **Step 4: Commit and push**

```bash
cd /home/merlin/code/edgeplane
git add .github/workflows/deploy-docs.yml
git commit -m "feat(docs): add GitHub Pages deploy workflow"
git push origin main
```

- [ ] **Step 5: Verify GitHub Actions run**

```bash
aria gh workflow status edgeplane 2>/dev/null || gh run list --repo RyanMerlin/edgeplane --workflow deploy-docs.yml --limit 3
```

Expected: a run appears for the `deploy-docs.yml` workflow. Wait for it to complete.

- [ ] **Step 6: Confirm site is live**

```bash
curl -sI https://ryanmerlin.github.io/edgeplane/ | head -5
```

Expected: `HTTP/2 200`

---

## Task 10: Final build verification

- [ ] **Step 1: Full clean build**

```bash
cd /home/merlin/code/edgeplane/site && npm run build 2>&1
```

Expected: exits 0, no errors. Warnings about missing alt text or similar are acceptable.

- [ ] **Step 2: Astro type check**

```bash
cd /home/merlin/code/edgeplane/site && npm run check 2>&1
```

Expected: exits 0.

- [ ] **Step 3: Spot-check nav coverage**

Confirm `dist/edgeplane/` contains the following paths:

```
index.html
getting-started/installation/index.html
getting-started/quick-start/index.html
getting-started/agent-setup/index.html
concepts/overview/index.html
concepts/missions-klusters-tasks/index.html
concepts/entity-reference/index.html
concepts/philosophy/index.html
architecture/overview/index.html
architecture/persistence/index.html
architecture/ephemeral-agents/index.html
guides/deployment/index.html
guides/oidc/index.html
guides/agent-setup/index.html
guides/upgrading/index.html
reference/cli/index.html
reference/command-map/index.html
reference/edgeplaned-daemon/index.html
reference/real-time/index.html
reference/ai-console/index.html
adr/0001-project-catalog/index.html
adr/0002-source-of-truth/index.html
adr/0003-cli-hierarchy/index.html
```

Run:
```bash
for p in \
  "index.html" \
  "getting-started/installation/index.html" \
  "concepts/overview/index.html" \
  "architecture/overview/index.html" \
  "guides/deployment/index.html" \
  "reference/cli/index.html" \
  "adr/0001-project-catalog/index.html"; do
  [ -f "dist/edgeplane/$p" ] && echo "OK: $p" || echo "MISSING: $p"
done
```

Expected: all lines print `OK`.

- [ ] **Step 4: Verify no Aria/personal refs leaked**

```bash
grep -r "aria\|Merlin\|excalibur\|epyc\|cloud0\|infisical\|tailscale\b\|authentik\b\|rustfs\b\|zellij\b" \
  /home/merlin/code/edgeplane/site/src/content/docs/ \
  --include="*.md" --include="*.mdx" -i -l
```

Expected: no output. Any files listed need the relevant scrub applied.

- [ ] **Step 5: Final commit if any fixes were needed**

```bash
cd /home/merlin/code/edgeplane
git add site/
git commit -m "fix(docs): scrub remaining personal/fleet references"
```
