---
title: Quick Start
description: Get an agent running under MissionControl in under 5 minutes.
---

This guide walks you from zero to a running agent session. Assumes you've completed [Installation](/missioncontrol/getting-started/installation/).

## 1. Start the Control Plane

If you're running locally:

```bash
mc-controlplane --serve --bind 127.0.0.1:8008
```

If you have a deployed instance, set `MC_BASE_URL` to point to it.

## 2. Configure Your Connection

```bash
export MC_BASE_URL="http://localhost:8008"   # or your deployed URL
export MC_TOKEN="your-token"                 # static token or OIDC JWT
```

Verify connectivity:

```bash
mc health --json
```

## 3. Create a Session Token (recommended)

Exchange your credentials for a revocable session token that's never written to agent config files:

```bash
mc auth login          # creates ~/.missioncontrol/session.json
mc auth whoami         # confirm identity
```

After this, `MC_TOKEN` is no longer needed in the environment — `mc` picks up the session automatically.

## 4. Launch an Agent

```bash
mc run claude          # Claude Code
mc run codex           # OpenAI Codex CLI
mc run gemini          # Google Gemini CLI
```

`mc run` validates your environment, fetches the onboarding manifest, and launches the agent with MissionControl wired in as an MCP server.

## 5. Create Your First Domain

Inside the running agent, or via the CLI:

```bash
mc domains list --json
```

Or open the TUI for a full fleet view:

```bash
mc tui
```

The TUI gives you real-time agent status, domain/mission/task drill-down, a live event feed, and a pending approvals queue.

## What's Next

- [Agent Setup](/missioncontrol/getting-started/agent-setup/) — per-agent config, MCP server setup, auth modes
- [Concepts: Domains, Missions & Tasks](/missioncontrol/concepts/domains-missions-tasks/) — the organizational model
- [Guides: Deployment](/missioncontrol/guides/deployment/) — running MC in production
