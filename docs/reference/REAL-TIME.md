# Real-Time Matrix Telemetry

Mission Control already emits a chunked SSE feed on `/events/stream` to describe inbox events, approvals, and governance signals.
This document codifies that schema, the rate-limit semantics, and the responsibilities of the Rust `mc daemon` so swarm-style workflows can subscribe and react.

## SSE schema

Each SSE chunk is a JSON object sent via `data:` lines with optional `event` and `id` metadata. Clients should treat the payload as:

```json
{
  "type": "approval" | "inbox" | "matrix", // indicates the kind of change
  "mission_id": "...",
  "kluster_id": "...",
  "agent_id": "...",
  "status": "pending" | "approved" | "rejected",
  "payload": { ... },        // arbitrary tool-specific context
  "rate_limit": {
    "limit": 60,            // max events per minute
    "remaining": 42,        // remaining allowance
    "reset_at": "2026-03-10T15:42:00Z"
  }
}
```

The optional `rate_limit` block mirrors upstream MQ/NATS guardrails so local daemons can back off when the server pauses. The `mc daemon` already prints the payload to stdout; it should also surface `remaining` + `reset_at` as part of diagnostics so dashboards can tell when the stream is throttling and when it will recover, and pause fan-out retransmission until the reset window reopens.

## Backoff & resilience

- Keep a short-lived backoff (start at 1s, double up to ~30s) whenever the `/events/stream` SSE connection drops; log the adapter errors, then reconnect automatically.
- When reconnecting, preserve the latest event `id` (if provided) so the server can resume the stream without gaps.
- Mirror the SSE feed to an optional websocket endpoint (`/events/ws`) or local SSE fan-out so dashboards never miss a chunk even if the FastAPI stream hiccups, and record reconnect timestamps inside `mc system doctor` so you can surface whether rate-limit throttling is the root cause.
- Track stream health (error rates, reconnects) and expose them via the local `mc system doctor` output or logs so dashboards can show whether the daemon is healthy.

- `mc system doctor` now probes the health, tools, and matrix endpoints together and emits a structured JSON report with repair hints; `mc system doctor --fix` also ensures `MC_HOME`/`MC_SKILLS_HOME` exist and seeds a stable `agent_id` file so local swarms and ruler daemons stay identifiable.

## Fan-out

- `mc daemon` can optionally start a local SSE/WebSocket fan-out server (default bound to `localhost`) that replays every structured event.
- Each local client (CLI, UI panel, local controller) opens `/events` and receives the same stream, eliminating expensive polling loops.
- Fan-out should respect the upstream rate limit by pausing retransmission when `remaining` hits zero and resuming after `reset_at` (or a small buffer) to avoid hitting the server’s throttles again.

The local fan-out can present the same stream over SSE or WebSocket (with text frames). Dashboards should send a `X-Client-ID` header when connecting and honor the daemon’s pause signals—no retransmissions while `remaining` is zero—and resume only after the upstream `reset_at` timestamp plus a small cushion. When the fan-out notices that upstream disabled the matrix feed (status ≠ 200), it should notify clients via a `event: matrix-down` chunk so the UI can show a reconnection banner.

The daemon also listens to MQTT topics (`missioncontrol/inbox` by default) and re-emits those messages via the same fan-out so planners running local swarms can treat MQTT inboxes as another matrix feed without introducing extra listeners.

If an operator needs faster-than-LLM loops, supply `--booster-wasm` with a Wasm module that implements `validate(ptr,len)`; the embedded default simply ensures the payload is non-empty. The module runs before every MCP tool call and (when enabled) can short-circuit the HTTP request with instant success while still emitting the structured telemetry the matrix front ends expect.

## Schema pack + booster

`mc` draws the same schema pack that the backend enforces so the booster and matrix instrumentation share a single source of truth. Set `MC_SCHEMA_PACK_FILE` to `docs/schema-packs/main.json` (or your own custom drop-in) and the daemon will validate `mission`, `kluster`, `task`, `doc`, and `artifact` payloads before invoking `/mcp/call`. Invalid schema packs are logged at startup and fall back to the embedded defaults described in [`docs/MC-RUST.md`](MC-RUST.md); the matrix FE can then trust the `type`/`payload` shape without hitting the server randomly.

## Operational guidance

- Document how to launch the daemon alongside swarm-style workflows: run `mc daemon --matrix-endpoint /events/stream --fanout-port 11234` on the planner host so scrapers or dashboards can read the local SSE feed.
- Mention TLS/rate-limit prerequisites (the daemon may need `MC_ALLOW_INSECURE` for dev proxies), document `MC_SCHEMA_PACK_FILE` so boosters share the same schema pack, and remind operators to keep `MC_TOKEN` or OIDC tokens rotate-ready.
- Tie this doc back to the `mc` companion guide at [docs/MC-RUST.md](MC-RUST.md).
