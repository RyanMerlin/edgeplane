# Agent Instructions — MissionControl

## Build & Check

The primary surface is the Rust `mc` CLI and `mcd` daemon.

```bash
# Quick syntax/type check (no linking)
cd crates/mc && cargo check -p mc
cd crates/mcd && cargo check --workspace

# Full build
cd crates/mc && cargo build
cd crates/mcd && cargo build --workspace

# Tests — prefer cargo nextest run (CI uses it)
cargo nextest run --manifest-path crates/mc/Cargo.toml --test-threads 1
cargo nextest run --manifest-path crates/mcd/Cargo.toml --workspace
cargo nextest run --manifest-path crates/mc-controlplane/Cargo.toml \
  --test test_health --test test_routes --test test_proxy --test test_work
```

The Python FastAPI backend at `backend/` is still present for legacy proxy use but
is not the primary development target. The Rust `mc-controlplane` (Axum) is the active
server implementation.

## Agent Launch

```bash
mc run claude              # Claude Code agent (default profile)
mc run codex               # Codex agent
mc run gemini              # Gemini agent

mc run claude -p <profile> --mission <id> --mode solo
mc run claude doctor [--fix]   # diagnose agent runtime issues
```

## Capabilities (mc exec)

```bash
mc capabilities                          # list all packs
mc capabilities --tag infra              # filter by tag
mc capabilities describe kubectl.get-pods
mc exec kubectl.get-pods --json          # run; always use --json for machine output
mc receipts last --json                  # last execution result
```

## Secrets — Infisical Profiles

```bash
mc secrets infisical add work \
  --service-token st.xxx \
  --project-id abc123 \
  --environment prod \
  --activate

mc secrets infisical list
mc secrets infisical use work
mc secrets infisical get MY_SECRET_NAME --reveal
mc secrets infisical test
```

## Secrets — Broker (inside agent subprocesses)

When mcd launches a capability subprocess it injects `MC_SECRETS_SOCKET` and
`MC_SECRETS_SESSION` instead of raw credential values. Use the helper to fetch:

```bash
VALUE=$(mcd get-secret MY_API_KEY)
```

Or speak the socket protocol directly:
```bash
echo '{"op":"get","session":"'$MC_SECRETS_SESSION'","name":"MY_API_KEY"}' \
  | nc -U "$MC_SECRETS_SOCKET"
```

## mcd Daemon

```bash
mcd run --backend-url http://localhost:8008 --token $MC_TOKEN
mcd version
```

Socket locations (`~/.mc/`):
- `mcd-mgmt.sock` — JSON-RPC 2.0 management gateway
- `mcd-secrets.sock` — secrets broker (agents only)
- `mcd.sock` — PTY attach gateway

## Server (mc-controlplane)

```bash
mc-controlplane --serve --bind 0.0.0.0:8008 [--api-proxy http://legacy:8000]
curl http://localhost:8008/health
curl http://localhost:8008/raft/status
```

## Machine-Readable Output

All `mc` subcommands support `--json` for structured output:

```bash
mc health --json
mc missions list --json
mc exec <cap> --json
mc receipts last --json
```

Always use `--json` when parsing output programmatically.
