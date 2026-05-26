# Command Map

This is the authoritative `edgeplane` CLI command hierarchy.

## Top Level

- `edgeplane status`
- `edgeplane doctor`
- `edgeplane health`
- `edgeplane version`
- `edgeplane config`
- `edgeplane use`
- `edgeplane release`
- `edgeplane logs`
- `edgeplane completion`
- `edgeplane auth`
- `edgeplane admin`
- `edgeplane data`
- `edgeplane system`
- `edgeplane agent`
- `edgeplane approvals`
- `edgeplane workspace`
- `edgeplane ops`
- `edgeplane daemon`
- `edgeplane launch`
- `edgeplane run`
- `edgeplane init`
- `edgeplane serve`
- `edgeplane profile`

## quick verbs

- `edgeplane status [--verify-lease]` — combined auth/runtime/attached-workspace status; optional lease validation heartbeat.
- `edgeplane doctor` — shortcut to `edgeplane system doctor`.
- `edgeplane health` — backend MCP health probe.
- `edgeplane version` — local CLI version + backend reachability.
- `edgeplane config` — effective local runtime config (redacted).
- `edgeplane use --profile <name>` — activate/apply profile (API-backed profile flow).
- `edgeplane use --mission-id <id> [--lease-seconds N] [--workspace-label <label>]` — acquire workspace lease lock (API-backed).
- `edgeplane use --release` — release current active lease.
- `edgeplane release [--reason <text>] [--ignore-missing]` — top-level lease release shortcut.
- `edgeplane logs` — local log tail helper (local-only utility).
- `edgeplane completion <shell>` — shell completion generator (local-only utility).
- `edgeplane run claude [-p <profile>] [--mission <id>] [--mode interactive|headless|solo] [-- args]` — launch Claude Code (profile runtime + mesh participation).
- `edgeplane run codex [-p <profile>] [--mission <id>] [--mode interactive|headless|solo] [-- args]` — launch Codex CLI.
- `edgeplane run gemini [-p <profile>] [-- args]` — launch Gemini CLI.
- `edgeplane run claude doctor [-p <profile>] [--fix] [--json]` — inspect/repair Claude runtime readiness.
- `edgeplane run claude exec [-p <profile>] -- [args]` — raw Claude passthrough in prepared runtime.
- `edgeplane run codex doctor [-p <profile>] [--fix] [--json]` — inspect/repair Codex runtime readiness.
- `edgeplane run codex status [-p <profile>] [--json]` — read-only Codex status.
- `edgeplane run codex exec [-p <profile>] -- [args]` — raw Codex passthrough in prepared runtime.
- `edgeplane run claude hook --event <session-start|post-tool-use|session-end>` — internal Claude lifecycle hook (used by hook scripts).

## auth

- `edgeplane auth login`
- `edgeplane auth whoami`
- `edgeplane auth logout`

## admin

- `edgeplane admin policy active`
- `edgeplane admin policy versions`
- `edgeplane admin policy events`
- `edgeplane admin governance ...`

## data

- `edgeplane data tools list`
- `edgeplane data tools call --tool <name> --payload '<json>'`
- `edgeplane data sync status ...`
- `edgeplane data sync promote ...`
- `edgeplane data explorer tree`
- `edgeplane data explorer node ...`

## system

- `edgeplane system doctor --fix`
- `edgeplane system backup --target postgres|rustfs|all`
- `edgeplane system profile-gc ...`
- `edgeplane system update ...`
- `edgeplane system compat ...`
- `edgeplane system drift ...`

## agent

Verb-first surface added in v0.8 (Phase 3 daemon-absorption). Each verb
auto-resolves: asks the local `edgeplaned` mgmt-gateway first, falls through to
the controlplane if the agent isn't known locally. Use `--local` /
`--remote` to force a single path when an id collides between sources.

- `edgeplane agent signal <id> --content "..."` — send a prompt (UserInput).
  Works against both fleet-imported ZellijHosted agents (e.g. `work`,
  `operator`) and controlplane ACP agents (e.g. `aria-operator-…`).
- `edgeplane agent cancel <id>` — interrupt the agent (`Ctrl c` for
  ZellijHosted; `--remote` cancel is not yet implemented).
- `edgeplane agent list [--source local|remote|all] [--json]` — enumerate
  visible agents, tagged by source.
- `edgeplane agent describe <id> [--json]` — show one agent's runtime,
  session, vault folder, supervision state.
- `edgeplane agent attach <id> [--web] [--web-base-url <URL>] [--remote]` —
  dispatches on runtime kind. ZellijHosted → `exec zellij attach
  <session>`; with `--web` prints the `zellij web` URL. ACP →
  WebSocket session/update stream (unchanged).

### `edgeplane agent cron` — scheduled prompts (Phase 4, v0.9)

edgeplaned owns `~/.ep/edgeplaned/cron.toml` (same schema as the legacy `aria-cron.toml`)
and runs its own 1-minute tick loop. Edit the file in `$EDITOR`; CLI is
inspection + reload only.

- `edgeplane agent cron list [--json]` — all jobs from file + last-fire status.
- `edgeplane agent cron describe <name> [--limit N] [--json]` — one job + recent
  fires.
- `edgeplane agent cron reload` — poke edgeplaned to re-parse the file.
- `edgeplane agent cron history [--name <n>] [-n N] [--json]` — recent fires
  across all (or one) job.
- `edgeplane agent cron gc-now [--history-days N] [--max-rows-per-job N]` —
  force a retention sweep.

### `edgeplane agent supervise` — systemd unit liveness (Phase 5, v0.10)

edgeplaned polls each fleet agent's systemd unit every 60s and restarts dead
ones with the same throttling aria-watchdog used (90s post-restart grace,
30-min retry throttle). Plus an optional nightly restart at 03:00.

- `edgeplane agent supervise list [--json]` — supervised agents + live unit
  state.
- `edgeplane agent supervise status <id> [--limit N] [--json]` — one agent +
  recent restart history.
- `edgeplane agent supervise restart <id>` — manual `systemctl --user restart`
  (logged as `reason=manual`).
- `edgeplane agent supervise pause [<id>] [--all]` — disable auto-restart for
  this agent (or all supervised ones).
- `edgeplane agent supervise resume [<id>] [--all]` — re-enable auto-restart.
- `edgeplane agent supervise history [--agent-id <id>] [-n N] [--json]` —
  recent restart events from `unit_restart_log`.
- `edgeplane agent supervise events [--json]` (v0.13) — stream live
  `SupervisorEvent`s as they fire (Ctrl-C to exit). `--json` passes
  raw frames through for `jq` pipelines.
- `edgeplane agent supervise watch [--poll-secs N] [--tail-size N]` (v0.14) —
  ratatui TUI: agent table at top (polled), live event tail at
  bottom (streamed). `q`/Esc/Ctrl-C to quit.

`pause` and `restart` are orthogonal: a paused agent stays paused after
a manual `restart`. Operators run `resume` separately.

REMOVED in 0.11.0 (Phase 6.5):

- `edgeplane signal <id>` (top-level) — use `edgeplane agent signal <id> --remote`.
- `edgeplane agent remote <verb>` — use `edgeplane agent <verb> --remote`.

Other agent surface:

- `edgeplane agent evolve ...` — self-improvement loop for Edgeplane.
- `edgeplane agent node register` — register this node with Edgeplane.
- `edgeplane agent node run` — start the resident node-agent daemon.
- `edgeplane agent node doctor` — validate node-agent connectivity.

## unchanged top-level domains

- `edgeplane approvals ...`
- `edgeplane workspace ...`
- `edgeplane ops ...`
- `edgeplane daemon ...`
- `edgeplane launch ...`
- `edgeplane init ...`
- `edgeplane serve ...`
- `edgeplane profile create <name>` — create empty profile shell on backend.
- `edgeplane profile list` — list profiles owned by current user.
- `edgeplane profile show <name>` — show profile metadata.
- `edgeplane profile activate <name>` — set profile as active default (atomic symlink swap).
- `edgeplane profile use <name>` — activate + download profile in one step (compat alias).
- `edgeplane profile download <name> [--out <path>]` — download bundle to local file.
- `edgeplane profile pull <name>` — pull bundle into local profile cache.
- `edgeplane profile publish <name>` — push local profile bundle to backend.
- `edgeplane profile pin <name> <sha256>` — pin profile to specific content hash.
- `edgeplane profile status <name>` — show local sync status vs backend.
- `edgeplane profile delete <name>` — remove profile from backend.

## Output Modes

- `--output human|json|jsonl`
- `--json` (alias for `--output json`)
- `EP_OUTPUT=human|json|jsonl`
