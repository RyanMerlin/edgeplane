# MC — Rust CLI & Daemon Reference

`mc` is the primary operator and agent interface for MissionControl. It owns all interactivity:
fleet views, agent launch, capability dispatch, secrets management, and the TUI.

`mc-mesh` is the headless executor daemon (like kubelet to `mc`'s kubectl). Agents reach it via
Unix socket; operators never interact with it directly.

`mc-controlplane` is the Axum HTTP server that backs the REST/SSE API.

## Installation

```bash
cd integrations/mc && cargo build --release
cp target/release/mc ~/.local/bin/mc

cd integrations/mc-mesh && cargo build --release
cp target/release/mc-mesh ~/.local/bin/mc-mesh

cd integrations/mc-controlplane && cargo build --release
cp target/release/mc-controlplane ~/.local/bin/mc-controlplane
```

## Environment

| Var | Meaning | Default |
|-----|---------|---------|
| `MC_BASE_URL` | Backend HTTP base URL | `http://localhost:8008` |
| `MC_TOKEN` | Bearer token | unset |

## Command Surface

### `mc tui`

Full-screen terminal UI for fleet management.

```bash
mc tui [--server http://localhost:8008] [--token $MC_TOKEN] [--mission <id>]
```

Screens (press key to switch):

| Key | Screen |
|-----|--------|
| `m` | Mission matrix — missions → klusters → tasks |
| `f` | Agent feed — live SSE event stream |
| `a` | Approval queue — pending y/n/e/s decisions |
| `q` | Receipts — capability execution history |
| `s` | Secrets browser — Infisical folder/secret tree |
| `?` | Help |
| Esc | Landing |
| Ctrl+Q / Ctrl+C | Quit |

Status bar shows `mc v<version> · <base_url> · node <id> · <role> · connected <ms>ms`.

### `mc run` — Agent Launch

```bash
mc run claude              # Claude Code (default profile)
mc run codex               # Codex
mc run gemini              # Gemini
mc run claude -p <profile> --mission <id> --mode solo
mc run claude doctor [--fix]   # diagnose agent runtime issues
```

### `mc capabilities` — Capability Packs

```bash
mc capabilities                          # list all packs
mc capabilities --tag infra              # filter by tag
mc capabilities describe kubectl.get-pods
mc exec kubectl.get-pods --json          # always use --json for machine output
mc receipts last --json
```

### `mc secrets` — Infisical Profiles

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
mc secrets infisical rm work
```

### Fleet Queries

```bash
mc missions list --json
mc health --json
```

### Machine-Readable Output

All subcommands support `--json` for structured output. Always use `--json` when parsing
programmatically — human-readable output is not a stable interface.

## mc-mesh Daemon

Headless work executor. Agents communicate via Unix socket.

```bash
mc-mesh run --backend-url http://localhost:8008 --token $MC_TOKEN
mc-mesh version
mc-mesh get-secret MY_API_KEY   # inside agent subprocess only
```

Socket paths (`~/.mc/`):
- `mc-mesh-mgmt.sock` — JSON-RPC 2.0 management gateway
- `mc-mesh-secrets.sock` — secrets broker (agents only; injected by mc-mesh)
- `mc-mesh.sock` — PTY attach gateway

### Secrets Broker (inside agent subprocesses)

mc-mesh injects `MC_SECRETS_SOCKET` and `MC_SECRETS_SESSION` instead of raw credentials.

```bash
VALUE=$(mc-mesh get-secret MY_API_KEY)
```

Or speak the protocol directly:

```bash
echo '{"op":"get","session":"'$MC_SECRETS_SESSION'","name":"MY_API_KEY"}' \
  | nc -U "$MC_SECRETS_SOCKET"
```

## mc-controlplane

Axum HTTP server. Backs `mc` REST/SSE calls and proxies to the Python backend for routes not
yet natively implemented.

```bash
mc-controlplane --serve --bind 0.0.0.0:8008 [--api-proxy http://legacy:8000]
curl http://localhost:8008/health
curl http://localhost:8008/raft/status
```

Native routes: `/health`, `/raft/status`, `/missions`, `/klusters`, `/tasks`, `/agents`.
Everything else proxies to `--api-proxy` with full header forwarding and streaming (SSE-safe).

## Build & Test

```bash
cd integrations/mc     && cargo check -p mc
cd integrations/mc     && cargo build
cd integrations/mc     && cargo test -- --test-threads=1

cd integrations/mc-mesh && cargo check
cd integrations/mc-mesh && cargo build
cd integrations/mc-mesh && cargo test

cd integrations/mc-controlplane && cargo build
```
