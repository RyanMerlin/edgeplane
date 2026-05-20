# mc

Rust-native Mission Control CLI, daemon, and matrix bridge.

This binary (previously referred to as mc-mcp-rs) is now the canonical local agent gateway: it talks
to the mc-controlplane API, keeps a lightweight agent context so approvals and sync metadata stay
aligned, and exposes the SSE matrix feed that powers the real-time inbox/approval dashboards described
in [`docs/reference/REAL-TIME.md`](../docs/reference/REAL-TIME.md).

## Building & installing

```
cd integrations/mc
cargo fmt && cargo clippy
cargo test
cargo build --release
cp target/release/mc /usr/local/bin/mc
```

Alternatively, install via `cargo install --path integrations/mc` or ship the binary inside your Linux
package of choice.

## Configuration

ENV | meaning | default
----|---------|--------
`MC_BASE_URL` | base URL for Mission Control API | `http://localhost:8008`
`MC_TOKEN` | bearer token for MCP endpoints | unset
`MC_AGENT_ID` | optional agent identity for governance/sync traces | unset
`MC_TIMEOUT_SECS` | outbound timeout for HTTP calls | `10`
`MC_ALLOW_INSECURE` | accept self-signed certs (daemon use) | `false`
`MC_SCHEMA_PACK_FILE` | optional path to a schema pack JSON to help the booster validate payloads | `docs/schema-packs/main.json`
`MC_BOOSTER_WASM` | optional path to a WASM booster module | embedded default
`MC_DISABLE_BOOSTER` | disable the WASM booster even if configured | `false`
`MC_MQTT_TOPIC` | MQTT topic for inbox updates | `missioncontrol/inbox`

All command-line flags mirror these env vars and can be passed explicitly when needed.

## TUI

`mc tui` launches a full-screen ratatui terminal UI for real-time fleet management.

```bash
# Auth and server come from env / ~/.mc/config.json / global flags:
MC_BASE_URL=http://localhost:8008 mc tui
mc --base-url http://localhost:8008 tui
mc tui --mission <mission-id>   # pre-focus a specific mission
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

Secrets tab requires an active Infisical profile (`mc secrets infisical add ... --activate`).

Design mockups: [`docs/tui/v3-agents.html`](../docs/tui/v3-agents.html) · [`v3-missions.html`](../docs/tui/v3-missions.html) · [`v3-feed.html`](../docs/tui/v3-feed.html)

## Command surface

```
mc [--base-url URL] [--token TOKEN] [--agent-id ID] [--allow-insecure] \
   [--booster-wasm PATH] [--disable-booster] <command>
```

### Data tools
- `mc data tools list` — enumerates `/mcp/tools`
- `mc data tools call --tool <tool> --payload <json>` — POST `/mcp/call`

### Data sync
- `mc data sync status --mission-id <id> [--kluster-id <id>] [--agent-id <id>]` — GET `/skills/sync/status`
- `mc data sync promote --mission-id <id> --snapshot-id <id> --snapshot-sha256 <hash> [--kluster-id ...]` — POST `/skills/sync/ack`

### Data explorer
- `mc data explorer tree` — mirrors `/explorer/tree`
- `mc data explorer node --node-type <mission|kluster|task> --node-id <id>` — fetches `/explorer/node/{type}/{id}`

### Admin policy
- `mc admin policy active` — `/governance/policy/active`
- `mc admin policy versions [--limit N]`
- `mc admin policy events [--limit N]`
- `mc approvals list --mission-id <id> [--status <status>] [--limit N]`
- `mc approvals create --mission-id <id> --action <action> [--reason <text>] [--request-context '{...}']`
- `mc approvals approve --approval-id <id> [--expires-in-seconds N] [--note <text>]`
- `mc approvals reject --approval-id <id> [--note <text>]`

### Governance automation
- `mc admin governance roles list --mission-id <id> [--limit N]`
- `mc admin governance roles upsert --mission-id <id> --subject <sub> --role <role>`
- `mc admin governance roles remove --mission-id <id> --subject <sub>`
- `mc admin governance policy active`
- `mc admin governance policy versions [--limit N]`
- `mc admin governance policy create-draft --file policy.json [--change-note text]`
- `mc admin governance policy publish --draft-id N [--change-note text]`
- `mc admin governance policy rollback --version N [--change-note text]`
- `mc admin governance events [--limit N]`

### AI-native operations
- `mc ops mission --action start --kluster-id <id> [--workspace-label <label>] [--agent-id <agent>] [--lease-seconds N]`
- `mc ops mission --action heartbeat --lease-id <id>`
- `mc ops mission --action commit --lease-id <id> --change-set '[{...}]' [--validation-mode <mode>]`
- `mc ops mission --action release --lease-id <id> [--reason text]`

### Agent evolve loop
- `mc agent evolve seed --spec <file>` — POST `/evolve/missions`
- `mc agent evolve run --mission <id> [--agent <name>]` — POST `/evolve/missions/{id}/run`
- `mc agent evolve status --mission <id>` — GET `/evolve/missions/{id}/status`

### Compatibility & drift loop
- `mc system compat matrix run [--providers claude,codex] [--mode smoke|full] [--out <path>]` — runs local compatibility checks and emits `compat-report.json` artifacts under `MC_HOME/compat`.
- `mc system compat matrix report-latest` — prints the latest compatibility artifact (`MC_HOME/compat/latest.json`).
- `mc system drift ingest --provider <name> --version <ver> --source-url <url> --summary <text> [--severity compatible|degraded|breaking]` — records `capability-delta.json` under `MC_HOME/drift`.
- `mc system drift triage [--mission <id>] [--provider <name>]` — merges latest compat+drift artifacts into a `policy-decision.json` gate decision.

### Maintenance & backups
- `mc system doctor [--matrix-endpoint /events/stream] [--matrix-sample-seconds 5] [--fix]` — includes an RTK availability check; `--fix` has no effect on RTK (install it separately).
- `mc system backup [--target postgres|rustfs|all] [--reason <note>]`

### Remote control
- `mc agent signal <id> --content '<payload>' --remote` (or omit `--remote` to auto-resolve local-first)
- `mc agent list --remote` / `mc agent describe <id> --remote`

### Self-update
- `mc system update self-update [--manifest-url URL]`

### Session auth
- `mc auth login [--ttl-hours N] [--print-token]` — exchange current credentials for a revocable session token
- `mc auth whoami` — show current identity from server (`/auth/me`)
- `mc auth logout [--local-only]` — revoke current session token and clear local session file

### Doctor & daemon
- `mc system doctor [--matrix-endpoint /events/stream] [--matrix-sample-seconds 5] [--fix]` — runs the health, tools, and matrix checks described in `[docs/REAL-TIME.md](../docs/reference/REAL-TIME.md)` and prints a JSON report; `--fix` ensures local directories + agent_id metadata are available for future runs.
- `mc daemon --matrix-endpoint /events/stream [--fanout-port <port>] [--mqtt-url mqtt://host:1884] [--mqtt-topic missioncontrol/inbox] [--shim-host 127.0.0.1] [--shim-port 8765] [--tools-cache-ttl-sec 60] [--tools-stale-sec 600] [--shim-token <token>]` — keeps an SSE stream alive for the matrix/inbox feed; fan-out and MQTT options replay the telemetry to local dashboards, and the shim API exposes local `/v1/*` control endpoints for MCP shim clients.

### Claude channel bridge
- `mc channel claude webhook [--listen-host 127.0.0.1] [--listen-port 8788] [--channel-name missioncontrol] [--enable-reply] [--instructions ...] [--debug-protocol]` — runs a Claude-channel MCP server over stdio, accepts inbound webhook `POST /` payloads (`text`/`content` + optional `meta`/`chat_id`) and emits `notifications/claude/channel`; optional `reply` tool writes to local SSE `GET /events` for integration testing.
- `mc channel claude missioncontrol --session-id <ai_session_id> [--poll-interval-ms 500] [--channel-name missioncontrol] [--instructions ...] [--debug-protocol]` — bridges MissionControl AI session SSE (`/ai/sessions/{id}/stream`) into `notifications/claude/channel` for `user_message` events. Reply tool is intentionally disabled in this mode until a non-looping outbound endpoint is added.

### Agent launch (unified)
- `mc run claude [-p PROFILE] [--mission ID] [--mode interactive|headless|solo] [--with-rtk] [-- ARGS...]` — unified Claude launch with profile runtime + optional mesh participation. `--with-rtk` is a soft flag: warns and continues if [rtk](https://github.com/merlinlabs/rtk) is not installed.
- `mc run codex [-p PROFILE] [--mission ID] [--mode interactive|headless|solo] [--with-rtk] [-- ARGS...]` — unified Codex launch.
- `mc run gemini [-p PROFILE] [--with-rtk] [-- ARGS...]` — unified Gemini launch.

### Runtime diagnostics
- `mc run claude doctor [-p PROFILE] [--fix] [--json]` — inspect/repair Claude runtime readiness.
- `mc run codex doctor [-p PROFILE] [--fix] [--json]` — inspect/repair Codex runtime readiness.
- `mc run codex status [-p PROFILE] [--json]` — read-only Codex status (exits 0 even when not ready).
- `mc run claude exec [-p PROFILE] [-- ARGS...]` — thin native Claude execution in prepared runtime.
- `mc run codex exec [-p PROFILE] [-- ARGS...]` — thin native Codex execution in prepared runtime.

### Node service
- `curl -fsSL "$BASE_URL/runtime/nodes/$NODE_ID/install-script" | sh` bootstraps a Linux node from MissionControl with a rendered config, join token, release artifact download, and `mc-node.service` enablement.
- `integrations/mc/install.sh` installs `mc` and the `mc-node.service` unit for Linux hosts from a local checkout.
- `mc node run [--node-name <name>] [--hostname <host>] [--trust-tier <tier>]` runs the resident node loop.
- `mc node doctor [--node-name <name>]` inspects local node state/config before enabling the service.

The node service uses `~/.missioncontrol/runtime/node-config.json` by default and accepts `MC_NODE_*` overrides from the unit environment file. MissionControl renders the install bundle and release manifest server-side, so the node can resolve the release artifact without hardcoding an asset URL.

Required runtime settings:

- `MC_BASE_URL`
- `MC_NODE_BOOTSTRAP_TOKEN`

Common optional settings:

- `MC_NODE_NAME`
- `MC_NODE_HOSTNAME`
- `MC_NODE_TRUST_TIER`
- `MC_NODE_POLL_SECONDS`
- `MC_NODE_HEARTBEAT_SECONDS`
- `MC_NODE_UPGRADE_CHANNEL`
- `MC_NODE_DESIRED_VERSION`
- `MC_NODE_UPGRADE_MANIFEST_URL`

The backend also exposes:

- `GET /runtime/releases/latest.json` — runtime release manifest for the bootstrap flow
- `GET /runtime/releases/latest/download` — redirect to the current node release artifact
- `GET /runtime/nodes/{id}/install-bundle` — rendered config/env/service bundle
- `GET /runtime/nodes/{id}/install-script` — one-shot bootstrap script

## Real-time matrix and swarm integration

The daemon mode connects to `/events/stream` and prints the chunked telemetry that powers the inbox,
approval, and matrix dashboards. When you pair local swarm-style workflows with Mission Control, run the
`mc daemon` process alongside the swarm’s leader so that the governance plane (approvals, policy
enforcement, skill sync metadata) stays in lockstep with the agent planners and vector memory.

Run `mc daemon` with `--fanout-port <port>` to expose a local SSE server on `/events` for dashboards and
local controller processes. The new [docs/REAL-TIME.md](../docs/reference/REAL-TIME.md) describes the `/events/stream` schema,
rate-limit expectations, reconnect/backoff behavior, and how the daemon should honor ticker headers so the
local fan-out does not exhaust the upstream MQ/NATS guardrails.

The WASM booster runs before every `mc data tools call` (unless disabled via `--disable-booster`). It loads the
configured module (`--booster-wasm`) or the embedded default, validates the JSON payload against the schema
pack configured via `MC_SCHEMA_PACK_FILE`, and if the booster agrees, short-circuits the remote call with a
quick success message so handwritten or automated agents can avoid slow LLM loops. Pointing the env var at
`docs/schema-packs/main.json` keeps the local validation consistent with backend expectations.

The daemon also peeks at MQTT (via `--mqtt-url`/`--mqtt-topic`) and republishes those inbox messages onto the
SSE fan-out so local swarms stay synced.

## Shim API compatibility

`mc daemon` now serves shim-compatible local endpoints by default on `127.0.0.1:8765`:

- `POST /v1/initialize`
- `GET /v1/tools`
- `POST /v1/call`
- `GET /v1/health` (plus `/healthz`, `/readyz`, `/livez`)

This lets MCP shim clients use the Rust daemon as their local control plane while keeping Mission Control
API access centralized in `mc`.

If `--shim-token` (or `MC_DAEMON_SHIM_TOKEN`) is set, shim requests must include either:

- `Authorization: Bearer <token>`
- `X-MC-Shim-Token: <token>`

The Rust CLI keeps scratchstate simple: tools use `serde_json` for payloads, sync/promote automates the
skill sync handshake, and the SSE stream ensures users see rapid alignment or approvals without poll
noise.

## Containerized daemon (optional)
Spin up a hardened container that runs `mc daemon` with `MC_HOME` mounted, fan-out ports exposed, and
secrets injected via Compose-managed files. The default experience still runs the native binary, but the
containerized daemon is recommended for production guardrails.
