# mcd — Agent Work Loop

mcd is the work-first agent coordination daemon. Think Temporal, not RKE2:
- **Mission** = namespace / long-lived workspace
- **Kluster** = objective owning a task DAG
- **MeshTask** = unit of work (claimed, executed, finished)
- **AgentRuntime** = worker. Five impls today: `claude_code` (one-shot `claude -p`), `claude_agent_acp` (persistent JSON-RPC; ACP), `codex`, `gemini`, `goose`, and `zellij_hosted` (long-running agents hosted in a Zellij pane — Aria fleet; signals via `mc agent signal`). See `crates/mcd/crates/mcd-runtimes/src/`.

The daemon (`mcd`) runs a headless attach gateway. The work loop (`mc run <runtime>`) connects to a mission, claims tasks, and supervises agent child processes.

### Absorbed responsibilities (daemon-absorption plan)

Across v0.8–v0.10, mcd absorbed the daemon-side responsibilities that
used to live in `aria-rs` (`aria fleet`, `aria cron`, `aria watchdog`).
aria-rs is now a pure toolchain — no long-running processes:

- **v0.8 Phase 2–3 — Fleet agents + CLI**: the `ZellijHosted` runtime
  drives long-running profile agents (operator, work, research, …)
  through `zellij action paste + send-keys`. `mc agent signal/cancel/
  attach/list/describe` is the surface.
- **v0.9 Phase 4 — Cron**: `~/.mc/mcd/cron.toml` is mcd's scheduling
  config; mcd ticks every minute and dispatches via the same signal
  path. `mc agent cron list/describe/reload/history/gc-now` is the
  surface.
- **v0.10 Phase 5 — Watchdog**: mcd polls each agent's systemd unit
  and restarts dead ones with throttling + nightly hygiene. Events
  publish to a broadcast channel for future TUI/web consumers. `mc
  agent supervise list/status/restart/pause/resume/history` is the
  surface.

---

## Install on a node

### Prerequisites

- Rust toolchain (if building from source) or prebuilt binary
- Agent runtime installed (e.g. `~/.local/bin/goose`)
- Tailscale (or direct network access to the MC backend)
- `~/.missioncontrol/session.json` with a valid token

### Build from source

```bash
# On the target machine (avoids glibc version mismatch)
git clone <repo> && cd missioncontrol/crates/mc
cargo build --release
cp target/release/mc ~/bin/mc
```

### Authenticate

```bash
# OIDC browser flow
curl -s http://<mc-host>/auth/oidc/cli-initiate
# open authorize_url in browser, copy grant_id from success page
curl -s -X POST http://<mc-host>/auth/oidc/exchange \
  -H "Content-Type: application/json" \
  -d '{"grant_id":"olg_…"}' > /tmp/tok.json

# Write session file
MC_HOST=http://<mc-host>
TOKEN=$(jq -r .token /tmp/tok.json)
cat > ~/.missioncontrol/session.json <<EOF
{"token":"$TOKEN","subject":"$(jq -r .subject /tmp/tok.json)",
 "email":"$(jq -r .email /tmp/tok.json)",
 "expires_at":"$(jq -r .expires_at /tmp/tok.json)",
 "base_url":"$MC_HOST","session_id":$(jq -r .session_id /tmp/tok.json)}
EOF
chmod 600 ~/.missioncontrol/session.json
```

---

## Run the work loop

### Enroll an agent

```bash
MC_BASE_URL=http://<mc-host> mc daemon agent enroll \
  --mission <mission-id> \
  --runtime goose
```

### Start the loop

```bash
PATH="$HOME/.local/bin:$PATH" \
MC_BASE_URL=http://<mc-host> \
MC_LITELLM_HOST=http://<litellm-host>:4000 \
MC_LITELLM_API_KEY=<key> \
mc run goose --mission <mission-id>
```

Run as a systemd user service for persistence:

```ini
# ~/.config/systemd/user/mc-goose.service
[Unit]
Description=MissionControl Goose work loop
After=network-online.target

[Service]
Environment=PATH=/home/%u/.local/bin:/usr/local/bin:/usr/bin:/bin
Environment=MC_BASE_URL=http://<mc-host>
Environment=MC_LITELLM_HOST=http://<litellm-host>:4000
Environment=MC_LITELLM_API_KEY=<key>
ExecStart=/home/%u/bin/mc run goose --mission <mission-id>
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
```

```bash
systemctl --user enable --now mc-goose
```

---

## Create and dispatch work

### Create a MeshTask (via work API)

```bash
TOKEN=mcs_…
KLUSTER_ID=<id>

curl -X POST http://<mc-host>/work/klusters/$KLUSTER_ID/tasks \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "task title",
    "description": "what to do",
    "claim_policy": "first_claim",
    "priority": 5
  }'
```

`claim_policy` options: `first_claim` (any available agent), `assigned` (specific agent), `broadcast` (all agents).

Tasks are auto-set to `ready` when created with no `depends_on`. The work loop picks them up on startup poll or via WebSocket `task_ready` events.

### Retry a failed task

```bash
curl -X POST http://<mc-host>/work/tasks/<task-id>/retry \
  -H "Authorization: Bearer $TOKEN"
```

---

## Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `MC_BASE_URL` | `http://localhost:8008` | Backend URL |
| `MC_LITELLM_HOST` | `http://litellm:4000` | LiteLLM proxy URL |
| `MC_LITELLM_API_KEY` | _(none)_ | LiteLLM master key → sets `LITELLM_API_KEY` for Goose |
| `MC_GOOSE_BIN` | _(PATH lookup)_ | Override path to goose binary (e.g. `~/.local/bin/goose`) |
| `MC_GOOSE_MODEL` | `local-agent` | Model name passed to Goose |

---

## Known limitations

- **Event bus threading**: `task_ready` WebSocket events from sync API handlers may not wake the work loop reliably in single-worker deployments. The startup poll (`/work/klusters/{id}/tasks?status=ready`) is the reliable dispatch path — restart the loop after creating tasks if events don't fire.
- **sudo in tasks**: Goose runs without a TTY; `sudo` will fail unless the node has passwordless sudo configured for the user (`NOPASSWD: ALL` or specific commands in `/etc/sudoers.d/`).
- **GLIBC mismatch**: Build `mc` natively on the target node if it runs an older glibc than the build machine.
- **Tasks vs MeshTasks**: The regular `/missions/{id}/k/{id}/t` task API is the Kanban-style tracker. The work loop only operates on `MeshTask` objects at `/work/klusters/{id}/tasks`. Always use the `/work/` API when creating tasks for agent dispatch.
