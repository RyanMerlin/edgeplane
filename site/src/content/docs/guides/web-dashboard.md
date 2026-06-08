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

Approve or deny via the buttons. Approved actions are HMAC-signed and returned to the requesting agent automatically.

### AI Console

A direct conversation interface backed by the `AiSession` model. Useful for one-off queries to a connected agent without opening a full attach session. The Console uses the REST `/api/ai/*` surface (not the PTY bridge).

### Feed

A live SSE event stream showing all domain events in real-time:
- `meshtask.*` — task lifecycle events
- `artifact.*` — artifact creation and updates
- `agent.*` — agent status changes
- `session.*` — ACP session lifecycle

Press the Pause button to freeze the feed for inspection.

## Theme

Toggle between light and dark mode via the avatar popup in the sidebar (Preferences → Theme).

## What's Next
- [Concepts: ACP](/concepts/acp/) — how the attach session works under the hood
- [Architecture: Components](/architecture/components/) — how the dashboard talks to the tower
