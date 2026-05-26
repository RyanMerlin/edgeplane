---
title: edgeplaned Daemon
description: Reference for the edgeplaned headless executor daemon — agent lifecycle, secrets brokering, task worker, and cron dispatch.
---

`edgeplaned` is the headless work-executor daemon. Think of `edgeplane` as kubectl and `edgeplaned` as kubelet — `edgeplane` is the operator interface, `edgeplaned` is the node-side executor. Agents communicate with it via Unix socket; operators interact through `edgeplane agent` commands, not `edgeplaned` directly.

## Responsibilities

`edgeplaned` absorbed these daemon responsibilities across v0.8–v0.10:

| Version | Responsibility |
|---------|---------------|
| v0.8–v0.9 | Fleet agent lifecycle (launch, restart, Zellij integration via `ZellijHosted` runtime) |
| v0.9 | Cron dispatch — `~/.edgeplane/edgeplaned/cron.toml`, 1-minute tick loop |
| v0.10 | Watchdog — polls systemd units, restarts dead agents with throttling |
| v0.15+ | Task worker — ephemeral subagent spawning for mesh execution |

## Starting edgeplaned

For normal operation, use `edgeplane daemon up`. For advanced scripting or debugging, invoke `edgeplaned` directly:

```bash
edgeplane daemon up

# Advanced / debug
edgeplaned run --backend-url http://localhost:8008
```

`edgeplaned` reads its node identity from `/etc/edgeplane/node.json` at startup. Register the node first:

```bash
edgeplane agent node register --node-name <name>
# → writes JWT to /etc/edgeplane/node.json
```

For persistent operation, run as a systemd user service:

```ini
[Unit]
Description=EdgePlane Daemon
After=network-online.target

[Service]
ExecStart=/usr/local/bin/edgeplaned run --backend-url http://localhost:8008
Restart=on-failure
RestartSec=5
EnvironmentFile=%h/.edgeplane/env

[Install]
WantedBy=default.target
```

## Agent Runtimes

edgeplaned supports these agent runtime kinds (see `crates/edgeplaned/crates/edgeplaned-runtimes/src/`):

| Runtime | Description |
|---------|-------------|
| `claude_code` | One-shot `claude -p` |
| `claude_agent_acp` | Persistent JSON-RPC; ACP protocol |
| `codex` | OpenAI Codex CLI |
| `gemini` | Google Gemini CLI |
| `goose` | Goose (LLM-agnostic agent) |
| `zellij_hosted` | Long-running agents hosted in a Zellij pane; signals via `edgeplane agent signal` |

## Unix Sockets

All sockets live in `~/.edgeplane/edgeplaned/`:

| Socket | Purpose |
|--------|---------|
| `mgmt.sock` | JSON-RPC 2.0 management gateway |
| `secrets.sock` | Secrets broker (agent subprocesses only) |
| `edgeplaned.sock` | PTY attach gateway |

## Secrets Broker

`edgeplaned` injects two environment variables into agent subprocesses:

- `EP_SECRETS_SOCKET` — path to `secrets.sock`
- `EP_SECRETS_SESSION` — session ID for this agent

Agents retrieve secrets without ever receiving raw credentials:

```bash
VALUE=$(edgeplaned get-secret MY_API_KEY)
```

Or speak the protocol directly:

```bash
echo '{"op":"get","session":"'$EP_SECRETS_SESSION'","name":"MY_API_KEY"}' \
  | nc -U "$EP_SECRETS_SOCKET"
```

Raw secret values are never written to disk or embedded in config files.

## Cron Scheduling

Jobs are defined in `~/.edgeplane/edgeplaned/cron.toml`. edgeplaned reads it at startup and on `edgeplane agent cron reload`.

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
| `signal` (default) | `edgeplane agent signal` to the profile agent — has full profile context |
| `bash` | Literal shell command passed to `bash -c` — no LLM, no session |

**Manage cron:**

```bash
edgeplane agent cron list              # list all jobs and next fire time
edgeplane agent cron describe <name>   # full config + last fire
edgeplane agent cron history <name>    # recent fires
edgeplane agent cron reload            # re-parse cron.toml (no daemon restart needed)
edgeplane agent cron gc-now            # force garbage-collect stale dedup entries
```

Cron schedules are local time. Dispatcher tick is on the order of minutes — sub-minute expressions and sub-minute heartbeat intervals won't fire faster than the tick.

## Watchdog (Agent Supervision)

edgeplaned polls each supervised agent's systemd unit every 60 seconds. Dead units are restarted with throttling (90s post-restart grace, 30-minute retry throttle) plus an optional nightly restart.

```bash
edgeplane agent supervise list
edgeplane agent supervise status <id>
edgeplane agent supervise restart <id>
edgeplane agent supervise pause <id>    # disable auto-restart
edgeplane agent supervise resume <id>   # re-enable auto-restart
```

## Task Worker

edgeplaned's task worker runs two loops that enable distributed mesh execution. See [Architecture: Ephemeral Task Agents](/architecture/ephemeral-agents/) for the full model.

**Running an agent work loop:**

```bash
edgeplane daemon agent enroll --domain <id> --runtime goose
edgeplane run goose --domain <id>
```

**Environment variables for the work loop:**

| Variable | Default | Purpose |
|----------|---------|---------|
| `EP_BASE_URL` | `http://localhost:8008` | Backend URL |
| `EP_LITELLM_HOST` | `http://localhost:4000` | LiteLLM proxy URL (for Goose runtime) |
| `EP_LITELLM_API_KEY` | _(none)_ | LiteLLM master key |
| `EP_GOOSE_BIN` | PATH lookup | Override path to Goose binary |
| `EP_GOOSE_MODEL` | `local-agent` | Model name passed to Goose |

**Creating a MeshTask for dispatch:**

The `/work/missions/$MISSION_ID/tasks` REST path is an internal API path. The agent-facing interface is via MCP tools — prefer `submit_mesh_task` from within an agent session and `claim_mesh_task` for claiming. The curl example below is for direct API access (e.g. from automation scripts or debugging):

```bash
curl -X POST http://<edgeplane-host>/work/missions/$MISSION_ID/tasks \
  -H "Authorization: Bearer <session-token>" \
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

**Retry a failed task** (internal REST path — prefer the `retry_mesh_task` MCP tool from agent sessions):

```bash
curl -X POST http://<edgeplane-host>/work/tasks/<task-id>/retry \
  -H "Authorization: Bearer <session-token>"
```

## Known Limitations

- **Event bus threading:** `task_ready` WebSocket events may not wake the work loop reliably in single-worker deployments. The startup poll is the reliable dispatch path — restart the loop after creating tasks if events don't fire.
- **sudo in tasks:** agent subprocesses run without a TTY; `sudo` will fail unless the node has passwordless sudo configured.
- **GLIBC mismatch:** build `edgeplane`/`edgeplaned` natively on the target node if it runs an older glibc than your build machine.
- **Tasks vs MeshTasks:** the regular task API (`/domains/{id}/m/{id}/t`) is Kanban-style tracking. The work loop only operates on `MeshTask` objects at `/work/missions/{id}/tasks`. Always use the `/work/` API when creating tasks for agent dispatch.

## See Also

- [Architecture: Ephemeral Task Agents](/architecture/ephemeral-agents/) — distributed subagent model
- [Reference: CLI](/reference/cli/) — full `edgeplane` command surface including `edgeplane agent cron` and `edgeplane agent supervise`
