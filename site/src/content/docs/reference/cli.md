---
title: edgeplane & edgeplaned CLI
description: Complete reference for the edgeplane CLI, edgeplaned daemon, and edgeplane-tower server binaries.
---

## edgeplane — Operator CLI

`edgeplane` is the primary operator and agent interface. All interactivity: fleet views, agent launch, capability dispatch, and the TUI.

### Global Flags

| Flag | Meaning |
|------|---------|
| `--base-url <URL>` | Control plane base URL (overrides `EP_BASE_URL`) |
| `--token <TOKEN>` | Bearer token (overrides `EP_TOKEN`) |
| `--json` | Output as JSON |
| `--output human\|json\|jsonl` | Output format |

### Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `EP_BASE_URL` | `http://localhost:8008` | Backend HTTP base URL |
| `EP_TOKEN` | unset | Bearer token for API auth |
| `EP_OUTPUT` | `human` | Default output format |

---

## Core Commands

### Status and Health

```bash
edgeplane status [--verify-lease]    # combined auth / runtime / workspace status
edgeplane doctor                     # shortcut to edgeplane system doctor
edgeplane health [--json]            # backend connectivity and MCP health probe
edgeplane version                    # local CLI version + backend reachability
edgeplane config                     # effective local runtime config (secrets redacted)
```

### TUI

```bash
edgeplane tui [--mission <id>]
```

Full-screen terminal UI. Server and token come from env or `~/.ep/config.json`.

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
edgeplane auth login [--ttl-hours N] [--print-token]    # exchange credentials for a session token
edgeplane auth whoami                                    # verify identity and session expiry
edgeplane auth logout [--local-only]                    # revoke server-side and clear local file
```

Session tokens (`mcs_*` prefix) are stored at `~/.edgeplane/session.json` (chmod 600). They are never written to agent config files on disk — injected at exec time only.

---

## Agent Launch

```bash
edgeplane run claude [-p <profile>] [--mission <id>] [--mode interactive|headless|solo] [-- args]
edgeplane run codex  [-p <profile>] [--mission <id>] [--mode interactive|headless|solo] [-- args]
edgeplane run gemini [-p <profile>] [-- args]
edgeplane launch openclaw    # OpenClaw
edgeplane launch custom      # Custom ACP agent
```

**Diagnostics:**

```bash
edgeplane run claude doctor [-p <profile>] [--fix] [--json]
edgeplane run codex doctor  [-p <profile>] [--fix] [--json]
edgeplane run codex status  [-p <profile>] [--json]
```

**Flags for `edgeplane launch` (non-Claude/Codex agents):**

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
edgeplane use --mission-id <id> [--lease-seconds N] [--workspace-label <label>]
edgeplane use --release
edgeplane release [--reason <text>] [--ignore-missing]
```

---

## Agent Management

```bash
edgeplane agent signal <id> --content "..."        # send a prompt to an agent
edgeplane agent list [--source local|remote|all] [--json]
edgeplane agent describe <id> [--json]
edgeplane agent attach <id> [--web] [--remote]
edgeplane agent cancel <id>
```

### Scheduled Jobs (`edgeplane agent cron`)

```bash
edgeplane agent cron list [--json]
edgeplane agent cron describe <name> [--limit N] [--json]
edgeplane agent cron reload                         # re-parse cron.toml
edgeplane agent cron history [--name <n>] [-n N] [--json]
edgeplane agent cron gc-now [--history-days N]
```

Jobs are defined in `~/.ep/edgeplaned/cron.toml`. See [edgeplaned Daemon](/edgeplane/reference/edgeplaned-daemon/) for the format.

### Supervision (`edgeplane agent supervise`)

```bash
edgeplane agent supervise list [--json]
edgeplane agent supervise status <id> [--limit N] [--json]
edgeplane agent supervise restart <id>
edgeplane agent supervise pause [<id>] [--all]
edgeplane agent supervise resume [<id>] [--all]
edgeplane agent supervise history [--agent-id <id>] [-n N] [--json]
edgeplane agent supervise events [--json]           # stream live supervisor events
edgeplane agent supervise watch [--poll-secs N]     # ratatui TUI
```

---

## Profiles

```bash
edgeplane profile create <name>
edgeplane profile list
edgeplane profile show <name>
edgeplane profile activate <name>               # atomic symlink swap
edgeplane profile use <name>                    # activate + download in one step
edgeplane profile download <name> [--out <path>]
edgeplane profile pull <name>
edgeplane profile publish <name>
edgeplane profile pin <name> <sha256>
edgeplane profile status <name>
edgeplane profile delete <name>
```

---

## Capabilities

```bash
edgeplane capabilities [--tag <tag>]            # list capability packs
edgeplane capabilities describe <pack>.<capability>
edgeplane exec <pack>.<capability> --json [--dry-run]
edgeplane receipts last [--json]
```

---

## Data and System

```bash
edgeplane data tools list
edgeplane data tools call --tool <name> --payload '<json>'
edgeplane data sync status --domain-id <id> [--mission-id <id>]
edgeplane data sync promote ...
edgeplane data explorer tree
edgeplane data explorer node ...

edgeplane system doctor [--fix]
edgeplane system backup --target postgres|s3|all
edgeplane system profile-gc ...
edgeplane system update ...
```

---

## Approvals and Administration

```bash
edgeplane approvals list
edgeplane approvals approve <id>
edgeplane approvals reject <id>

edgeplane admin policy active
edgeplane admin policy versions
edgeplane admin policy events
edgeplane admin governance ...
```

---

## Utility

```bash
edgeplane init                         # initialize local configuration
edgeplane serve                        # start MCP stdio server (used by agents)
edgeplane logs                         # local log tail
edgeplane completion <shell>           # shell completion generator
```

---

## edgeplaned Daemon

See [edgeplaned Daemon](/edgeplane/reference/edgeplaned-daemon/) for the full reference.

Quick commands:

```bash
edgeplaned run --backend-url http://localhost:8008 --token $EP_TOKEN
edgeplaned version
edgeplaned get-secret MY_API_KEY    # inside agent subprocess only
```

---

## edgeplane-tower Server

```bash
edgeplane-tower --serve --bind 0.0.0.0:8008
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
edgeplane missions list --json | jq '.[] | .id'
edgeplane health --json
edgeplane agent list --json
```
