# mc-mesh

Work-first agent coordination daemon for MissionControl. Dispatches tasks to agent runtimes (claude-code, codex, gemini), streams structured progress events, and coordinates agents around a shared mission/kluster.

## Architecture

```
integrations/mc-mesh/
├── crates/
│   ├── mc-mesh/          # daemon binary
│   ├── mc-mesh-core/     # shared types, client, progress events, AgentRuntime trait
│   ├── mc-mesh-work/     # task dispatch, DAG, claim, messaging, watchdog
│   └── mc-mesh-runtimes/ # claude-code, codex, gemini AgentRuntime impls
├── systemd/              # mc-mesh.service unit template
└── scripts/
    ├── install.sh        # build and install daemon + mc
    └── e2e-test.sh       # acceptance test (3-runtime canonical demo)
```

**Mental model:** Temporal-inspired.

| Temporal | mc-mesh |
|---|---|
| Namespace | Mission |
| Workflow | Kluster |
| Activity | Task |
| Worker | Agent runtime |
| Task queue | Kluster's claimable task queue |
| Workflow history | Task progress event stream |
| Signal | Inter-agent message |
| Activity result | Artifact |

## Usage

`mc-mesh` is a headless daemon. All user interaction is through the `mc mesh` subcommand group.

```sh
mc mesh up                          # install and start daemon
mc mesh status                      # daemon health + active agents
mc mesh runtime install claude-code # ensure runtime CLI is available
mc mesh runtime install codex
mc mesh runtime install gemini

mc mesh agent enroll --mission <id> --runtime claude-code
mc mesh agent ls --mission <id>

mc mesh task run <kluster> --title "build it" --runtime claude-code
mc mesh task watch <task-id>        # live progress stream
mc mesh task show <task-id>         # full detail + replay
mc mesh attach <agent-or-task-id>   # PTY into a running agent

mc mesh watch --kluster <id>        # unified live feed
mc mesh watch --mission <id>

mc mesh down
mc mesh upgrade
mc mesh uninstall
```

## Work model

### Tasks

Tasks live inside a kluster and can express a DAG via `depends_on`. A task stays `pending` until all its dependencies reach `finished`, then transitions to `ready` for claiming.

Claim policies:
- `first_claim` — first agent to call `/work/tasks/{id}/claim` wins; task transitions to `claimed`
- `assigned` — pre-assigned to a specific agent
- `broadcast` — every matching agent can claim; task runs concurrently across all claimants

Leases have a TTL (default 120s). Agents must call `/work/tasks/{id}/heartbeat` to renew. Expired leases on non-broadcast tasks are automatically returned to `ready`.

### Progress events

Events are typed structs, not freeform logs:

```
phase_started  phase_finished
step_started   step_finished
artifact_produced  artifact_consumed
waiting_on  unblocked
needs_input  input_received
message_sent  message_received
error  warning  info
```

Events are persisted in `task_progress_event` and can be replayed after the fact with full fidelity.

### Inter-agent messaging

Agents send typed messages scoped to a mission or kluster via `agent_message`. Channels: `coordination`, `handoff`, `question`, `answer`, `artifact_share`, `custom`. The daemon's message relay loop polls the backend every 3s and delivers inbound messages to the runtime via `AgentRuntime::signal()`.

### Watchdog

When the backend is unreachable past the grace period, the watchdog transitions in-flight tasks according to the offline policy:

- `strict` — kill all mutable work immediately
- `safe_readonly` — allow read/monitor ops; block writes
- `autonomous { max_ttl_secs }` — continue whitelisted agents until TTL, then fall back to strict

## Agent runtimes

All three runtimes share the `AgentRuntime` trait and are pluggable:

- **claude-code** — `claude -p "<prompt>" --output-format stream-json`. Parses JSONL into typed `ProgressEvent`s. When `LaunchContext::with_rtk` is true, `ensure_rtk_hooks()` is called once per session before the first task is injected, wiring RTK token-compression hooks into the agent's Claude Code profile.
- **codex** — `codex --approval-mode full-auto --quiet "<prompt>"`. Classifies output lines heuristically.
- **gemini** — `gemini -p "<prompt>" --yolo`. Detects tool calls via `Tool:`/`Calling tool:` prefixes.

All three support interactive PTY attach via `portable-pty`.

### Optional: RTK token compression

[rtk](https://github.com/merlinlabs/rtk) can be installed alongside mc-mesh to compress shell command output before it reaches agent context windows (typically 60–90% token reduction). It is a soft dependency — agents launch normally if it is absent, with a one-time warning. Enable per-launch with `mc run claude --with-rtk` or set `with_rtk: true` on `LaunchContext` when dispatching via mc-mesh directly.

## Backend endpoints

The work model is mounted at `/work/` in the MissionControl backend (`backend/app/routers/work.py`). Key endpoints:

```
POST /work/klusters/{kluster_id}/tasks
GET  /work/klusters/{kluster_id}/tasks
GET  /work/klusters/{kluster_id}/graph
POST /work/tasks/{task_id}/claim
POST /work/tasks/{task_id}/heartbeat
POST /work/tasks/{task_id}/progress
POST /work/tasks/{task_id}/complete
POST /work/tasks/{task_id}/fail
POST /work/missions/{mission_id}/agents/enroll
WS   /work/klusters/{kluster_id}/stream
WS   /work/agents/{agent_id}/attach
```

## Tests

Backend (Python):
```sh
cd backend && ../.venv/bin/python -m unittest tests.test_work -v
```

Rust unit tests are inline in each source file:
```sh
cd integrations/mc-mesh && cargo test
```

End-to-end acceptance (requires running backend + all three CLIs on PATH):
```sh
integrations/mc-mesh/scripts/e2e-test.sh
```

## Installation

```sh
integrations/mc-mesh/scripts/install.sh       # builds from source, installs to ~/.cargo/bin/
mc mesh up                       # starts daemon, optionally installs systemd unit
```

Or via the `mc` bootstrap in `integrations/mc/`:
```sh
curl -fsSL <bootstrap-url> | sh
```
