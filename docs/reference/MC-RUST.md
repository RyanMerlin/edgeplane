# MC — Rust CLI & Daemon Reference

`edgeplane` is the primary operator and agent interface for Edgeplane. It owns all interactivity:
fleet views, agent launch, capability dispatch, secrets management, and the TUI.

`edgeplaned` is the headless executor daemon (like kubelet to `edgeplane`'s kubectl). Agents reach it via
Unix socket; operators never interact with it directly.

`edgeplane-tower` is the Axum HTTP server that backs the REST/SSE API.

## Installation

```bash
cd crates/edgeplane && cargo build --release
cp target/release/edgeplane ~/.local/bin/edgeplane

cd crates/edgeplaned && cargo build --release
cp target/release/edgeplaned ~/.local/bin/edgeplaned

cd crates/edgeplane-tower && cargo build --release
cp target/release/edgeplane-tower ~/.local/bin/edgeplane-tower
```

## Environment

| Var | Meaning | Default |
|-----|---------|---------|
| `EP_BASE_URL` | Backend HTTP base URL | `http://localhost:8008` |
| `EP_TOKEN` | Bearer token | unset |

## Command Surface

### `edgeplane tui`

Full-screen terminal UI for fleet management. Auth and server are global `edgeplane` flags:

```bash
# Server/token come from env, ~/.ep/config.json, or global flags — not on edgeplane tui itself
EP_BASE_URL=http://localhost:8008 edgeplane tui [--mission <id>]
# or:
edgeplane --base-url http://localhost:8008 tui
```

Screens (press key to switch):

| Key | Tab | Description |
|-----|-----|-------------|
| `a` | Agents | Fleet nodes — status, current task, ops |
| `m` | Missions | Missions → Klusters → Tasks (Enter to drill down) |
| `f` | Feed | Live SSE event stream (`p` to pause) |
| `p` | Approvals | Pending approval queue (`y` approve / `n` deny / `s` skip) |
| `s` | Secrets | Infisical folder/secret browser (read-only) |
| `c` | Config | Connection status and server info |
| Ctrl+Q / Ctrl+C | | Quit |

Status bar shows `● connected` / `○ offline` and a live clock.

### `edgeplane run` — Agent Launch

```bash
edgeplane run claude              # Claude Code (default profile)
edgeplane run codex               # Codex
edgeplane run gemini              # Gemini
edgeplane run claude -p <profile> --mission <id> --mode solo
edgeplane run claude doctor [--fix]   # diagnose agent runtime issues
```

### `edgeplane capabilities` — Capability Packs

```bash
edgeplane capabilities                          # list all packs
edgeplane capabilities --tag infra              # filter by tag
edgeplane capabilities describe kubectl.get-pods
edgeplane exec kubectl.get-pods --json          # always use --json for machine output
edgeplane receipts last --json
```

### `edgeplane secrets` — Infisical Profiles

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
edgeplane secrets infisical rm work
```

### Fleet Queries

```bash
edgeplane missions list --json
edgeplane health --json
```

### Machine-Readable Output

All subcommands support `--json` for structured output. Always use `--json` when parsing
programmatically — human-readable output is not a stable interface.

## edgeplaned Daemon

Headless work executor. Agents communicate via Unix socket.

```bash
edgeplaned run --backend-url http://localhost:8008 --token $EP_TOKEN
edgeplaned version
edgeplaned get-secret MY_API_KEY   # inside agent subprocess only
```

Socket paths (`~/.ep/`):
- `edgeplaned-mgmt.sock` — JSON-RPC 2.0 management gateway
- `edgeplaned-secrets.sock` — secrets broker (agents only; injected by edgeplaned)
- `edgeplaned.sock` — PTY attach gateway

### Secrets Broker (inside agent subprocesses)

edgeplaned injects `MC_SECRETS_SOCKET` and `MC_SECRETS_SESSION` instead of raw credentials.

```bash
VALUE=$(edgeplaned get-secret MY_API_KEY)
```

Or speak the protocol directly:

```bash
echo '{"op":"get","session":"'$MC_SECRETS_SESSION'","name":"MY_API_KEY"}' \
  | nc -U "$MC_SECRETS_SOCKET"
```

## edgeplane-tower

Axum HTTP server. Full Rust implementation of the Edgeplane API — missions, klusters, tasks, agents, approvals, governance, SSE telemetry, and OIDC auth. Migrations run automatically on startup via sqlx.

```bash
edgeplane-tower --serve --bind 0.0.0.0:8008
curl http://localhost:8008/health
curl http://localhost:8008/raft/status
```

Native routes: `/health`, `/raft/status`, `/missions`, `/klusters`, `/tasks`, `/agents`.
Everything else proxies to `--api-proxy` with full header forwarding and streaming (SSE-safe).

## Build & Test

```bash
cd crates/edgeplane     && cargo check -p edgeplane
cd crates/edgeplane     && cargo build
cd crates/edgeplane     && cargo test -- --test-threads=1

cd crates/edgeplaned && cargo check
cd crates/edgeplaned && cargo build
cd crates/edgeplaned && cargo test

cd crates/edgeplane-tower && cargo build
```
