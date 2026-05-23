# Agent Instructions — Edgeplane

## Build & Check

The primary surface is the Rust `edgeplane` CLI and `edgeplaned` daemon.

```bash
# Quick syntax/type check (no linking)
cd crates/edgeplane && cargo check -p edgeplane
cd crates/edgeplaned && cargo check --workspace

# Full build
cd crates/edgeplane && cargo build
cd crates/edgeplaned && cargo build --workspace

# Tests — prefer cargo nextest run (CI uses it)
cargo nextest run --manifest-path crates/edgeplane/Cargo.toml --test-threads 1
cargo nextest run --manifest-path crates/edgeplaned/Cargo.toml --workspace
cargo nextest run --manifest-path crates/edgeplane-tower/Cargo.toml \
  --test test_health --test test_routes --test test_proxy --test test_work
```

The Python FastAPI backend at `backend/` is still present for legacy proxy use but
is not the primary development target. The Rust `edgeplane-tower` (Axum) is the active
server implementation.

## Agent Launch

```bash
edgeplane run claude              # Claude Code agent (default profile)
edgeplane run codex               # Codex agent
edgeplane run gemini              # Gemini agent

edgeplane run claude -p <profile> --mission <id> --mode solo
edgeplane run claude doctor [--fix]   # diagnose agent runtime issues
```

## Capabilities (edgeplane exec)

```bash
edgeplane capabilities                          # list all packs
edgeplane capabilities --tag infra              # filter by tag
edgeplane capabilities describe kubectl.get-pods
edgeplane exec kubectl.get-pods --json          # run; always use --json for machine output
edgeplane receipts last --json                  # last execution result
```

## Secrets — Infisical Profiles

```bash
edgeplane secrets infisical add work \
  --service-token st.xxx \
  --project-id abc123 \
  --environment prod \
  --activate

edgeplane secrets infisical list
edgeplane secrets infisical use work
edgeplane secrets infisical get MY_SECRET_NAME --reveal
edgeplane secrets infisical test
```

## Secrets — Broker (inside agent subprocesses)

When edgeplaned launches a capability subprocess it injects `EP_SECRETS_SOCKET` and
`EP_SECRETS_SESSION` instead of raw credential values. Use the helper to fetch:

```bash
VALUE=$(edgeplaned get-secret MY_API_KEY)
```

Or speak the socket protocol directly:
```bash
echo '{"op":"get","session":"'$EP_SECRETS_SESSION'","name":"MY_API_KEY"}' \
  | nc -U "$EP_SECRETS_SOCKET"
```

## edgeplaned Daemon

```bash
edgeplaned run --backend-url http://localhost:8008 --token $EP_TOKEN
edgeplaned version
```

Socket locations (`~/.ep/`):
- `edgeplaned-mgmt.sock` — JSON-RPC 2.0 management gateway
- `edgeplaned-secrets.sock` — secrets broker (agents only)
- `edgeplaned.sock` — PTY attach gateway

## Server (edgeplane-tower)

```bash
edgeplane-tower --serve --bind 0.0.0.0:8008 [--api-proxy http://legacy:8000]
curl http://localhost:8008/health
curl http://localhost:8008/raft/status
```

## Machine-Readable Output

All `edgeplane` subcommands support `--json` for structured output:

```bash
edgeplane health --json
edgeplane missions list --json
edgeplane exec <cap> --json
edgeplane receipts last --json
```

Always use `--json` when parsing output programmatically.
