# mc-mesh: Persistent Session Architecture

**Date:** 2026-05-09
**Status:** Design — in-progress (mc-engineer session implementing)
**Context:** Evaluated current mc-mesh implementation against intended long-running agent session model.

---

## Vision

mc-mesh is the node daemon — analogous to RKE2/kubelet, not a job runner. It runs on every physical or virtual node, registers with mc-controlplane, and manages all agents on that node. Agents are long-running, persistent Claude sessions — not disposable one-shot processes.

**Primary use case:** You're in Boulder. An agent is running in Vail. You open the web UI, find the Vail node, open the agent session, watch the live output in an xterm terminal, and type a steering prompt. The agent responds. You close the tab. The session keeps running.

**Secondary use case:** Discrete short tasks (Goose, Codex, Gemini) — headless, one-shot, result returned. This already works.

---

## Current Implementation State

### What works today

- **Headless task execution** — `ClaudeCodeRuntime.inject_task()` spawns `claude -p "<prompt>" --output-format stream-json`, streams `ProgressEvent`s back to the task loop, marks the task complete. Correct for short tasks.
- **Local PTY attach** — `attach_gateway` binds a Unix socket at `~/.missioncontrol/mc-mesh.sock`. `mc mesh attach <agent-id>` connects locally, negotiates agent ID, then becomes a raw PTY proxy. Works for same-machine access.
- **Task loop with WS notify** — `task_loop::run_for_agent()` polls for tasks, claims them, injects via `inject_task()`, forwards progress. WebSocket notify listener (`run_notify_ws`) wakes the loop immediately on `task_available` push — no idle polling.
- **Supervisor** — tracks launched `AgentHandle`s by `agent_id`. Owns the runtime reference.
- **Message relay** — `run_message_relay()` polls inbound peer messages and delivers via `signal()`. `signal()` is currently a no-op stub.

### What is missing for the persistent session model

#### Gap 1 — No persistent session runtime

`ClaudeCodeRuntime` has two modes today:
- `inject_task()` → `claude -p` → exits when done
- `attach_pty()` → opens PTY to interactive Claude (no `-p`) → returns channels

There is no runtime loop that:
1. Launches an interactive Claude session and keeps it alive
2. Restarts it on crash (what `launch.sh` does today with a `while true` loop)
3. Injects prompts into the running session via stdin (not by spawning a new process)
4. Streams stdout continuously for the lifetime of the session

The `launch.sh` files in each Aria profile are doing this job today, outside mc-mesh entirely. mc-mesh needs to own this.

#### Gap 2 — No remote PTY relay in controlplane

`attach_gateway` only accepts local Unix socket connections. For remote access (Vail agent, Boulder viewer):

- mc-mesh needs to expose the PTY stream over a network-capable transport (WebSocket)
- mc-controlplane needs a proxy route: `GET /runtime/nodes/{node_id}/agents/{agent_id}/attach` → upgrades to WS → forwards to the mc-mesh node
- mc-mesh nodes connect to controlplane on startup (for task loop) — the controlplane can use this existing connection as the back-channel, or mc-mesh can accept an inbound WS on a dedicated port

The controlplane currently has no such route. `test_proxy.rs` exists but covers a different proxy path.

#### Gap 3 — No web UI terminal component

The web UI (mc-engineer frontend session) needs an xterm.js terminal wired to the WS attach stream from the controlplane. This is a new component — not in the current web UI build.

#### Gap 4 — No session lifecycle in mc-mesh config

`AgentEntry` today:
```rust
pub struct AgentEntry {
    pub agent_id: String,
    pub runtime_kind: String,
}
```

No field to express "this agent is a persistent session" vs "this agent is a headless task worker." The config needs to express this distinction, and the daemon startup path needs to branch accordingly.

---

## Proposed Architecture

### Runtime modes

Two distinct modes for `ClaudeCodeRuntime`, selected per agent in config:

| Mode | Config | Behavior |
|------|--------|----------|
| `task` (current) | `session_mode: task` | Polls for tasks, runs `claude -p` per task, exits |
| `persistent` (new) | `session_mode: persistent` | Launches interactive Claude, keeps alive, injects via stdin |

`AgentEntry` gains:
```rust
pub struct AgentEntry {
    pub agent_id: String,
    pub runtime_kind: String,
    #[serde(default = "default_session_mode")]
    pub session_mode: SessionMode,   // "task" | "persistent"
    #[serde(default)]
    pub capabilities: Vec<String>,   // override runtime defaults
    #[serde(default)]
    pub profile_path: Option<PathBuf>, // CLAUDE.md / launch context for this agent
}
```

### Persistent session runtime loop

For `session_mode: persistent`, the daemon runs a **session supervisor loop** instead of the task loop:

```
loop {
    launch Claude in interactive PTY mode (no -p)
    register PTY channels in attach_gateway
    monitor stdout → forward to session log / progress stream
    on stdin signal (from signal() call) → write to PTY stdin
    on process exit → wait backoff → restart
}
```

This replaces what `launch.sh` does today. The `signal()` method becomes the injection point — write the prompt to the PTY stdin, same as what `aria-trigger.sh` does via `tmux send-keys`.

### Remote PTY relay

```
Web UI (Boulder)
  xterm.js terminal
       ↕ WebSocket
mc-controlplane
  GET /runtime/nodes/{node_id}/agents/{agent_id}/attach
  → upgrades to WS
  → proxies to mc-mesh node via back-channel
       ↕ WS or multiplexed stream
mc-mesh (Vail node)
  session supervisor holds PTY channels
  → pipes PTY output to relay
  → writes relay input to PTY stdin
```

The controlplane→mc-mesh back-channel can be:
- **Option A**: mc-mesh exposes a WS server on a local port; controlplane dials in when a web UI attaches. Simpler but requires mc-mesh to be network-reachable from controlplane (Tailscale handles this).
- **Option B**: mc-mesh opens a persistent multiplexed connection to controlplane at startup; attach streams are multiplexed over it. More complex but works even without direct reachability.

Tailscale is present on all nodes — Option A is the right call.

### mc-mesh.yaml shape (when ready)

One config file per node at `~/.mc/mc-mesh.yaml`. One daemon per node manages all agents on that node.

```yaml
backend_url: http://missioncontrol:8008
node_id: vail-epyc   # registered in controlplane

missions:
  - mission_id: aria-core
    agents:
      - agent_id: aria-operator
        runtime_kind: claude_code
        session_mode: persistent
        capabilities: [orchestration, fleet-management]
        profile_path: /home/merlin/code/aria/profiles/operator

      - agent_id: aria-research
        runtime_kind: claude_code
        session_mode: persistent
        capabilities: [research, analysis]
        profile_path: /home/merlin/code/aria/profiles/research

      - agent_id: aria-work
        runtime_kind: claude_code
        session_mode: persistent
        capabilities: [work, alteryx]
        profile_path: /home/merlin/code/aria/profiles/work

      - agent_id: aria-merlinlabs
        runtime_kind: claude_code
        session_mode: persistent
        capabilities: [kubernetes, homelab, infra]
        profile_path: /home/merlin/code/aria/profiles/merlinlabs

      - agent_id: aria-mc-engineer
        runtime_kind: claude_code
        session_mode: persistent
        capabilities: [missioncontrol, engineering]
        profile_path: /home/merlin/code/aria/profiles/mc-engineer
```

The `launch.sh` files are retired once mc-mesh owns the session lifecycle.

---

## Implementation Sequence

### Phase 0 — Capability routing fix (safe to do now)

`ClaudeCodeRuntime::new()` hardcodes 5 capabilities. Custom capabilities (`research`, `orchestration`) won't match task `required_capabilities` filters.

**Files:**
- `mc-mesh-runtimes/src/claude_code.rs` — change `new()` to `new(capabilities: Vec<Capability>)`
- `mc-mesh/src/config.rs` — add `capabilities: Vec<String>` to `AgentEntry`
- `mc-mesh/src/daemon.rs` — pass agent capabilities when constructing runtime

No conflict with any in-flight work. Can be done independently.

### Phase 1 — Persistent session runtime (mc-mesh)

Implement `session_mode: persistent` in `ClaudeCodeRuntime`:
- Add `SessionMode` enum to config
- Add `session_supervisor_loop()` in `claude_code.rs` — launch interactive Claude, restart on exit, pipe stdin/stdout
- Wire `signal()` to write to PTY stdin
- Register PTY with attach_gateway on launch
- Add `session_mode` branch in `daemon.rs` startup: persistent agents get session supervisor, task agents get task loop

**Gate:** This makes `launch.sh` redundant. Don't retire `launch.sh` until a persistent agent is validated end-to-end.

### Phase 2 — Remote PTY relay (controlplane + mc-mesh)

- mc-mesh: add WS server on a local port (e.g. `127.0.0.1:8009`) that accepts attach connections identified by `agent_id`. Each connection proxies to the PTY channels registered by the session supervisor.
- mc-controlplane: add route `GET /runtime/nodes/{node_id}/agents/{agent_id}/attach` — resolves node Tailscale address, dials mc-mesh WS, upgrades caller to WS, proxies bidirectionally.

**Gate:** Requires Phase 1 (PTY channels must exist to relay).

### Phase 3 — Web UI terminal (mc-engineer frontend)

- xterm.js terminal component in web UI
- Connects to controlplane attach route via WS
- Resize events forwarded (SIGWINCH)
- Rendered in the agent detail panel

**Gate:** Requires Phase 2 (relay must exist to connect to).

### Phase 4 — Dependency result injection in build_prompt() (mc-mesh)

For task-mode agents: when a task has completed dependencies, auto-fetch `GET /work/tasks/{dep_id}/progress` and append the last `PhaseFinished` summary as a `[DEPENDENCY RESULT]` section in the prompt.

This is the operator→research delegation result delivery path. No new routes needed — uses the WS notify + task dependency chain already in place.

**Gate:** WS notify is already landed. Phase 0 capability fix should land first so task routing works correctly.

---

## What Does NOT Change

- `launch.sh` files: untouched until Phase 1 is validated
- `aria-trigger.sh`: untouched until Phase 1 is validated (signal() replaces it)
- `systemd` timers (`aria-briefing.timer`, etc.): these inject into tmux sessions — they need to be updated to call `mc signal <agent_id> "<prompt>"` once Phase 1 lands, but not before
- Task-mode runtime (`claude -p`): stays as-is, used for Goose/Codex/Gemini short tasks

---

## Open Questions for mc-engineer Session

1. **Attach route design** — should controlplane proxy via Tailscale hostname (Option A) or multiplex over existing mc-mesh→controlplane connection (Option B)?
2. **Session mode field name** — `session_mode: persistent` or a separate runtime kind like `claude_code_session`?
3. **Web UI terminal placement** — agent detail panel drawer, full-screen overlay, or tabbed panel alongside task progress?
4. **Resize handling** — xterm.js sends resize events; WS relay needs to forward SIGWINCH to the PTY
5. **Auth on the attach WS** — same bearer token as the rest of the controlplane API, or a short-lived signed URL?
