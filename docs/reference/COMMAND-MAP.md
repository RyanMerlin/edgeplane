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
- `mc mesh`
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

- `mc agent remote ...`
- `mc agent evolve ...`
- `mc agent node register` — register this node with MissionControl and obtain a join token.
- `mc agent node run` — start the resident node-agent daemon (polls for jobs, reports health).
- `mc agent node doctor` — validate node-agent connectivity and config.

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
