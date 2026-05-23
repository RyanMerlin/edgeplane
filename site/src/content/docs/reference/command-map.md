---
title: Command Map
description: Authoritative mc CLI command hierarchy at a glance.
---

This is the authoritative `mc` CLI command hierarchy. For full descriptions of each command, see [mc & mcd CLI](/missioncontrol/reference/cli/).

## Top Level

```
mc status
mc doctor
mc health
mc version
mc config
mc use
mc release
mc logs
mc completion
mc auth
mc admin
mc data
mc system
mc agent
mc approvals
mc workspace
mc ops
mc daemon
mc launch
mc run
mc init
mc serve
mc profile
mc tui
```

## Quick Verbs

| Command | Description |
|---------|-------------|
| `mc status [--verify-lease]` | Combined auth/runtime/workspace status |
| `mc doctor` | Shortcut to `mc system doctor` |
| `mc health` | Backend MCP health probe |
| `mc version` | Local CLI version + backend reachability |
| `mc config` | Effective local runtime config (redacted) |
| `mc use --profile <name>` | Activate/apply profile |
| `mc use --mission-id <id> [--lease-seconds N]` | Acquire workspace lease |
| `mc use --release` | Release current lease |
| `mc release [--reason <text>]` | Top-level lease release shortcut |
| `mc logs` | Local log tail |
| `mc completion <shell>` | Shell completion generator |

## `mc run` — Agent Launch

| Command | Description |
|---------|-------------|
| `mc run claude [-p <profile>] [--mission <id>] [--mode ...]` | Launch Claude Code |
| `mc run codex [-p <profile>] [--mission <id>] [--mode ...]` | Launch Codex CLI |
| `mc run gemini [-p <profile>] [-- args]` | Launch Gemini CLI |
| `mc run claude doctor [-p <profile>] [--fix] [--json]` | Inspect/repair Claude runtime |
| `mc run claude exec [-p <profile>] -- [args]` | Raw Claude passthrough |
| `mc run codex doctor [-p <profile>] [--fix] [--json]` | Inspect/repair Codex runtime |
| `mc run codex status [-p <profile>] [--json]` | Read-only Codex status |
| `mc run codex exec [-p <profile>] -- [args]` | Raw Codex passthrough |
| `mc run claude hook --event <type>` | Internal Claude lifecycle hook |

## `mc auth`

| Command | Description |
|---------|-------------|
| `mc auth login [--ttl-hours N] [--print-token]` | Exchange credentials for session token |
| `mc auth whoami` | Verify identity and session expiry |
| `mc auth logout [--local-only]` | Revoke session |

## `mc agent`

| Command | Description |
|---------|-------------|
| `mc agent signal <id> --content "..."` | Send prompt to agent |
| `mc agent cancel <id>` | Interrupt agent |
| `mc agent list [--source local\|remote\|all] [--json]` | List agents |
| `mc agent describe <id> [--json]` | Show agent details |
| `mc agent attach <id> [--web] [--remote]` | Attach to agent session |
| `mc agent evolve ...` | Self-improvement loop |
| `mc agent node register` | Register node |
| `mc agent node run` | Start resident node-agent daemon |
| `mc agent node doctor` | Validate node-agent connectivity |

### `mc agent cron`

| Command | Description |
|---------|-------------|
| `mc agent cron list [--json]` | List jobs + last-fire status |
| `mc agent cron describe <name> [--limit N]` | One job + recent fires |
| `mc agent cron reload` | Re-parse cron.toml |
| `mc agent cron history [--name <n>] [-n N]` | Recent fires |
| `mc agent cron gc-now [--history-days N]` | Force retention sweep |

### `mc agent supervise`

| Command | Description |
|---------|-------------|
| `mc agent supervise list [--json]` | Supervised agents + unit state |
| `mc agent supervise status <id> [--limit N]` | One agent + restart history |
| `mc agent supervise restart <id>` | Manual restart |
| `mc agent supervise pause [<id>] [--all]` | Disable auto-restart |
| `mc agent supervise resume [<id>] [--all]` | Re-enable auto-restart |
| `mc agent supervise history [--agent-id <id>] [-n N]` | Restart events |
| `mc agent supervise events [--json]` | Stream live supervisor events |
| `mc agent supervise watch [--poll-secs N]` | Ratatui TUI |

## `mc admin`

| Command | Description |
|---------|-------------|
| `mc admin policy active` | Show active policy |
| `mc admin policy versions` | List policy versions |
| `mc admin policy events` | Show policy events |
| `mc admin governance ...` | Governance operations |

## `mc data`

| Command | Description |
|---------|-------------|
| `mc data tools list` | List available MCP tools |
| `mc data tools call --tool <name> --payload '<json>'` | Call a tool directly |
| `mc data sync status ...` | Skill sync status |
| `mc data sync promote ...` | Promote a skill version |
| `mc data explorer tree` | Entity tree view |
| `mc data explorer node ...` | Node details |

## `mc system`

| Command | Description |
|---------|-------------|
| `mc system doctor [--fix]` | Diagnose and repair runtime issues |
| `mc system backup --target postgres\|s3\|all` | Trigger backup |
| `mc system profile-gc ...` | Profile garbage collection |
| `mc system update ...` | Update binaries |
| `mc system compat ...` | Compatibility checks |
| `mc system drift ...` | Drift detection |

## `mc profile`

| Command | Description |
|---------|-------------|
| `mc profile create <name>` | Create profile shell |
| `mc profile list` | List owned profiles |
| `mc profile show <name>` | Show profile metadata |
| `mc profile activate <name>` | Set active profile (atomic symlink swap) |
| `mc profile use <name>` | Activate + download (compat alias) |
| `mc profile download <name> [--out <path>]` | Download bundle |
| `mc profile pull <name>` | Pull into local cache |
| `mc profile publish <name>` | Push local profile to backend |
| `mc profile pin <name> <sha256>` | Pin to content hash |
| `mc profile status <name>` | Local sync status vs backend |
| `mc profile delete <name>` | Remove from backend |

## Removed in 0.11.0

The following commands were removed. Use the replacements shown:

| Removed | Replacement |
|---------|-------------|
| `mc signal <id>` | `mc agent signal <id> --remote` |
| `mc agent remote <verb>` | `mc agent <verb> --remote` |
