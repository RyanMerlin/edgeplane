---
title: Quick Start
description: Get an agent running under EdgePlane in under 5 minutes.
---

This guide walks you from zero to a running agent session. Assumes you've completed [Installation](/getting-started/installation/).

## 1. Start the Control Plane

If you're running locally:

```bash
edgeplane-tower --serve --bind 127.0.0.1:8008
```

If you have a deployed instance, point the CLI at it:

```bash
export EP_BASE_URL="https://edgeplane.example.com"
```

## 2. Authenticate

```bash
edgeplane auth login       # opens a browser OIDC flow → writes ~/.edgeplane/session.json
edgeplane auth whoami      # confirm identity and token expiry
```

After `auth login` succeeds, `edgeplane` picks up the session automatically from `~/.edgeplane/session.json` — no further credential management needed.

## 3. Verify the Connection

```bash
edgeplane health --json
edgeplane status           # shows auth, runtime, and workspace lease status
```

Both should report `ok: true`. If `health` fails, confirm `EP_BASE_URL` is set and the tower is reachable.

## 4. Launch an Agent

```bash
edgeplane run claude         # Claude Code with EdgePlane MCP wired in
edgeplane run codex          # OpenAI Codex CLI
edgeplane run gemini         # Google Gemini CLI
```

`edgeplane run` validates your environment, fetches the onboarding manifest, and injects EdgePlane as an MCP server before handing off to the agent binary. No manual MCP config required.

:::note[v0.13.0]
`edgeplane launch` was removed in v0.13.0. `edgeplane run` is the single entry point for all runtimes.
:::

## 5. Explore the Fleet

```bash
edgeplane tui
```

The TUI gives you a full-screen fleet view. Key bindings: `a` — agents, `m` — domains, `f` — live event feed, `p` — pending approvals queue.

## What's Next

- [Agent Setup](/getting-started/agent-setup/) — per-agent config, ACP runtime, profile loading
- [Concepts: Domains, Missions & Tasks](/concepts/domains-missions-tasks/) — the organizational model
- [Guides: Deployment](/guides/deployment/) — running EdgePlane in production
