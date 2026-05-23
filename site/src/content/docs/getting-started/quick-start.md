---
title: Quick Start
description: Get an agent running under Edgeplane in under 5 minutes.
---

This guide walks you from zero to a running agent session. Assumes you've completed [Installation](/edgeplane/getting-started/installation/).

## 1. Start the Control Plane

If you're running locally:

```bash
edgeplane-tower --serve --bind 127.0.0.1:8008
```

If you have a deployed instance, set `EP_BASE_URL` to point to it.

## 2. Configure Your Connection

```bash
export EP_BASE_URL="http://localhost:8008"   # or your deployed URL
export EP_TOKEN="your-token"                 # static token or OIDC JWT
```

Verify connectivity:

```bash
edgeplane health --json
```

## 3. Create a Session Token (recommended)

Exchange your credentials for a revocable session token that's never written to agent config files:

```bash
edgeplane auth login          # creates ~/.edgeplane/session.json
edgeplane auth whoami         # confirm identity
```

After this, `EP_TOKEN` is no longer needed in the environment — `edgeplane` picks up the session automatically.

## 4. Launch an Agent

```bash
edgeplane run claude          # Claude Code
edgeplane run codex           # OpenAI Codex CLI
edgeplane run gemini          # Google Gemini CLI
```

`edgeplane run` validates your environment, fetches the onboarding manifest, and launches the agent with Edgeplane wired in as an MCP server.

## 5. Create Your First Domain

Inside the running agent, or via the CLI:

```bash
edgeplane domains list --json
```

Or open the TUI for a full fleet view:

```bash
edgeplane tui
```

The TUI gives you real-time agent status, domain/mission/task drill-down, a live event feed, and a pending approvals queue.

## What's Next

- [Agent Setup](/edgeplane/getting-started/agent-setup/) — per-agent config, MCP server setup, auth modes
- [Concepts: Domains, Missions & Tasks](/edgeplane/concepts/domains-missions-tasks/) — the organizational model
- [Guides: Deployment](/edgeplane/guides/deployment/) — running MC in production
