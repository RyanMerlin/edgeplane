---
title: Command Map
description: Authoritative edgeplane CLI command hierarchy at a glance.
---

This is the authoritative `edgeplane` CLI command hierarchy. For full descriptions of each command, see [edgeplane & edgeplaned CLI](/reference/cli/).

## Top Level

```
edgeplane status
edgeplane doctor
edgeplane health
edgeplane version
edgeplane config
edgeplane use
edgeplane release
edgeplane logs
edgeplane completion
edgeplane auth
edgeplane admin
edgeplane data
edgeplane system
edgeplane agent
edgeplane approvals
edgeplane workspace
edgeplane ops
edgeplane daemon
edgeplane run
edgeplane init
edgeplane serve
edgeplane profile
edgeplane tui
```

## Quick Verbs

| Command | Description |
|---------|-------------|
| `edgeplane status [--verify-lease]` | Combined auth/runtime/workspace status |
| `edgeplane doctor` | Shortcut to `edgeplane system doctor` |
| `edgeplane health` | Backend MCP health probe |
| `edgeplane version` | Local CLI version + backend reachability |
| `edgeplane config` | Effective local runtime config (redacted) |
| `edgeplane use --profile <name>` | Activate/apply profile |
| `edgeplane use --mission-id <id> [--lease-seconds N]` | Acquire workspace lease |
| `edgeplane use --release` | Release current lease |
| `edgeplane release [--reason <text>]` | Top-level lease release shortcut |
| `edgeplane logs` | Local log tail |
| `edgeplane completion <shell>` | Shell completion generator |

## `edgeplane run` — Agent Launch

All agents launch through `edgeplane run`. `edgeplane launch` was removed in v0.13.0.

| Command | Description |
|---------|-------------|
| `edgeplane run claude [-p <profile>] [--mission <id>] [--mode ...]` | Launch Claude Code (ACP persistent session) |
| `edgeplane run codex [-p <profile>] [--mission <id>] [--mode ...]` | Launch OpenAI Codex CLI |
| `edgeplane run gemini [-p <profile>] [-- args]` | Launch Google Gemini CLI |
| `edgeplane run goose [-p <profile>] [--domain <id>] [-- args]` | Launch Goose (native, profile-scoped) |
| `edgeplane run openclaw [-- args]` | Launch OpenClaw (driver agent) |
| `edgeplane run custom [-- args]` | Launch custom ACP agent |
| `edgeplane run claude doctor [-p <profile>] [--fix] [--json]` | Inspect/repair Claude runtime |
| `edgeplane run claude exec [-p <profile>] -- [args]` | Raw Claude passthrough |
| `edgeplane run claude hook --event <type>` | Internal Claude lifecycle hook |
| `edgeplane run codex doctor [-p <profile>] [--fix] [--json]` | Inspect/repair Codex runtime |
| `edgeplane run codex status [-p <profile>] [--json]` | Read-only Codex status |
| `edgeplane run codex exec [-p <profile>] -- [args]` | Raw Codex passthrough |

## `edgeplane auth`

| Command | Description |
|---------|-------------|
| `edgeplane auth login [--ttl-hours N] [--print-token]` | Exchange credentials for session token |
| `edgeplane auth whoami` | Verify identity and session expiry |
| `edgeplane auth logout [--local-only]` | Revoke session |

## `edgeplane agent`

| Command | Description |
|---------|-------------|
| `edgeplane agent signal <id> --content "..."` | Send prompt to agent |
| `edgeplane agent cancel <id>` | Interrupt agent |
| `edgeplane agent list [--source local\|remote\|all] [--json]` | List agents |
| `edgeplane agent describe <id> [--json]` | Show agent details |
| `edgeplane agent attach <id> [--web] [--remote]` | Attach to agent session |
| `edgeplane agent evolve ...` | Self-improvement loop |
| `edgeplane agent node register` | Register node |
| `edgeplane agent node run` | Start resident node-agent daemon |
| `edgeplane agent node doctor` | Validate node-agent connectivity |

### `edgeplane agent cron`

| Command | Description |
|---------|-------------|
| `edgeplane agent cron list [--json]` | List jobs + last-fire status |
| `edgeplane agent cron describe <name> [--limit N]` | One job + recent fires |
| `edgeplane agent cron reload` | Re-parse cron.toml |
| `edgeplane agent cron history [--name <n>] [-n N]` | Recent fires |
| `edgeplane agent cron gc-now [--history-days N]` | Force retention sweep |

### `edgeplane agent supervise`

| Command | Description |
|---------|-------------|
| `edgeplane agent supervise list [--json]` | Supervised agents + unit state |
| `edgeplane agent supervise status <id> [--limit N]` | One agent + restart history |
| `edgeplane agent supervise restart <id>` | Manual restart |
| `edgeplane agent supervise pause [<id>] [--all]` | Disable auto-restart |
| `edgeplane agent supervise resume [<id>] [--all]` | Re-enable auto-restart |
| `edgeplane agent supervise history [--agent-id <id>] [-n N]` | Restart events |
| `edgeplane agent supervise events [--json]` | Stream live supervisor events |
| `edgeplane agent supervise watch [--poll-secs N]` | Ratatui TUI |

## `edgeplane admin`

| Command | Description |
|---------|-------------|
| `edgeplane admin policy active` | Show active policy |
| `edgeplane admin policy versions` | List policy versions |
| `edgeplane admin policy events` | Show policy events |
| `edgeplane admin governance ...` | Governance operations |

## `edgeplane data`

| Command | Description |
|---------|-------------|
| `edgeplane data tools list` | List available MCP tools |
| `edgeplane data tools call --tool <name> --payload '<json>'` | Call a tool directly |
| `edgeplane data sync status ...` | Skill sync status |
| `edgeplane data sync promote ...` | Promote a skill version |
| `edgeplane data explorer tree` | Entity tree view |
| `edgeplane data explorer node ...` | Node details |

## `edgeplane system`

| Command | Description |
|---------|-------------|
| `edgeplane system doctor [--fix]` | Diagnose and repair runtime issues |
| `edgeplane system backup --target postgres\|s3\|all` | Trigger backup |
| `edgeplane system profile-gc ...` | Profile garbage collection |
| `edgeplane system update ...` | Update binaries |
| `edgeplane system compat ...` | Compatibility checks |
| `edgeplane system drift ...` | Drift detection |

## `edgeplane profile`

| Command | Description |
|---------|-------------|
| `edgeplane profile list` | List owned profiles |
| `edgeplane profile show --name <name>` | Show profile metadata |
| `edgeplane profile create --name <name>` | Create profile shell |
| `edgeplane profile activate --name <name>` | Set active profile (atomic symlink swap) |
| `edgeplane profile use --name <name>` | Activate + pull in one step |
| `edgeplane profile publish --name <name>` | Upload local bundle → server |
| `edgeplane profile pull --name <name>` | Pull bundle into local profile cache |
| `edgeplane profile download --name <name> [--out <file>]` | Save bundle to a local file |
| `edgeplane profile pin --name <name> --sha256 <hash>` | Pin to content hash |
| `edgeplane profile status --name <name>` | Local sync status vs backend |
| `edgeplane profile delete --name <name> --confirm-delete` | Remove from backend |

## Removed in 0.13.0

| Removed | Replacement |
|---------|-------------|
| `edgeplane launch <agent>` | `edgeplane run <agent>` |

## Removed in 0.11.0

| Removed | Replacement |
|---------|-------------|
| `edgeplane signal <id>` | `edgeplane agent signal <id> --remote` |
| `edgeplane agent remote <verb>` | `edgeplane agent <verb> --remote` |
