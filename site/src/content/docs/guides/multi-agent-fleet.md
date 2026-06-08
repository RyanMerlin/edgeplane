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

If you can't write to `/etc/`, you can specify an alternate path and configure `edgeplaned` to read it there.

### 2. Start the Daemon

```bash
edgeplane daemon up
# Or run the daemon binary directly:
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
edgeplane run claude [-p <profile>]
edgeplane run codex  [-p <profile>]
edgeplane run gemini [-p <profile>]
```

## Federated Attach

When an agent is running on a remote node, the web dashboard can still attach to it. The tower mediates the attach via the node's `edgeplaned` — you never need direct network access to the remote node.

```
Web browser → /api/attach-ws → edgeplane-tower → edgeplaned (remote node) → agent PTY
```

The agent's node must have `edgeplaned` running and registered. The attach-secret handshake is self-healing — if `edgeplaned` restarts, it re-registers and the next attach works automatically.

## Monitoring the Fleet

```bash
edgeplane agent list [--json]          # all registered agents + status
edgeplane daemon status                # daemon health check
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
prompt   = "run nightly code review"
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
