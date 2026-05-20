# Command Map

This is the authoritative `mc` CLI command hierarchy.

## Top Level

- `mc status`
- `mc doctor`
- `mc health`
- `mc version`
- `mc config`
- `mc use`
- `mc release`
- `mc logs`
- `mc completion`
- `mc auth`
- `mc admin`
- `mc data`
- `mc system`
- `mc agent`
- `mc approvals`
- `mc workspace`
- `mc ops`
- `mc daemon`
- `mc launch`
- `mc run`
- `mc init`
- `mc serve`
- `mc profile`

## quick verbs

- `mc status [--verify-lease]` — combined auth/runtime/attached-workspace status; optional lease validation heartbeat.
- `mc doctor` — shortcut to `mc system doctor`.
- `mc health` — backend MCP health probe.
- `mc version` — local CLI version + backend reachability.
- `mc config` — effective local runtime config (redacted).
- `mc use --profile <name>` — activate/apply profile (API-backed profile flow).
- `mc use --kluster-id <id> [--lease-seconds N] [--workspace-label <label>]` — acquire workspace lease lock (API-backed).
- `mc use --release` — release current active lease.
- `mc release [--reason <text>] [--ignore-missing]` — top-level lease release shortcut.
- `mc logs` — local log tail helper (local-only utility).
- `mc completion <shell>` — shell completion generator (local-only utility).
- `mc run claude [-p <profile>] [--mission <id>] [--mode interactive|headless|solo] [-- args]` — launch Claude Code (profile runtime + mesh participation).
- `mc run codex [-p <profile>] [--mission <id>] [--mode interactive|headless|solo] [-- args]` — launch Codex CLI.
- `mc run gemini [-p <profile>] [-- args]` — launch Gemini CLI.
- `mc run claude doctor [-p <profile>] [--fix] [--json]` — inspect/repair Claude runtime readiness.
- `mc run claude exec [-p <profile>] -- [args]` — raw Claude passthrough in prepared runtime.
- `mc run codex doctor [-p <profile>] [--fix] [--json]` — inspect/repair Codex runtime readiness.
- `mc run codex status [-p <profile>] [--json]` — read-only Codex status.
- `mc run codex exec [-p <profile>] -- [args]` — raw Codex passthrough in prepared runtime.
- `mc run claude hook --event <session-start|post-tool-use|session-end>` — internal Claude lifecycle hook (used by hook scripts).

## auth

- `mc auth login`
- `mc auth whoami`
- `mc auth logout`

## admin

- `mc admin policy active`
- `mc admin policy versions`
- `mc admin policy events`
- `mc admin governance ...`

## data

- `mc data tools list`
- `mc data tools call --tool <name> --payload '<json>'`
- `mc data sync status ...`
- `mc data sync promote ...`
- `mc data explorer tree`
- `mc data explorer node ...`

## system

- `mc system doctor --fix`
- `mc system backup --target postgres|rustfs|all`
- `mc system profile-gc ...`
- `mc system update ...`
- `mc system compat ...`
- `mc system drift ...`

## agent

Verb-first surface added in v0.8 (Phase 3 daemon-absorption). Each verb
auto-resolves: asks the local `mcd` mgmt-gateway first, falls through to
the controlplane if the agent isn't known locally. Use `--local` /
`--remote` to force a single path when an id collides between sources.

- `mc agent signal <id> --content "..."` — send a prompt (UserInput).
  Works against both fleet-imported ZellijHosted agents (e.g. `work`,
  `operator`) and controlplane ACP agents (e.g. `aria-operator-…`).
- `mc agent cancel <id>` — interrupt the agent (`Ctrl c` for
  ZellijHosted; `--remote` cancel is not yet implemented).
- `mc agent list [--source local|remote|all] [--json]` — enumerate
  visible agents, tagged by source.
- `mc agent describe <id> [--json]` — show one agent's runtime,
  session, vault folder, supervision state.
- `mc agent attach <id> [--web] [--web-base-url <URL>] [--remote]` —
  dispatches on runtime kind. ZellijHosted → `exec zellij attach
  <session>`; with `--web` prints the `zellij web` URL. ACP →
  WebSocket session/update stream (unchanged).

### `mc agent cron` — scheduled prompts (Phase 4, v0.9)

mcd owns `~/.mc/mcd/cron.toml` (same schema as the legacy `aria-cron.toml`)
and runs its own 1-minute tick loop. Edit the file in `$EDITOR`; CLI is
inspection + reload only.

- `mc agent cron list [--json]` — all jobs from file + last-fire status.
- `mc agent cron describe <name> [--limit N] [--json]` — one job + recent
  fires.
- `mc agent cron reload` — poke mcd to re-parse the file.
- `mc agent cron history [--name <n>] [-n N] [--json]` — recent fires
  across all (or one) job.
- `mc agent cron gc-now [--history-days N] [--max-rows-per-job N]` —
  force a retention sweep.

### `mc agent supervise` — systemd unit liveness (Phase 5, v0.10)

mcd polls each fleet agent's systemd unit every 60s and restarts dead
ones with the same throttling aria-watchdog used (90s post-restart grace,
30-min retry throttle). Plus an optional nightly restart at 03:00.

- `mc agent supervise list [--json]` — supervised agents + live unit
  state.
- `mc agent supervise status <id> [--limit N] [--json]` — one agent +
  recent restart history.
- `mc agent supervise restart <id>` — manual `systemctl --user restart`
  (logged as `reason=manual`).
- `mc agent supervise pause [<id>] [--all]` — disable auto-restart for
  this agent (or all supervised ones).
- `mc agent supervise resume [<id>] [--all]` — re-enable auto-restart.
- `mc agent supervise history [--agent-id <id>] [-n N] [--json]` —
  recent restart events from `unit_restart_log`.

`pause` and `restart` are orthogonal: a paused agent stays paused after
a manual `restart`. Operators run `resume` separately.

DEPRECATED — kept for muscle memory; removed in a future cleanup:

- `mc signal <id>` (top-level) — alias for `mc agent signal --remote`.
  Use `mc agent signal` instead.
- `mc agent remote ...` — controlplane-only verbs. Use `mc agent <verb>`
  with optional `--remote` instead.

Other agent surface:

- `mc agent evolve ...` — self-improvement loop for MissionControl.
- `mc agent node register` — register this node with MissionControl.
- `mc agent node run` — start the resident node-agent daemon.
- `mc agent node doctor` — validate node-agent connectivity.

## unchanged top-level domains

- `mc approvals ...`
- `mc workspace ...`
- `mc ops ...`
- `mc daemon ...`
- `mc launch ...`
- `mc init ...`
- `mc serve ...`
- `mc profile create <name>` — create empty profile shell on backend.
- `mc profile list` — list profiles owned by current user.
- `mc profile show <name>` — show profile metadata.
- `mc profile activate <name>` — set profile as active default (atomic symlink swap).
- `mc profile use <name>` — activate + download profile in one step (compat alias).
- `mc profile download <name> [--out <path>]` — download bundle to local file.
- `mc profile pull <name>` — pull bundle into local profile cache.
- `mc profile publish <name>` — push local profile bundle to backend.
- `mc profile pin <name> <sha256>` — pin profile to specific content hash.
- `mc profile status <name>` — show local sync status vs backend.
- `mc profile delete <name>` — remove profile from backend.

## Output Modes

- `--output human|json|jsonl`
- `--json` (alias for `--output json`)
- `MC_OUTPUT=human|json|jsonl`
