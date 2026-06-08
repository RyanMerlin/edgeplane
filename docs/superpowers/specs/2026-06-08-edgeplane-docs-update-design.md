# EdgePlane Docs Overhaul — Design Spec

**Date:** 2026-06-08  
**Status:** Approved  
**Approach:** Parallel subagent rewrite (Approach A) with IA pre-pass

---

## Goal

Bring edgeplane.ai documentation to full alignment with the current platform (v0.13.x). Fill the architecture section (currently empty), add first-class coverage for ACP, zRPC, MeshTasks, Web UI, and the unified `edgeplane run` launcher. Position EdgePlane as a serious, top-tier OSS infrastructure project for AI coordination.

---

## Execution Strategy

1. All agents work from this IA spec and the shared scrubbing rules below.
2. Fan out one agent per section (Getting Started, Concepts, Architecture, Guides, Reference + ADRs) — all run concurrently.
3. Each agent reads: current page(s) + CHANGELOG + relevant source/docs + this spec.
4. A final consistency pass checks cross-links, nav config (`astro.config.mjs`), and frontmatter.

---

## Scrubbing Rules (apply to every page)

| Prohibited | Why |
|-----------|-----|
| Homelab hostnames (`epyc`, `cloud0`, `kai`, `aria-memory-pg`, `*.ts.net`) | Personal infra |
| Tailscale addresses / VPN topology | Personal infra |
| Personal Infisical paths (`/providers/`, `/aria-sa/`, `/infra/`) | Internal secret schema |
| Fleet profile names (`operator`, `research`, `merlinlabs`, `work`, `engineer`) | Personal fleet |
| Vault/S3 bucket names or paths | Internal storage |
| Personal GitHub handle in code examples | Use `<your-org>` or generic |
| `/workspace/cache/zellij` path | Homelab-specific cache location |

Generic replacements: use `your-tower-host`, `https://edgeplane.example.com`, `<your-domain>`, `<org>`.

---

## Information Architecture

### Getting Started

| Page | Status | Key changes |
|------|--------|-------------|
| `installation.md` | Update | v0.13.x binaries, remove `launch` references, add `edgeplane run` intro |
| `quick-start.md` | Update | New auth flow (`edgeplane auth login`), `edgeplane run claude`, current health/status commands |
| `agent-setup.md` | Update | ACP runtime explanation, session token mechanics, profile loading on startup |

### Concepts

| Page | Status | Key changes |
|------|--------|-------------|
| `overview.md` | Minor update | Add ACP, MeshTasks, Web UI to capability table |
| `philosophy.md` | Keep | No changes needed |
| `domains-missions-tasks.md` | Minor update | Tighten MeshTask mention; link to new MeshTask concept page |
| `entity-reference.md` | Update | Add MeshTask, AgentSession, AiSession entities |
| `acp.md` | **New** | ACP concept: what it is, transport model, session lifecycle, runtimes (`claude_agent_acp`, `zellij_hosted`, `driver`) |
| `mesh-tasks.md` | **New** | Distributed agent execution: what a MeshTask is, claim/heartbeat/complete lifecycle, use cases |
| `profiles.md` | **New** | Personal operator profiles: what they carry (env, tools, instructions), how they sync, profile switching |

### Architecture

All new. The `architecture/` directory in the site is currently empty.

| Page | Status | Content |
|------|--------|---------|
| `overview.md` | **New** | Component map: `edgeplane` CLI, `edgeplane-tower` server, `edgeplaned` daemon, React web UI, `edgeplane-zrpc` Zellij plugin. How they relate. |
| `components.md` | **New** | Per-binary deep-dive: responsibilities, config, trust boundary, deployment target |
| `data-flow.md` | **New** | Full lifecycle: task creation → agent claim → execution → artifact publication → ledger commit |
| `security.md` | **New** | OIDC auth, HMAC-signed approval tokens, audit trail, session scoping, node JWT |

### Guides

| Page | Status | Key changes |
|------|--------|-------------|
| `deployment.md` | Update | `/api` prefix, current docker-compose pattern, tower startup flags |
| `oidc.md` | Update | Correct callback URL (`/api/auth/oidc/callback`), Authentik integration pattern |
| `upgrading.md` | Keep | Minor version note for v0.13.x breaking changes |
| `advanced-agent-configuration.md` | Keep | Minor updates |
| `web-dashboard.md` | **New** | The React UI: tabs (fleet, domains, governance, console, feed), ACP attach, how to read the event feed, avatar/auth |
| `zellij-integration.md` | **New** | `edgeplane-zrpc` plugin: what it unlocks (focus-free inject/cancel, scrollback, pane lifecycle), setup steps (env vars, feature flag), permissions model |
| `multi-agent-fleet.md` | **New** | Running `edgeplaned`: node registration, federated attach, fleet overview in the web UI |
| `fleet-profiles-advanced.md` | **New** | (Advanced) Self-hosted personal profiles: what they contain, how to push/pull from any machine, profile switching |

### Reference

| Page | Status | Key changes |
|------|--------|-------------|
| `cli.md` | Update | Remove `launch`, document all `edgeplane run` runtimes (`claude`, `codex`, `gemini`, `goose`, `openclaw`, `custom`), current auth commands, `edgeplane tui` key map |
| `edgeplaned-daemon.md` | Update | Phase 7 federated attach, node JWT (`/etc/edgeplane/node.json`), self-heal mechanics, `attach_secret` |
| `real-time.md` | Minor update | Current SSE event types including `progress` |
| `ai-console.md` | Keep | Minor link updates |
| `command-map.md` | Update | Remove retired commands, add new ones |
| `zrpc-plugin.md` | **New** | `edgeplane-zrpc` reference: env vars (`EDGEPLANE_ZRPC_PLUGIN_PATH`, `EDGEPLANE_ZRPC_SESSIONS`), pipe interface, permissions, feature flag behavior |

### ADRs

| Page | Status | Content |
|------|--------|---------|
| Existing 0001–0003 | Keep | No changes |
| `0004-attach-transport.md` | Keep | No changes |
| `0005-unified-agent-launcher.md` | **New** | Decision: retire `edgeplane launch`, unify on `edgeplane run <runtime>`. Context, options considered, consequences. |

---

## Narrative Arc

The docs should tell a coherent story in this order:

1. **Why** — AI agents can't coordinate without infrastructure (philosophy)
2. **What** — EdgePlane is that infrastructure: domains, missions, tasks, artifacts (concepts)
3. **How it's built** — the components and how they connect (architecture)
4. **How to run it** — installation, quick start, deployment (getting started + guides)
5. **The details** — full CLI/daemon reference for operators (reference)

Every new page should link forward and backward in this arc.

---

## Quality Bar

- No homelab references per scrubbing rules above
- All `edgeplane launch` references replaced with `edgeplane run`
- All `/hooks/` references updated to `/api/hooks/` (post-0.12.0)
- Code blocks use generic hostnames
- Every new page has a `## Next Steps` or `## What's Next` section linking to related docs
- Frontmatter: `title`, `description` on every page
- No broken internal links (verify after nav config update)

---

## Nav Config Changes (`site/astro.config.mjs`)

Add `architecture` to sidebar (currently missing despite having a directory):
```js
{ label: 'Architecture', autogenerate: { directory: 'architecture' } },
```

New pages auto-discovered via `autogenerate` — no manual sidebar entries needed as long as frontmatter is present.

Create `site/src/content/docs/architecture/` directory.

---

## Agent Fan-out Map

| Agent | Pages owned | Source material |
|-------|-------------|----------------|
| Agent 1: Getting Started | installation, quick-start, agent-setup | CHANGELOG, CLI reference, current pages |
| Agent 2: Concepts | overview (update), entity-reference (update), acp (new), mesh-tasks (new), profiles (new) | entities.md, CHANGELOG, architecture.md |
| Agent 3: Architecture | all 4 new pages | architecture.md, CHANGELOG, crate list, security patterns |
| Agent 4: Guides | web-dashboard (new), zellij-integration (new), multi-agent-fleet (new), fleet-profiles-advanced (new), deployment (update), oidc (update) | CHANGELOG, guides source, zrpc docs |
| Agent 5: Reference + ADRs | cli (update), edgeplaned-daemon (update), command-map (update), zrpc-plugin (new), adr-0005 (new) | CHANGELOG, CLI help text, ADR sources |
| Agent 6: Consistency | nav config, cross-links, frontmatter audit | All pages post-write |
