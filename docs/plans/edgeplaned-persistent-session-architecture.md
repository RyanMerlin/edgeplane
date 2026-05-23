# edgeplaned: Persistent Session Architecture

**Date:** 2026-05-09
**Status:** Phases 1-3 code-complete as of 0.6.0 (`feat/agent-public-id`). The remaining work — Phase 4 (task-mode dependency-result injection), per-profile cutover, and tmux retirement — is tracked in `2026-05-11-retire-tmux-via-acp-persistent-sessions.md`.
**Context:** Evaluated current edgeplaned implementation against intended long-running agent session model.

> **Architectural update — 2026-05-11:** PTY and xterm.js are dropped from this design. ACP (Agent Client Protocol, via `claude-code-acp`) is the only transport for persistent agent sessions. The web UI and TUI render structured ACP messages, not terminal output. See "Transport: ACP-only" below for the consolidated decision and the consequences for each gap.

---

## Vision

edgeplaned is the node daemon — analogous to RKE2/kubelet, not a job runner. It runs on every physical or virtual node, registers with edgeplane-tower, and manages all agents on that node. Agents are long-running, persistent Claude sessions — not disposable one-shot processes.

**Primary use case:** You're in Boulder. An agent is running in Vail. You open the web UI, find the Vail node, open the agent session, watch the live conversation as structured ACP messages — assistant turns, tool calls, tool results — and type a steering prompt. The agent responds. You close the tab. The session keeps running.

**Secondary use case:** Discrete short tasks (Goose, Codex, Gemini) — headless, one-shot, result returned. This already works.

---

## Current Implementation State

### What works today

- **Headless task execution** — `ClaudeCodeRuntime.inject_task()` spawns `claude -p "<prompt>" --output-format stream-json`, streams `ProgressEvent`s back to the task loop, marks the task complete. Correct for short tasks.
- **Local PTY attach (deprecated by ACP-only decision)** — `attach_gateway` binds a Unix socket at `~/.edgeplane/edgeplaned.sock`. `edgeplane daemon attach <agent-id>` connects locally, negotiates agent ID, then becomes a raw PTY proxy. This path is retained until ACP local attach lands; it does not factor into the persistent-session implementation plan below.
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

The `launch.sh` files in each Aria profile are doing this job today, outside edgeplaned entirely. edgeplaned needs to own this.

#### Gap 2 — No remote ACP relay in controlplane

For remote access (Vail agent, Boulder viewer), ACP messages must flow over a network transport:

- edgeplaned exposes a WebSocket endpoint per persistent agent that proxies the agent's ACP stream (JSON-RPC over stdio) bidirectionally
- edgeplane-tower provides the proxy route: `GET /runtime/nodes/{node_id}/agents/{agent_id}/attach` → upgrades to WS → forwards JSON-RPC frames to the edgeplaned node over Tailscale
- edgeplaned nodes already maintain an outbound connection to controlplane for task loop; that channel is suitable as the back-channel, or edgeplaned accepts inbound WS on a dedicated port (chosen at implementation time)

Because ACP is JSON-RPC over text frames — not a binary terminal stream — the relay is a straightforward message forwarder. No SIGWINCH, no terminal geometry, no escape-sequence handling. The controlplane currently has no such route.

#### Gap 3 — No web UI conversation pane

The web UI (mc-engineer frontend session) needs a structured conversation component — assistant turns, tool calls, tool results, permission prompts — wired to the WS ACP stream from the controlplane. This is a normal chat UI, not a terminal emulator. xterm.js is explicitly **not** used.

#### Gap 4 — No session lifecycle in edgeplaned config

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

### Transport: ACP-only

The persistent-session transport is ACP (Agent Client Protocol) end-to-end. edgeplaned spawns `claude-code-acp` as the agent child process; messages flow as JSON-RPC over stdio inside the node, and as JSON-RPC over WebSocket between nodes / to clients.

What this buys:
- **Structured surface.** Every assistant turn, tool call, tool result, and permission request is a typed event — not bytes to parse. The web UI and TUI render this directly.
- **Trivial relay.** Forwarding ACP between edgeplaned ↔ controlplane ↔ client is a JSON-RPC frame pump. No SIGWINCH, no terminal geometry, no escape sequences.
- **Native cancel + permission.** `session/cancel` and `session/request_permission` are first-class — no need to parse prompt text or trap signals.
- **Process supervision is normal.** `claude-code-acp` is a regular child process. Crash-and-restart works the same way as any supervised service; no PTY allocation, no terminal session leadership.

Runtime gotchas to honor (see `feedback_acp_runtime_gotchas.md`):
- Strip `CLAUDECODE` and `CLAUDE_CODE_*` from the child env before spawning, otherwise the child auto-detects and misbehaves.
- The agent ignores stdin close; shutdown requires `SIGTERM`.

### Persistent session runtime loop

For `session_mode: persistent`, the daemon runs a **session supervisor loop** instead of the task loop:

```
loop {
    spawn claude-code-acp (stdio JSON-RPC, env scrubbed)
    register ACP client handle in attach_gateway (by agent_id)
    issue session/new → record session_id
    on session/update events → forward to log + progress stream + attached clients
    on signal() call → translate to session/prompt
    on process exit / SIGCHLD → wait backoff → restart, with session reload
}
```

The `signal()` method becomes the prompt injection point — it dispatches `session/prompt` over the ACP channel, replacing the `tmux send-keys` approach used by today's `aria-trigger.sh`.

### Remote ACP relay

```
Web UI (Boulder)
  ACP chat pane (structured rendering)
       ↕ WebSocket (JSON-RPC frames)
edgeplane-tower
  GET /runtime/nodes/{node_id}/agents/{agent_id}/attach
  → upgrades to WS
  → forwards JSON-RPC frames to the edgeplaned node
       ↕ WS or multiplexed stream
edgeplaned (Vail node)
  session supervisor holds ACP client handle
  → fans out session/update events to attached relays
  → injects session/prompt from relay input
```

Back-channel between controlplane and edgeplaned:
- **Option A**: edgeplaned exposes a WS server on a local port; controlplane dials in when a client attaches. Simpler; relies on Tailscale for reachability.
- **Option B**: edgeplaned opens a persistent multiplexed connection to controlplane at startup; attach streams are multiplexed over it. More complex but works even without direct reachability.

Tailscale is present on all nodes — Option A is the right call.

### edgeplaned.yaml shape (when ready)

One config file per node at `~/.ep/edgeplaned.yaml`. One daemon per node manages all agents on that node.

```yaml
backend_url: http://edgeplane:8008
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
        capabilities: [edgeplane, engineering]
        profile_path: /home/merlin/code/aria/profiles/mc-engineer
```

The `launch.sh` files are retired once edgeplaned owns the session lifecycle.

---

## Implementation Sequence

### Phase 0 — Capability routing fix (safe to do now)

`ClaudeCodeRuntime::new()` hardcodes 5 capabilities. Custom capabilities (`research`, `orchestration`) won't match task `required_capabilities` filters.

**Files:**
- `edgeplaned-runtimes/src/claude_code.rs` — change `new()` to `new(capabilities: Vec<Capability>)`
- `edgeplaned/src/config.rs` — add `capabilities: Vec<String>` to `AgentEntry`
- `edgeplaned/src/daemon.rs` — pass agent capabilities when constructing runtime

No conflict with any in-flight work. Can be done independently.

### Phase 1 — Persistent session runtime (edgeplaned)

Implement `session_mode: persistent` in `ClaudeCodeRuntime`:
- Add `SessionMode` enum to config
- Add `session_supervisor_loop()` in `claude_code.rs` — spawn `claude-code-acp`, scrub `CLAUDECODE`/`CLAUDE_CODE_*` env, restart on exit, drive ACP `session/new` → fan out `session/update` events
- Wire `signal()` to dispatch `session/prompt` over the ACP channel
- Register the ACP client handle with attach_gateway on launch (keyed by `agent_id`)
- Add `session_mode` branch in `daemon.rs` startup: persistent agents get session supervisor, task agents get task loop

**Gate:** This makes `launch.sh` redundant. Don't retire `launch.sh` until a persistent agent is validated end-to-end via ACP.

### Phase 2 — Remote ACP relay (controlplane + edgeplaned)

- edgeplaned: add WS server on a local port (e.g. `127.0.0.1:8009`) that accepts attach connections identified by `agent_id`. Each connection becomes a JSON-RPC frame pump between the caller and the agent's ACP client handle held by the session supervisor.
- edgeplane-tower: add route `GET /runtime/nodes/{node_id}/agents/{agent_id}/attach` — resolves node Tailscale address, dials edgeplaned WS, upgrades caller to WS, forwards JSON-RPC frames bidirectionally.

**Gate:** Requires Phase 1 (ACP client handles must exist to relay).

### Phase 3 — Web UI conversation pane (mc-engineer frontend)

- Structured chat component in web UI: renders assistant turns, tool calls, tool results, and permission prompts from `session/update` events
- Permission prompts surface as inline approve/deny affordances (no terminal modal hacks)
- Sends `session/prompt` for user input and `session/cancel` for interrupts
- Rendered in the agent detail panel — no terminal emulator dependency

**Gate:** Requires Phase 2 (relay must exist to connect to).

### Phase 4 — Dependency result injection in build_prompt() (edgeplaned)

For task-mode agents: when a task has completed dependencies, auto-fetch `GET /work/tasks/{dep_id}/progress` and append the last `PhaseFinished` summary as a `[DEPENDENCY RESULT]` section in the prompt.

This is the operator→research delegation result delivery path. No new routes needed — uses the WS notify + task dependency chain already in place.

**Gate:** WS notify is already landed. Phase 0 capability fix should land first so task routing works correctly.

---

## What Does NOT Change

- `launch.sh` files: untouched until Phase 1 is validated
- `aria-trigger.sh`: untouched until Phase 1 is validated (signal() replaces it)
- `systemd` timers (`aria-briefing.timer`, etc.): these inject into tmux sessions — they need to be updated to call `edgeplane signal <agent_id> "<prompt>"` once Phase 1 lands, but not before
- Task-mode runtime (`claude -p`): stays as-is, used for Goose/Codex/Gemini short tasks

---

## Open Questions for mc-engineer Session

1. **Attach route design** — should controlplane proxy via Tailscale hostname (Option A) or multiplex over existing edgeplaned→controlplane connection (Option B)?
2. **Session mode field name** — `session_mode: persistent` or a separate runtime kind like `claude_code_session`?
3. **Conversation pane placement** — agent detail panel drawer, full-screen overlay, or tabbed panel alongside task progress?
4. **Multi-client attach** — when two viewers (web UI + TUI) attach to the same agent, does each get its own ACP fan-out of `session/update`, or do they share a single subscription? Implication for input ownership.
5. **Auth on the attach WS** — same bearer token as the rest of the controlplane API, or a short-lived signed URL?
6. **Session history on attach** — when a new viewer attaches mid-session, do they get the conversation so far? ACP supports session load; need to confirm `claude-code-acp` exposes it cleanly.
