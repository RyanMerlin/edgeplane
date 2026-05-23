---
title: Real-Time Events
description: SSE event stream, fan-out, schema, and rate-limit semantics for MissionControl telemetry.
---

MissionControl emits a chunked Server-Sent Events (SSE) feed on `/events/stream` describing inbox events, approvals, governance signals, and mission state changes.

## SSE Endpoint

```
GET /events/stream
Authorization: Bearer <token>
```

Each SSE chunk is a JSON object sent via `data:` lines:

```json
{
  "type": "approval" | "inbox" | "matrix",
  "domain_id": "...",
  "mission_id": "...",
  "agent_id": "...",
  "status": "pending" | "approved" | "rejected",
  "payload": { },
  "rate_limit": {
    "limit": 60,
    "remaining": 42,
    "reset_at": "2026-03-10T15:42:00Z"
  }
}
```

The optional `rate_limit` block lets local consumers back off when the server is throttling.

## Event Types

| `type` | Description |
|--------|-------------|
| `approval` | A governance approval request or resolution |
| `inbox` | Agent inbox message |
| `matrix` | Mission state change or telemetry event |

## Backoff and Resilience

Recommended client behavior:

- Start with 1s backoff on disconnect, double up to ~30s
- On reconnect, send the last known `id` to resume without gaps
- Pause fan-out retransmission while `rate_limit.remaining == 0`; resume after `reset_at` plus a small buffer
- Log reconnect timestamps for observability (`mc system doctor` surfaces rate-limit throttling)

## Local Fan-Out

`mc daemon` can run a local SSE/WebSocket fan-out server (default: `localhost`) that replays every structured event to local consumers:

```bash
mc daemon --matrix-endpoint /events/stream --fanout-port 11234
```

Local clients (CLI panels, dashboards, local controllers) connect to the fan-out at `/events` and receive the same stream without expensive polling.

The fan-out:
- Respects the upstream rate limit — pauses retransmission when `remaining` hits zero
- Emits `event: matrix-down` when upstream returns non-200, so UIs can show a reconnection banner
- Optionally accepts MQTT topics and re-emits them on the same stream

## WebSocket Endpoint

An optional WebSocket mirror is available at `/events/ws` for clients that prefer WebSocket over SSE.

## Schema Pack Validation

`mc` validates payloads against the same schema pack the backend enforces:

```bash
export MC_SCHEMA_PACK_FILE=docs/schema-packs/main.json
```

With `MC_SCHEMA_PACK_FILE` set, the daemon validates `domain`, `mission`, `task`, `doc`, and `artifact` payloads before invoking `/mcp/call`. Invalid packs fall back to embedded defaults at startup.

## Planner Booster (Advanced)

For faster-than-LLM validation loops, supply a Wasm module with `--booster-wasm`:

```bash
mc daemon --booster-wasm /path/to/validate.wasm
```

The module implements `validate(ptr, len)` and runs before every MCP tool call. It can short-circuit the HTTP request with instant success while still emitting structured telemetry. The embedded default validates that payloads are non-empty.

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `MC_ALLOW_INSECURE` | Allow HTTP (no TLS) for dev proxy setups |
| `MC_SCHEMA_PACK_FILE` | Path to schema pack file for booster validation |

## Operational Notes

- Keep `MC_TOKEN` or OIDC session tokens rotation-ready — the SSE stream authenticates per-connection
- `mc system doctor` probes health, tools, and matrix endpoints and emits a structured JSON report with repair hints
- `mc system doctor --fix` ensures `MC_HOME`/`MC_SKILLS_HOME` exist and seeds a stable `agent_id` for local swarms

## See Also

- [Reference: AI Console](/missioncontrol/reference/ai-console/) — session-based AI interaction via the web UI
- [Reference: CLI](/missioncontrol/reference/cli/) — `mc daemon` and `mc system doctor` commands
