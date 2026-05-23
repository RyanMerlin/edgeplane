# Retire tmux: Migrate Aria Fleet to ACP Persistent Sessions via edgeplaned

**Date:** 2026-05-11
**Author:** aria-operator (drafted at Merlin's request)
**Owner:** mc-engineer
**Status:** Code-side complete for Phases 1, 2, B, C, D.1. Phase A validation, D.2-3, and E are operational (require running edgeplaned against the fleet) and remain to do.
**Depends on:** `edgeplaned-persistent-session-architecture.md` (the umbrella architecture), `2026-05-11-agent-public-id-edgeplaned-fix.md` (merged in `feat/agent-public-id`)

---

## Status (2026-05-11, post `feat/agent-public-id`)

| Phase | Status | Notes |
|---|---|---|
| 1 — Persistent session runtime in edgeplaned | ✅ Complete | `acp_session_supervisor`, `AcpSession`, daemon wiring all in. |
| 2 — Remote ACP relay | ✅ Complete | `agent_attach_proxy` (controlplane), `attach_ws` + `pump_acp` (edgeplaned). |
| A — Validate Phase 1 with operator-acp-test | 🟡 Ready | Code is in; needs operational setup. The CLI viewer + web viewer + `edgeplane signal` all exist as test surfaces. |
| B — `edgeplane agent attach` CLI | ✅ Complete | `edgeplane agent attach <id> [--json]`. URL builder unit-tested; integration covered by Phase A walkthrough. |
| C — Web conversation pane | ✅ Complete (scaffold + history replay) | `/agents/<id>/` route, `AgentConversation.svelte`, typed `AcpAttach` client. History-replay on reconnect ships in the same commit as the supervisor's `ReplayBroadcast`. Permission-request UI deferred until the supervisor surfaces those frames. |
| D.1 — `edgeplane signal` verb | ✅ Complete | `edgeplane signal <to-id> --content "…"`; auto-creates `edgeplane-signal-<host>` sender; `--dry-run` for systemd validation. |
| D.2 — systemd unit migration | 🟡 To do | Operational change. `edgeplane signal` is ready to drop into ExecStart lines. |
| D.3 — Dual-write soak | 🟡 To do | Depends on D.2. |
| E — Per-profile cutover + tmux retire | 🟡 To do | Depends on A and D running clean. |

---

## Why this exists

Today, all six Aria profiles run as `claude` processes inside dedicated `tmux` sessions launched by `profiles/<name>/launch.sh`. tmux is the host of last resort — it keeps the Claude process alive across SSH disconnects and lets Merlin attach a terminal to see "what each agent's screen looks like."

**This is the wrong layer.** Edgeplane should be that layer. The umbrella plan (`edgeplaned-persistent-session-architecture.md`) calls for `edgeplaned` to own persistent agent processes and expose their conversations as **structured ACP message streams** to a browser-rendered conversation pane in edgeplane-tower's web UI.

The umbrella plan has 5 phases. Most of the code is already in:

| Phase | Status (verified 2026-05-11) |
|---|---|
| 0 — Capability routing fix | ✅ Landed (config carries capability lists, daemon honors them) |
| 1 — Persistent session runtime in edgeplaned | ✅ Code-complete (`acp_session_supervisor.rs`, `claude_agent_acp.rs`, `SessionMode::Persistent` wired in `daemon.rs:647`). **Not validated against any real Aria profile.** |
| 2 — Remote ACP relay (controlplane ↔ edgeplaned) | ✅ Code-complete (`agent_attach_proxy` in `runtime.rs:2598`, `attach_ws` server in edgeplaned). **Not validated end-to-end.** |
| 3 — Web UI conversation pane | ❌ Not built. No `session/update` rendering in `web/src/`. |
| 4 — Dependency result injection | ⬜ Independent of tmux retirement; deferred |

So the work split is:
- **Validate** what's already built (Phase 1, Phase 2)
- **Build** what's missing (Phase 3 web UI + `edgeplane signal` CLI verb + systemd timer migration)
- **Cut over** one profile at a time

This plan walks that path.

---

## Goal (concrete success criteria)

When this plan is complete:

1. `systemctl --user start aria-mesh-node.service` brings up edgeplaned on excalibur
2. edgeplaned reads its config from `~/.ep/edgeplaned.yaml` (or pulled from controlplane) and launches all six Aria profiles as `claude-code-acp` children
3. **No `aria-*` tmux session exists.** `tmux list-sessions` returns nothing (or only Merlin's personal sessions, none auto-created)
4. **No `profiles/<name>/launch.sh` is invoked.** Those scripts can be deleted from the repo (or kept as legacy reference, but `chmod -x`)
5. Merlin opens `https://edgeplane/agents/aria-operator` in a browser and sees the live conversation — assistant turns, tool calls, permission prompts — rendered as structured chat. Typing in the input field sends `session/prompt`; clicking "Cancel" sends `session/cancel`. No terminal emulator anywhere.
6. systemd timers (`aria-briefing.timer`, `aria-evolve.timer`, etc.) call `edgeplane signal <agent-id> "<prompt>"` instead of `aria-trigger.sh`. Briefings still arrive at 5:30 AM
7. If a Claude child crashes, the edgeplaned supervisor restarts it with the documented backoff (1s → 60s, reset on 30s+ stable runs). No human intervention
8. Merlin can be in Boulder, attach to a node in Vail via the same web UI, and steer that agent from his laptop

---

## Non-goals

- **Task-mode agents stay task-mode.** Goose, Codex, Gemini short-lived task workers are not in scope. Phase 1's `session_mode: task` continues to handle them.
- **No new RBAC / multi-tenant changes.** This is a transport migration. Existing auth (Authentik + session tokens) is reused as-is.
- **No edgeplane-tower database migrations.** All required schema (agent public_id, attach_secret) is already landed by `2026-05-11-agent-public-id-edgeplaned-fix.md`.
- **No removal of `aria-trigger.sh` until validated** — the script stays in place during cutover so timers can fall back if needed; gets removed in Phase E.

---

## Implementation Sequence

### Phase A — Validate Phase 1 with one profile (operator)

**Goal:** confirm that the already-landed persistent-session supervisor actually runs an Aria profile end-to-end without tmux.

**Risk hedge:** validate with **`operator`** profile. It's the meta-orchestrator, but if it churns, the other 5 profiles continue running on tmux. The other profiles must NOT be touched in this phase.

**A.1 — Construct an `edgeplaned.yaml` entry for operator**

File: `~/.ep/edgeplaned.yaml` (on excalibur)

```yaml
backend_url: http://edgeplane:8008
node_id: excalibur

agents:
  - agent_id: aria-operator-acp-test
    runtime_kind: claude_agent_acp
    session_mode: persistent
    capabilities: [orchestration, fleet-management, claude-code]
    profile_path: /home/merlin/code/aria/profiles/operator
```

Key choices:
- `agent_id: aria-operator-acp-test` — deliberately suffixed so it cannot collide with the live tmux-hosted operator while we validate. Live operator continues running in tmux throughout Phase A.
- `runtime_kind: claude_agent_acp` — the new runtime, not `claude_code`.
- `session_mode: persistent` — triggers the `acp_session_supervisor` path at `daemon.rs:662`.
- `profile_path` — passed through to the supervisor's `cwd`; the spawned `claude-code-acp` inherits the operator's CLAUDE.md and skills.

**A.2 — Spawn edgeplaned in foreground mode for observability**

```bash
# Pseudo-systemd unit:
ExecStart=edgeplaned --node-id excalibur --config ~/.ep/edgeplaned.yaml --log-level debug
```

For Phase A, run in a foreground terminal (not systemd) so we can see logs and SIGINT freely.

**A.3 — Exit criteria for Phase A**

In order, all must hold:

1. **Process is alive.** `pgrep -f "claude-code-acp" | wc -l` returns at least 1
2. **Environment is correct.** `cat /proc/<pid>/environ | tr '\0' '\n' | grep ^CLAUDE` returns empty (`CLAUDECODE` and `CLAUDE_CODE_*` were stripped per `feedback_acp_runtime_gotchas.md`)
3. **ACP session is open.** edgeplaned logs `session/new` accepted, broadcast channel established
4. **Prompt injection works.** Run `edgeplane agent remote message --agent-id <id> --to-agent-id aria-operator-acp-test --content "respond with just 'hello'"`. Within 30s, an assistant turn appears in edgeplaned logs and the agent's `session/update` stream broadcasts it
5. **Crash recovery works.** Find the child PID, `kill -9 <child>`. Supervisor logs "Agent process exited", backoff timer fires, new child spawns within 5s, ACP session re-establishes
6. **Capabilities are claimable.** Create a task with `required_capabilities: [orchestration]`, claim succeeds against the persistent agent

**A.4 — On exit criteria pass:** snapshot the working config + log a session into Graphiti under `group_id=infra`. Move to Phase B.

**A.5 — On exit criteria fail:** the failure is the answer. Diagnose, fix in `acp_session_supervisor.rs` / `claude_agent_acp.rs` / `daemon.rs`. Phase A re-runs from A.2. Do NOT move past Phase A until all 6 criteria pass for at least one 24h continuous run.

---

### Phase B — Validate Phase 2 attach proxy with a CLI consumer

**Goal:** prove the controlplane→mesh ACP relay (`agent_attach_proxy` at `runtime.rs:2598`) works end-to-end before building the web UI on top of it.

**B.1 — Build `edgeplane agent attach` CLI verb**

A new subcommand under `edgeplane agent`:

```
edgeplane agent attach <agent-id> [--json]
```

Behavior:
- Resolves `agent-id` → `node_id` via existing agent listing API
- Mints an attach token via `POST /runtime/nodes/{node_id}/attach-secret` (or reuses cached)
- Opens WebSocket to `GET /runtime/nodes/{node_id}/agents/{agent_id}/attach`
- Streams `session/update` frames to stdout as either:
  - Human-readable rendering (default): `[assistant] ...`, `[tool_call] ...`, `[permission_request] ...`
  - Raw JSON-RPC (`--json`): one frame per line
- Reads stdin line-by-line; each line becomes a `session/prompt` sent over the WS
- `Ctrl-C` sends `session/cancel` and exits cleanly

**Files to touch:**
- `crates/edgeplane/src/commands.rs` — add `Attach(AttachArgs)` to `AgentCommand`
- `crates/edgeplane/src/attach.rs` (new) — WS client logic, tungstenite already in deps
- `crates/edgeplane/src/main.rs` — dispatch wired

**B.2 — Exit criteria for Phase B**

1. `edgeplane agent attach aria-operator-acp-test` shows live assistant turns as the agent processes a prompt sent via a separate `edgeplane agent remote message`
2. Typing in the attach session steers the agent (prompt arrives in agent log within 1s)
3. Ctrl-C cleanly cancels in-flight turn
4. Attach survives a Claude child restart (Phase A.5 scenario) — the WS reconnects to the new ACP session automatically (or fails cleanly with a "session ended, run `edgeplane agent attach` again" message — both are acceptable; document which behavior we land on)

**B.3 — Phase B isn't blocking Phase C:** Phase C (web UI) can start in parallel; both consume the same controlplane endpoint and can be debugged against each other. But Phase B's CLI consumer is the simpler test surface — get it green first, then proceed to web.

---

### Phase C — Web UI conversation pane

**Goal:** render the live ACP session in a browser, no terminal emulator.

This is the user-visible payoff. It's also the largest unknown — no scaffolding exists in `web/src/` for this yet.

**C.1 — Component scaffolding**

Add a Svelte route `web/src/routes/agents/[agentId]/+page.svelte` (or extend an existing agent detail panel — check `edgeplane-frontend-overhaul-plan.md` for naming).

Children:
- `ConversationPane.svelte` — renders a flat list of `session/update` events
- `AssistantTurn.svelte` — formats assistant message text + thinking blocks (collapsible)
- `ToolCall.svelte` — tool name, input JSON (collapsible), result JSON (collapsible), elapsed time
- `PermissionRequest.svelte` — inline approve/deny with reason text
- `PromptInput.svelte` — textarea + send button; submits `session/prompt`
- `CancelButton.svelte` — sends `session/cancel` on the active turn

**C.2 — WebSocket plumbing**

`web/src/lib/acp-attach.ts` (new):
- Opens WS to `/runtime/nodes/{node_id}/agents/{agent_id}/attach` (relative URL — controlplane is the origin)
- Parses incoming JSON-RPC frames into typed Svelte stores
- Emits `prompt`, `cancel` outbound

Re-uses the auth pattern from existing edgeplane-tower web pages (session cookie / token).

**C.3 — Exit criteria for Phase C**

1. Open `https://edgeplane/agents/aria-operator-acp-test` in a fresh browser tab
2. The full conversation history since last process restart renders (replay), then live updates stream as new events arrive
3. Typing a prompt and clicking Send → agent receives it, response renders within Claude's normal latency
4. A long-running tool call shows a spinner + elapsed time; result renders inline when done
5. Permission requests (e.g. agent tries to write a file) surface as a card with Approve / Deny buttons; clicking emits the right `session/request_permission` response
6. Closing the tab does NOT terminate the agent; reopening reconnects and replays from the last event

**C.4 — Design discipline (don't regress to a terminal)**

- Do NOT add `xterm.js` or any ANSI-escape rendering. Period.
- Do NOT proxy raw stdout. The renderer only consumes typed `session/update` payloads.
- If a non-ACP event appears (e.g. Claude logs a deprecation warning to stderr), drop it. Logs belong in edgeplaned's `journalctl`, not the conversation pane.

---

### Phase D — `edgeplane signal` CLI + systemd timer migration

**Goal:** systemd timers no longer touch tmux.

**D.1 — Add `edgeplane signal` verb**

Under `edgeplane agent`:

```
edgeplane signal <agent-id> --content "<prompt-text>"
```

Implementation: wraps `edgeplane agent remote message` with sender resolved to the local node's identity (saves the operator from passing `--agent-id` twice). Internally, this becomes an `AgentSignal::PeerMessage` delivered via the existing message relay, which the supervisor renders as `session/prompt` with a `[system]` provenance prefix.

**Why a thin wrapper instead of just using `edgeplane agent remote message`:** ergonomic. The systemd unit file becomes `ExecStart=edgeplane signal aria-operator-acp-test --content "Run /briefing skill..."` — readable, single-line, easy to grep for in `systemctl --user list-timers`.

**D.2 — Migrate each timer**

Files in `/home/merlin/.config/systemd/user/`:

- `aria-briefing.service` (5:30 AM daily) → `ExecStart=edgeplane signal aria-operator-acp-test --content "Run /briefing skill..."`
- `aria-evolve.service` (Sunday 11:30 PM) → same pattern
- `aria-earnings-check.service` (Sunday 1:47 PM) → `edgeplane signal aria-research-acp-test ...`
- `aria-kb-curate.service` (2 AM daily) → same pattern
- `aria-vault-lint.service` (Sunday 11:45 PM) → same pattern

**D.3 — Run timers in dual-write mode for one week**

Initially, each timer fires BOTH the old `aria-trigger.sh` path AND `edgeplane signal`. This proves the new path works without losing data if it fails. After 7 days of clean dual-runs, drop the old path.

**D.4 — Exit criteria for Phase D**

1. All 5 timers converted to `edgeplane signal` (initially dual-write)
2. One full week of timers fire successfully on the new path (verified by output landing in expected destinations: briefings delivered inline to operator, evolve runs, etc.)
3. `aria-trigger.sh` calls removed from systemd unit files
4. `aria-trigger.sh` script `chmod -x`'d (kept for reference, can't be accidentally invoked)

---

### Phase E — Migrate remaining 5 profiles + retire tmux

**Goal:** all six profiles on edgeplaned, tmux gone.

**E.1 — One-at-a-time profile migration**

For each profile in this order (safest first):
1. `aria-research` (least critical, mostly task-mode work already)
2. `aria-work` (similar)
3. `aria-publisher` (recent, less battle-tested by tmux)
4. `aria-merlinlabs` (homelab — failure modes are recoverable)
5. `aria-mc` (mc-engineer itself — this is the meta-loop, do last)
6. `aria-operator` (FLIP the validated `-acp-test` to canonical; retire the tmux operator)

For each:
- Add an `agents:` entry in `~/.ep/edgeplaned.yaml` mirroring Phase A's pattern
- Stop the tmux session: `tmux kill-session -t aria-<name>`
- Verify the edgeplaned-launched copy is responsive via `edgeplane agent attach`
- Monitor for 24h before moving to the next

**E.2 — Retire launch infrastructure**

After all six are migrated:
- `chmod -x profiles/*/launch.sh` (or move to `profiles/*/launch.sh.legacy`)
- Update `profiles/*/CLAUDE.md`: remove references to tmux session names
- Update `aria-rs/src/cli/watchdog.rs` if it checks tmux sessions (replace with edgeplaned agent status check)
- Update operator `CLAUDE.md` dispatch table — remove "tmux send-keys" patterns, document `edgeplane signal` / `edgeplane agent attach` as the canonical UX

**E.3 — Exit criteria for Phase E**

1. `tmux list-sessions` shows no `aria-*` sessions
2. `systemctl --user status aria-mesh-node.service` shows active running
3. `edgeplane agent list` shows all 6 profiles with status `online`, hosted on `excalibur`
4. Merlin attaches to each profile via web UI, verifies the conversation pane renders correctly
5. One full day passes with all systemd timers firing cleanly via `edgeplane signal`
6. Update `SOUL.md` / `USER.md` if either mentions tmux as the agent-host (probably not, but check)

---

## Rollback Strategy

At every phase, the old path is preserved until the new path is proven. Rollback per phase:

- **A**: kill the `-acp-test` agent; live tmux operator is untouched. Zero impact.
- **B**: `edgeplane agent attach` failure doesn't affect agent execution — it only affects observability.
- **C**: web UI failures don't break the agent — fall back to `edgeplane agent attach` CLI for visibility.
- **D**: dual-write means a `edgeplane signal` failure has the old `aria-trigger.sh` path as backup for the first week.
- **E**: revert the `~/.ep/edgeplaned.yaml` entry for a profile, restart its `launch.sh`. Tmux session re-appears. ~30s of churn per profile.

**Hardest rollback point: Phase E.2.** After `launch.sh` is removed/disabled, going back means resurrecting the scripts. Mitigation: leave `launch.sh.legacy` in place for 30 days before deletion.

---

## What's Out of Scope (Explicitly)

- **`session_mode: task` agents** — Goose, Codex, Gemini stay where they are. This plan is about persistent interactive sessions only.
- **Multi-node fleet** — the plan validates on excalibur only. Vail / mobile node deployment is a future plan; this work makes it possible but doesn't deliver it.
- **ACP protocol changes** — we consume `claude-code-acp` as-is. Any wire-format issues should be filed upstream, not patched locally.
- **Auth model changes** — controlplane's existing session-token auth covers the attach proxy. No new identity primitives needed.
- **Phase 4 of the umbrella plan (dependency result injection)** — orthogonal, doesn't block tmux retirement, scheduled separately.

---

## Estimated Effort

Generous estimates (mc-engineer can recalibrate):

| Phase | Active dev time | Calendar time (incl. 24h soak) |
|---|---|---|
| A | 0.5-1 day | 2 days (24h validation soak) |
| B | 1-1.5 days | 2 days |
| C | 2-3 days | 4 days |
| D | 0.5 day + 7 days dual-write soak | 8 days |
| E | 0.5 day per profile × 6 + 1 day infra cleanup | 10 days (with per-profile 24h soak) |
| **Total** | **8-10 days dev** | **~4 weeks calendar** |

Parallelization opportunities:
- C can start as soon as A is green (doesn't need B)
- D can start as soon as A is green (doesn't need B or C)
- Phase E profile migrations can run in parallel with Phase D's dual-write soak

Realistic compressed schedule with parallel work: **~2.5 weeks calendar**.

---

## Open Questions (mc-engineer to resolve during execution)

1. **`edgeplane signal` semantics on a queued turn** — ACP allows only one prompt turn at a time per session. If a signal arrives while the agent is mid-turn, do we queue (FIFO, unbounded?), reject (back-pressure on caller), or replace (cancel + new)? Recommend: queue with a small bounded buffer (8 deep), reject with clear error past that.
2. **Web UI auth model for attach** — does the existing session cookie cover WS upgrade? If not, mint a short-lived attach token via REST first and pass via subprotocol. The `attach_token_prefix` machinery in `runtime.rs` is already there — verify it works for browser clients.
3. **Profile path layout for ACP children** — does `claude-code-acp` honor the same `CLAUDE.md` discovery as `claude`? Verify by reading the upstream agent code or by running A's exit criteria #2.
4. **Migration of `aria-mc` (the mc-engineer itself)** — this agent is editing the very code that hosts it. Plan a "self-hosted bootstrap" moment carefully — probably means killing tmux mc-engineer, letting edgeplaned launch a fresh one, with a short window where there's no mc-engineer running. Acceptable for ~5 minutes.

---

## Cross-References

- Umbrella architecture: `docs/plans/edgeplaned-persistent-session-architecture.md`
- Public ID prerequisite: `docs/plans/2026-05-11-agent-public-id-edgeplaned-fix.md`
- ACP runtime gotchas: `feedback_acp_runtime_gotchas.md` (in Aria memory: strip `CLAUDECODE`/`CLAUDE_CODE_*` env, agent ignores stdin close)
- Phase 1 implementation: `crates/edgeplaned/crates/edgeplaned/src/acp_session_supervisor.rs`
- Phase 2 implementation: `crates/edgeplane-tower/src/routes/runtime.rs:2598` (`agent_attach_proxy`)
- Agent identity (public_id): `8c89c1a feat(edgeplane): link meshagent → agent public_id, display in CLI + TUI`
