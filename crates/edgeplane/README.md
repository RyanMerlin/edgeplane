# edgeplane

Rust-native Edgeplane CLI, daemon, and matrix bridge.

This binary (previously referred to as edgeplane-mcp-rs) is now the canonical local agent gateway: it talks
to the edgeplane-tower API, keeps a lightweight agent context so approvals and sync metadata stay
aligned, and exposes the SSE matrix feed that powers the real-time inbox/approval dashboards described
in [`docs/reference/REAL-TIME.md`](../docs/reference/REAL-TIME.md).

## Building & installing

```
cd crates/edgeplane
cargo fmt && cargo clippy
cargo test
cargo build --release
cp target/release/edgeplane /usr/local/bin/edgeplane
```

Alternatively, install via `cargo install --path crates/edgeplane` or ship the binary inside your Linux
package of choice.

## Configuration

ENV | meaning | default
----|---------|--------
`EP_BASE_URL` | base URL for Edgeplane API | `http://localhost:8008`
`EP_TOKEN` | bearer token for MCP endpoints | unset
`EP_AGENT_ID` | optional agent identity for governance/sync traces | unset
`EP_TIMEOUT_SECS` | outbound timeout for HTTP calls | `10`
`EP_ALLOW_INSECURE` | accept self-signed certs (daemon use) | `false`
`EP_SCHEMA_PACK_FILE` | optional path to a schema pack JSON to help the booster validate payloads | `docs/schema-packs/main.json`
`EP_BOOSTER_WASM` | optional path to a WASM booster module | embedded default
`EP_DISABLE_BOOSTER` | disable the WASM booster even if configured | `false`
`EP_MQTT_TOPIC` | MQTT topic for inbox updates | `edgeplane/inbox`

All command-line flags mirror these env vars and can be passed explicitly when needed.

## TUI

`edgeplane tui` launches a full-screen ratatui terminal UI for real-time fleet management.

```bash
# Auth and server come from env / ~/.ep/config.json / global flags:
EP_BASE_URL=http://localhost:8008 edgeplane tui
edgeplane --base-url http://localhost:8008 tui
edgeplane tui --mission <mission-id>   # pre-focus a specific mission
```

| Key | Tab | Description |
|-----|-----|-------------|
| `a` | Agents | Fleet nodes — status, current task, runtime |
| `m` | Missions | Missions → Klusters → Tasks drill-down (Enter to expand) |
| `f` | Feed | Live SSE event stream (`p` to pause) |
| `p` | Approvals | Pending approvals (`y` approve / `n` deny / `s` skip) |
| `s` | Secrets | Infisical folder/secret browser (read-only) |
| `c` | Config | Connection status and server info |
| Ctrl+Q | | Quit |

Secrets tab requires an active Infisical profile (`edgeplane secrets infisical add ... --activate`).

Design mockups: [`docs/tui/v3-agents.html`](../docs/tui/v3-agents.html) · [`v3-missions.html`](../docs/tui/v3-missions.html) · [`v3-feed.html`](../docs/tui/v3-feed.html)

## Command surface

```
edgeplane [--base-url URL] [--token TOKEN] [--agent-id ID] [--allow-insecure] \
   [--booster-wasm PATH] [--disable-booster] <command>
```

### Data tools
- `edgeplane data tools list` — enumerates `/mcp/tools`
- `edgeplane data tools call --tool <tool> --payload <json>` — POST `/mcp/call`

### Data sync
- `edgeplane data sync status --mission-id <id> [--kluster-id <id>] [--agent-id <id>]` — GET `/skills/sync/status`
- `edgeplane data sync promote --mission-id <id> --snapshot-id <id> --snapshot-sha256 <hash> [--kluster-id ...]` — POST `/skills/sync/ack`

### Data explorer
- `edgeplane data explorer tree` — mirrors `/explorer/tree`
- `edgeplane data explorer node --node-type <mission|kluster|task> --node-id <id>` — fetches `/explorer/node/{type}/{id}`

### Admin policy
- `edgeplane admin policy active` — `/governance/policy/active`
- `edgeplane admin policy versions [--limit N]`
- `edgeplane admin policy events [--limit N]`
- `edgeplane approvals list --mission-id <id> [--status <status>] [--limit N]`
- `edgeplane approvals create --mission-id <id> --action <action> [--reason <text>] [--request-context '{...}']`
- `edgeplane approvals approve --approval-id <id> [--expires-in-seconds N] [--note <text>]`
- `edgeplane approvals reject --approval-id <id> [--note <text>]`

### Governance automation
- `edgeplane admin governance roles list --mission-id <id> [--limit N]`
- `edgeplane admin governance roles upsert --mission-id <id> --subject <sub> --role <role>`
- `edgeplane admin governance roles remove --mission-id <id> --subject <sub>`
- `edgeplane admin governance policy active`
- `edgeplane admin governance policy versions [--limit N]`
- `edgeplane admin governance policy create-draft --file policy.json [--change-note text]`
- `edgeplane admin governance policy publish --draft-id N [--change-note text]`
- `edgeplane admin governance policy rollback --version N [--change-note text]`
- `edgeplane admin governance events [--limit N]`

### AI-native operations
- `edgeplane ops mission --action start --kluster-id <id> [--workspace-label <label>] [--agent-id <agent>] [--lease-seconds N]`
- `edgeplane ops mission --action heartbeat --lease-id <id>`
- `edgeplane ops mission --action commit --lease-id <id> --change-set '[{...}]' [--validation-mode <mode>]`
- `edgeplane ops mission --action release --lease-id <id> [--reason text]`

### Agent evolve loop
- `edgeplane agent evolve seed --spec <file>` — POST `/evolve/missions`
- `edgeplane agent evolve run --mission <id> [--agent <name>]` — POST `/evolve/missions/{id}/run`
- `edgeplane agent evolve status --mission <id>` — GET `/evolve/missions/{id}/status`

### Compatibility & drift loop
- `edgeplane system compat matrix run [--providers claude,codex] [--mode smoke|full] [--out <path>]` — runs local compatibility checks and emits `compat-report.json` artifacts under `EP_HOME/compat`.
- `edgeplane system compat matrix report-latest` — prints the latest compatibility artifact (`EP_HOME/compat/latest.json`).
- `edgeplane system drift ingest --provider <name> --version <ver> --source-url <url> --summary <text> [--severity compatible|degraded|breaking]` — records `capability-delta.json` under `EP_HOME/drift`.
- `edgeplane system drift triage [--mission <id>] [--provider <name>]` — merges latest compat+drift artifacts into a `policy-decision.json` gate decision.

### Maintenance & backups
- `edgeplane system doctor [--matrix-endpoint /events/stream] [--matrix-sample-seconds 5] [--fix]` — includes an RTK availability check; `--fix` has no effect on RTK (install it separately).
- `edgeplane system backup [--target postgres|rustfs|all] [--reason <note>]`

### Remote control
- `edgeplane agent signal <id> --content '<payload>' --remote` (or omit `--remote` to auto-resolve local-first)
- `edgeplane agent list --remote` / `edgeplane agent describe <id> --remote`

### Self-update
- `edgeplane system update self-update [--manifest-url URL]`

### Session auth
- `edgeplane auth login [--ttl-hours N] [--print-token]` — exchange current credentials for a revocable session token
- `edgeplane auth whoami` — show current identity from server (`/auth/me`)
- `edgeplane auth logout [--local-only]` — revoke current session token and clear local session file

### Doctor & daemon
- `edgeplane system doctor [--matrix-endpoint /events/stream] [--matrix-sample-seconds 5] [--fix]` — runs the health, tools, and matrix checks described in `[docs/REAL-TIME.md](../docs/reference/REAL-TIME.md)` and prints a JSON report; `--fix` ensures local directories + agent_id metadata are available for future runs.
- `edgeplane daemon --matrix-endpoint /events/stream [--fanout-port <port>] [--mqtt-url mqtt://host:1884] [--mqtt-topic edgeplane/inbox] [--shim-host 127.0.0.1] [--shim-port 8765] [--tools-cache-ttl-sec 60] [--tools-stale-sec 600] [--shim-token <token>]` — keeps an SSE stream alive for the matrix/inbox feed; fan-out and MQTT options replay the telemetry to local dashboards, and the shim API exposes local `/v1/*` control endpoints for MCP shim clients.

### Claude channel bridge
- `edgeplane channel claude webhook [--listen-host 127.0.0.1] [--listen-port 8788] [--channel-name edgeplane] [--enable-reply] [--instructions ...] [--debug-protocol]` — runs a Claude-channel MCP server over stdio, accepts inbound webhook `POST /` payloads (`text`/`content` + optional `meta`/`chat_id`) and emits `notifications/claude/channel`; optional `reply` tool writes to local SSE `GET /events` for integration testing.
- `edgeplane channel claude edgeplane --session-id <ai_session_id> [--poll-interval-ms 500] [--channel-name edgeplane] [--instructions ...] [--debug-protocol]` — bridges Edgeplane AI session SSE (`/ai/sessions/{id}/stream`) into `notifications/claude/channel` for `user_message` events. Reply tool is intentionally disabled in this mode until a non-looping outbound endpoint is added.

### Agent launch (unified)
- `edgeplane run claude [-p PROFILE] [--mission ID] [--mode interactive|headless|solo] [--with-rtk] [-- ARGS...]` — unified Claude launch with profile runtime + optional mesh participation. `--with-rtk` is a soft flag: warns and continues if [rtk](https://github.com/merlinlabs/rtk) is not installed.
- `edgeplane run codex [-p PROFILE] [--mission ID] [--mode interactive|headless|solo] [--with-rtk] [-- ARGS...]` — unified Codex launch.
- `edgeplane run gemini [-p PROFILE] [-- ARGS...]` — Gemini launch.
- `edgeplane run goose [-p PROFILE] [-- ARGS...]` — Goose launch (local models via LiteLLM).
- `edgeplane run openclaw [-p PROFILE] [-- ARGS...]` — OpenClaw launch (ACP driver agent).
- `edgeplane run custom [-p PROFILE] [-- ARGS...]` — custom ACP agent launch.

`edgeplane run <runtime>` is the single entry point for every agent. (The
legacy `edgeplane launch` command was removed; claude/codex/goose are native
runtimes, gemini/openclaw/custom are driver agents — all under `run`.)

### Runtime diagnostics
- `edgeplane run claude doctor [-p PROFILE] [--fix] [--json]` — inspect/repair Claude runtime readiness.
- `edgeplane run codex doctor [-p PROFILE] [--fix] [--json]` — inspect/repair Codex runtime readiness.
- `edgeplane run codex status [-p PROFILE] [--json]` — read-only Codex status (exits 0 even when not ready).
- `edgeplane run claude exec [-p PROFILE] [-- ARGS...]` — thin native Claude execution in prepared runtime.
- `edgeplane run codex exec [-p PROFILE] [-- ARGS...]` — thin native Codex execution in prepared runtime.

### Node service
- `curl -fsSL "$BASE_URL/runtime/nodes/$NODE_ID/install-script" | sh` bootstraps a Linux node from Edgeplane with a rendered config, join token, release artifact download, and `edgeplane-node.service` enablement.
- `crates/edgeplane/install.sh` installs `edgeplane` and the `edgeplane-node.service` unit for Linux hosts from a local checkout.
- `edgeplane node run [--node-name <name>] [--hostname <host>] [--trust-tier <tier>]` runs the resident node loop.
- `edgeplane node doctor [--node-name <name>]` inspects local node state/config before enabling the service.

The node service uses `~/.edgeplane/runtime/node-config.json` by default and accepts `EP_NODE_*` overrides from the unit environment file. Edgeplane renders the install bundle and release manifest server-side, so the node can resolve the release artifact without hardcoding an asset URL.

Required runtime settings:

- `EP_BASE_URL`
- `EP_NODE_BOOTSTRAP_TOKEN`

Common optional settings:

- `EP_NODE_NAME`
- `EP_NODE_HOSTNAME`
- `EP_NODE_TRUST_TIER`
- `EP_NODE_POLL_SECONDS`
- `EP_NODE_HEARTBEAT_SECONDS`
- `EP_NODE_UPGRADE_CHANNEL`
- `EP_NODE_DESIRED_VERSION`
- `EP_NODE_UPGRADE_MANIFEST_URL`

The backend also exposes:

- `GET /runtime/releases/latest.json` — runtime release manifest for the bootstrap flow
- `GET /runtime/releases/latest/download` — redirect to the current node release artifact
- `GET /runtime/nodes/{id}/install-bundle` — rendered config/env/service bundle
- `GET /runtime/nodes/{id}/install-script` — one-shot bootstrap script

## Real-time matrix and swarm integration

The daemon mode connects to `/events/stream` and prints the chunked telemetry that powers the inbox,
approval, and matrix dashboards. When you pair local swarm-style workflows with Edgeplane, run the
`edgeplane daemon` process alongside the swarm’s leader so that the governance plane (approvals, policy
enforcement, skill sync metadata) stays in lockstep with the agent planners and vector memory.

Run `edgeplane daemon` with `--fanout-port <port>` to expose a local SSE server on `/events` for dashboards and
local controller processes. The new [docs/REAL-TIME.md](../docs/reference/REAL-TIME.md) describes the `/events/stream` schema,
rate-limit expectations, reconnect/backoff behavior, and how the daemon should honor ticker headers so the
local fan-out does not exhaust the upstream MQ/NATS guardrails.

The WASM booster runs before every `edgeplane data tools call` (unless disabled via `--disable-booster`). It loads the
configured module (`--booster-wasm`) or the embedded default, validates the JSON payload against the schema
pack configured via `EP_SCHEMA_PACK_FILE`, and if the booster agrees, short-circuits the remote call with a
quick success message so handwritten or automated agents can avoid slow LLM loops. Pointing the env var at
`docs/schema-packs/main.json` keeps the local validation consistent with backend expectations.

The daemon also peeks at MQTT (via `--mqtt-url`/`--mqtt-topic`) and republishes those inbox messages onto the
SSE fan-out so local swarms stay synced.

## Shim API compatibility

`edgeplane daemon` now serves shim-compatible local endpoints by default on `127.0.0.1:8765`:

- `POST /v1/initialize`
- `GET /v1/tools`
- `POST /v1/call`
- `GET /v1/health` (plus `/healthz`, `/readyz`, `/livez`)

This lets MCP shim clients use the Rust daemon as their local control plane while keeping Edgeplane
API access centralized in `edgeplane`.

If `--shim-token` (or `EP_DAEMON_SHIM_TOKEN`) is set, shim requests must include either:

- `Authorization: Bearer <token>`
- `X-EP-Shim-Token: <token>`

The Rust CLI keeps scratchstate simple: tools use `serde_json` for payloads, sync/promote automates the
skill sync handshake, and the SSE stream ensures users see rapid alignment or approvals without poll
noise.

## Containerized daemon (optional)
Spin up a hardened container that runs `edgeplane daemon` with `EP_HOME` mounted, fan-out ports exposed, and
secrets injected via Compose-managed files. The default experience still runs the native binary, but the
containerized daemon is recommended for production guardrails.
