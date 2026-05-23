---
title: mcd Daemon
description: Reference for the mcd headless executor daemon — agent lifecycle, secrets brokering, task worker, and cron dispatch.
---

`mcd` is the headless work-executor daemon. Think of `mc` as kubectl and `mcd` as kubelet — `mc` is the operator interface, `mcd` is the node-side executor. Agents communicate with it via Unix socket; operators interact through `mc agent` commands, not `mcd` directly.

## Responsibilities

`mcd` absorbed these daemon responsibilities across v0.8–v0.10:

| Version | Responsibility |
|---------|---------------|
| v0.8–v0.9 | Fleet agent lifecycle (launch, restart, Zellij integration via `ZellijHosted` runtime) |
| v0.9 | Cron dispatch — `~/.mc/mcd/cron.toml`, 1-minute tick loop |
| v0.10 | Watchdog — polls systemd units, restarts dead agents with throttling |
| v0.15+ | Task worker — ephemeral subagent spawning for mesh execution |

## Starting mcd

```bash
mcd run --backend-url http://localhost:8008 --token $MC_TOKEN
```

For persistent operation, run as a systemd user service:

```ini
[Unit]
Description=MissionControl Daemon
After=network-online.target

[Service]
ExecStart=/usr/local/bin/mcd run --backend-url http://localhost:8008
Restart=on-failure
RestartSec=5
EnvironmentFile=%h/.missioncontrol/env

[Install]
WantedBy=default.target
```

## Agent Runtimes

mcd supports these agent runtime kinds (see `crates/mcd/crates/mcd-runtimes/src/`):

| Runtime | Description |
|---------|-------------|
| `claude_code` | One-shot `claude -p` |
| `claude_agent_acp` | Persistent JSON-RPC; ACP protocol |
| `codex` | OpenAI Codex CLI |
| `gemini` | Google Gemini CLI |
| `goose` | Goose (LLM-agnostic agent) |
| `zellij_hosted` | Long-running agents hosted in a Zellij pane; signals via `mc agent signal` |

## Unix Sockets

All sockets live in `~/.mc/`:

| Socket | Purpose |
|--------|---------|
| `mcd-mgmt.sock` | JSON-RPC 2.0 management gateway |
| `mcd-secrets.sock` | Secrets broker (agent subprocesses only) |
| `mcd.sock` | PTY attach gateway |

## Secrets Broker

`mcd` injects two environment variables into agent subprocesses:

- `MC_SECRETS_SOCKET` — path to `mcd-secrets.sock`
- `MC_SECRETS_SESSION` — session ID for this agent

Agents retrieve secrets without ever receiving raw credentials:

```bash
VALUE=$(mcd get-secret MY_API_KEY)
```

Or speak the protocol directly:

```bash
echo '{"op":"get","session":"'$MC_SECRETS_SESSION'","name":"MY_API_KEY"}' \
  | nc -U "$MC_SECRETS_SOCKET"
```

Raw secret values are never written to disk or embedded in config files.

## Cron Scheduling

Jobs are defined in `~/.mc/mcd/cron.toml`. mcd reads it at startup and on `mc agent cron reload`.

### Cron job (exact timing)

```toml
[[job]]
name     = "my-job"
schedule = "30 6 * * 1-5"   # 5-field cron, local time
session  = "my-profile"      # target profile / Zellij session name
prompt   = "run /my-skill"
# dispatch = "signal"        # "signal" (default) or "bash"
```

### Heartbeat job (approximate cadence)

```toml
[[job]]
name     = "my-heartbeat"
kind     = "heartbeat"
interval = "30m"             # "Ns", "Nm", "Nh", "Nd", or compound "2h30m"
session  = "my-profile"
prompt   = "run /my-check"
```

Dispatch modes:

| `dispatch` | Behavior |
|------------|---------|
| `signal` (default) | `mc agent signal` to the profile agent — has full profile context |
| `bash` | Literal shell command passed to `bash -c` — no LLM, no session |

**Manage cron:**

```bash
mc agent cron list              # list all jobs and next fire time
mc agent cron describe <name>   # full config + last fire
mc agent cron history <name>    # recent fires
mc agent cron reload            # re-parse cron.toml (no daemon restart needed)
mc agent cron gc-now            # force garbage-collect stale dedup entries
```

Cron schedules are local time. Dispatcher tick is on the order of minutes — sub-minute expressions and sub-minute heartbeat intervals won't fire faster than the tick.

## Watchdog (Agent Supervision)

mcd polls each supervised agent's systemd unit every 60 seconds. Dead units are restarted with throttling (90s post-restart grace, 30-minute retry throttle) plus an optional nightly restart.

```bash
mc agent supervise list
mc agent supervise status <id>
mc agent supervise restart <id>
mc agent supervise pause <id>    # disable auto-restart
mc agent supervise resume <id>   # re-enable auto-restart
```

## Task Worker

mcd's task worker runs two loops that enable distributed mesh execution. See [Architecture: Ephemeral Task Agents](/missioncontrol/architecture/ephemeral-agents/) for the full model.

**Running an agent work loop:**

```bash
mc daemon agent enroll --domain <id> --runtime goose
mc run goose --domain <id>
```

**Environment variables for the work loop:**

| Variable | Default | Purpose |
|----------|---------|---------|
| `MC_BASE_URL` | `http://localhost:8008` | Backend URL |
| `MC_LITELLM_HOST` | `http://localhost:4000` | LiteLLM proxy URL (for Goose runtime) |
| `MC_LITELLM_API_KEY` | _(none)_ | LiteLLM master key |
| `MC_GOOSE_BIN` | PATH lookup | Override path to Goose binary |
| `MC_GOOSE_MODEL` | `local-agent` | Model name passed to Goose |

**Creating a MeshTask for dispatch:**

```bash
curl -X POST http://<mc-host>/work/missions/$MISSION_ID/tasks \
  -H "Authorization: Bearer $MC_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "task title",
    "description": "what to do",
    "claim_policy": "first_claim",
    "priority": 5
  }'
```

Claim policy options:

| Policy | Behavior |
|--------|---------|
| `first_claim` | Any available agent with matching capabilities |
| `assigned` | Specific agent only |
| `broadcast` | All agents |

**Retry a failed task:**

```bash
curl -X POST http://<mc-host>/work/tasks/<task-id>/retry \
  -H "Authorization: Bearer $MC_TOKEN"
```

## Known Limitations

- **Event bus threading:** `task_ready` WebSocket events may not wake the work loop reliably in single-worker deployments. The startup poll is the reliable dispatch path — restart the loop after creating tasks if events don't fire.
- **sudo in tasks:** agent subprocesses run without a TTY; `sudo` will fail unless the node has passwordless sudo configured.
- **GLIBC mismatch:** build `mc`/`mcd` natively on the target node if it runs an older glibc than your build machine.
- **Tasks vs MeshTasks:** the regular task API (`/domains/{id}/m/{id}/t`) is Kanban-style tracking. The work loop only operates on `MeshTask` objects at `/work/missions/{id}/tasks`. Always use the `/work/` API when creating tasks for agent dispatch.

## See Also

- [Architecture: Ephemeral Task Agents](/missioncontrol/architecture/ephemeral-agents/) — distributed subagent model
- [Reference: CLI](/missioncontrol/reference/cli/) — full `mc` command surface including `mc agent cron` and `mc agent supervise`
