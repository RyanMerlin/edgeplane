---
title: mc & mcd CLI
description: Complete reference for the mc CLI, mcd daemon, and mc-controlplane server binaries.
---

## mc — Operator CLI

`mc` is the primary operator and agent interface. All interactivity: fleet views, agent launch, capability dispatch, and the TUI.

### Global Flags

| Flag | Meaning |
|------|---------|
| `--base-url <URL>` | Control plane base URL (overrides `MC_BASE_URL`) |
| `--token <TOKEN>` | Bearer token (overrides `MC_TOKEN`) |
| `--json` | Output as JSON |
| `--output human\|json\|jsonl` | Output format |

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `MC_BASE_URL` | `http://localhost:8008` | Backend HTTP base URL |
| `MC_TOKEN` | unset | Bearer token for API auth |
| `MC_OUTPUT` | `human` | Default output format |

---

## Core Commands

### Status and Health

```bash
mc status [--verify-lease]    # combined auth / runtime / workspace status
mc doctor                     # shortcut to mc system doctor
mc health [--json]            # backend connectivity and MCP health probe
mc version                    # local CLI version + backend reachability
mc config                     # effective local runtime config (secrets redacted)
```

### TUI

```bash
mc tui [--mission <id>]
```

Full-screen terminal UI. Server and token come from env or `~/.mc/config.json`.

| Key | Tab | Description |
|-----|-----|-------------|
| `a` | Agents | Fleet nodes — status, current task, ops |
| `m` | Domains | Domains → Missions → Tasks (Enter to drill down) |
| `f` | Feed | Live SSE event stream (`p` to pause) |
| `p` | Approvals | Pending approval queue (`y` approve / `n` deny / `s` skip) |
| `s` | Secrets | Infisical folder/secret browser (read-only) |
| `c` | Config | Connection status and server info |
| Ctrl+Q / Ctrl+C | | Quit |

---

## Auth

```bash
mc auth login [--ttl-hours N] [--print-token]    # exchange credentials for a session token
mc auth whoami                                    # verify identity and session expiry
mc auth logout [--local-only]                    # revoke server-side and clear local file
```

Session tokens (`mcs_*` prefix) are stored at `~/.missioncontrol/session.json` (chmod 600). They are never written to agent config files on disk — injected at exec time only.

---

## Agent Launch

```bash
mc run claude [-p <profile>] [--mission <id>] [--mode interactive|headless|solo] [-- args]
mc run codex  [-p <profile>] [--mission <id>] [--mode interactive|headless|solo] [-- args]
mc run gemini [-p <profile>] [-- args]
mc launch openclaw    # OpenClaw
mc launch custom      # Custom ACP agent
```

**Diagnostics:**

```bash
mc run claude doctor [-p <profile>] [--fix] [--json]
mc run codex doctor  [-p <profile>] [--fix] [--json]
mc run codex status  [-p <profile>] [--json]
```

**Flags for `mc launch` (non-Claude/Codex agents):**

| Flag | Effect |
|------|--------|
| `--preflight-only` | Validate env + auth without launching |
| `--no-daemon` | Skip daemon management |
| `--skip-config-gen` | Use existing config, skip manifest fetch |
| `--no-embed-token` | Omit token from written config |
| `--legacy-global-config` | Write config to global agent paths |
| `--daemon-timeout N` | Seconds to wait for daemon ready (default: 15) |
| `-- <args>` | Pass remaining args verbatim to the agent |

---

## Workspace Lease

```bash
mc use --mission-id <id> [--lease-seconds N] [--workspace-label <label>]
mc use --release
mc release [--reason <text>] [--ignore-missing]
```

---

## Agent Management

```bash
mc agent signal <id> --content "..."        # send a prompt to an agent
mc agent list [--source local|remote|all] [--json]
mc agent describe <id> [--json]
mc agent attach <id> [--web] [--remote]
mc agent cancel <id>
```

### Scheduled Jobs (`mc agent cron`)

```bash
mc agent cron list [--json]
mc agent cron describe <name> [--limit N] [--json]
mc agent cron reload                         # re-parse cron.toml
mc agent cron history [--name <n>] [-n N] [--json]
mc agent cron gc-now [--history-days N]
```

Jobs are defined in `~/.mc/mcd/cron.toml`. See [mcd Daemon](/missioncontrol/reference/mcd-daemon/) for the format.

### Supervision (`mc agent supervise`)

```bash
mc agent supervise list [--json]
mc agent supervise status <id> [--limit N] [--json]
mc agent supervise restart <id>
mc agent supervise pause [<id>] [--all]
mc agent supervise resume [<id>] [--all]
mc agent supervise history [--agent-id <id>] [-n N] [--json]
mc agent supervise events [--json]           # stream live supervisor events
mc agent supervise watch [--poll-secs N]     # ratatui TUI
```

---

## Profiles

```bash
mc profile create <name>
mc profile list
mc profile show <name>
mc profile activate <name>               # atomic symlink swap
mc profile use <name>                    # activate + download in one step
mc profile download <name> [--out <path>]
mc profile pull <name>
mc profile publish <name>
mc profile pin <name> <sha256>
mc profile status <name>
mc profile delete <name>
```

---

## Capabilities

```bash
mc capabilities [--tag <tag>]            # list capability packs
mc capabilities describe <pack>.<capability>
mc exec <pack>.<capability> --json [--dry-run]
mc receipts last [--json]
```

---

## Data and System

```bash
mc data tools list
mc data tools call --tool <name> --payload '<json>'
mc data sync status --domain-id <id> [--mission-id <id>]
mc data sync promote ...
mc data explorer tree
mc data explorer node ...

mc system doctor [--fix]
mc system backup --target postgres|s3|all
mc system profile-gc ...
mc system update ...
```

---

## Approvals and Administration

```bash
mc approvals list
mc approvals approve <id>
mc approvals reject <id>

mc admin policy active
mc admin policy versions
mc admin policy events
mc admin governance ...
```

---

## Utility

```bash
mc init                         # initialize local configuration
mc serve                        # start MCP stdio server (used by agents)
mc logs                         # local log tail
mc completion <shell>           # shell completion generator
```

---

## mcd Daemon

See [mcd Daemon](/missioncontrol/reference/mcd-daemon/) for the full reference.

Quick commands:

```bash
mcd run --backend-url http://localhost:8008 --token $MC_TOKEN
mcd version
mcd get-secret MY_API_KEY    # inside agent subprocess only
```

---

## mc-controlplane Server

```bash
mc-controlplane --serve --bind 0.0.0.0:8008
```

Native routes: `/health`, `/raft/status`, `/domains`, `/missions`, `/tasks`, `/agents`.

```bash
curl http://localhost:8008/health
curl http://localhost:8008/raft/status
```

---

## Machine-Readable Output

All commands support `--json` for structured output. Always use `--json` when parsing programmatically — human-readable output format is not stable across releases.

```bash
mc missions list --json | jq '.[] | .id'
mc health --json
mc agent list --json
```
