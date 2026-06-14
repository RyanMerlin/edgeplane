---
title: Channel Integration
description: Forward external events and messages to running agents using EdgePlane's channel bridge.
---

## What Channels Are

EdgePlane channels give a running agent a real-time inbox. Instead of an agent polling for instructions, external systems push events directly to it via the MCP `notifications/claude/channel` notification. The agent receives each message as a channel notification and can optionally reply back through the same bridge.

Channels are useful any time you want an agent to react to events that originate outside its normal task queue — a human typing in Slack, a CI pipeline emitting a result, or another AI session completing a step and wanting to hand off context.

The `edgeplane channel claude` family of subcommands implements this bridge. Each subcommand exposes a different transport while speaking the same MCP channel notification protocol toward Claude.

---

## `edgeplane channel claude webhook`

### What it does

Starts a local HTTP server. Any external system that can POST JSON to a URL — Slack outgoing webhooks, GitHub webhooks, curl scripts, other services — can send messages through it. Each inbound POST is normalized into a `notifications/claude/channel` MCP notification and forwarded to Claude over stdio.

The server also exposes:
- `GET /healthz` — liveness check, returns `{"ok": true}`
- `GET /events` — SSE stream of reply events (when `--enable-reply` is active)

### Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--listen-host` | `127.0.0.1` | Interface to bind. Use `0.0.0.0` to accept connections from other machines. |
| `--listen-port` | `8788` | Port to listen on. |
| `--channel-name` | `edgeplane` | Name included in channel metadata. |
| `--instructions` | (default) | Optional system instructions sent to Claude during MCP initialization. |
| `--enable-reply` | false | Registers a `reply` MCP tool so Claude can send replies back. Replies are published on the `/events` SSE stream. |
| `--debug-protocol` | false | Log all MCP protocol frames to stderr. Useful when debugging integration behavior. |

### Inbound message format

The server accepts arbitrary JSON. It extracts the message text from the first field it finds named `text`, `content`, `message`, or `body`. A `chat_id` field at the top level is promoted into the metadata so Claude can use it when replying. Any `meta` object in the payload is also forwarded (with keys normalized to `snake_case`).

If the POST body is already a valid `notifications/claude/channel` JSON-RPC notification, it is passed through unchanged.

### Example

```bash
# Start the bridge, bound to localhost:8788
edgeplane channel claude webhook \
  --channel-name my-webhook \
  --instructions "You are monitoring CI. When a build fails, summarize the error and suggest a fix." \
  --enable-reply

# From another terminal, send a test event
curl -s -X POST http://127.0.0.1:8788/ \
  -H 'Content-Type: application/json' \
  -d '{"text": "Build failed on main: test_auth_token panicked", "chat_id": "ci-alerts"}'
```

Claude receives the message as a channel notification and can call the `reply` tool with `chat_id: "ci-alerts"` to send a response back, which appears on the `/events` SSE stream.

### Connecting to Slack

Slack's outgoing webhooks POST JSON with a `text` field — the format the webhook endpoint already handles. Point Slack's webhook URL at a publicly reachable address that proxies to your local listener (e.g., via ngrok or a Tailscale funnel):

```bash
# Expose the listener via Tailscale funnel on port 443 → local 8788
tailscale funnel --bg 8788

edgeplane channel claude webhook \
  --listen-host 127.0.0.1 \
  --listen-port 8788 \
  --channel-name slack \
  --enable-reply
```

Set the Tailscale funnel URL as your Slack app's request URL. Replies published on `/events` can be picked up by a small relay process that posts them back to Slack using the Slack API.

### Running in the background

```bash
edgeplane channel claude webhook --channel-name ci &
WEBHOOK_PID=$!

# ... Claude session runs ...

kill $WEBHOOK_PID
```

For persistent deployments, wrap the command in a systemd unit or a process supervisor alongside your agent's main process.

---

## The `--enable-reply` flag

When passed to `edgeplane channel claude webhook`, this flag adds a `reply` MCP tool to the session:

```
reply(chat_id: string, text: string) -> "ok"
```

Claude can call this tool to send a response back to the originating conversation. The reply is broadcast on the `/events` SSE stream of the webhook server. Any subscriber listening to `GET /events` will receive it as a server-sent event with name `reply`.

The flag is only meaningful for the webhook transport.

---

## Practical notes

**Text extraction:** The webhook server tries `text`, then `content`, then `message`, then `body` to find the message string in the inbound JSON. If none of those fields exist, the POST is rejected with HTTP 400. Structure your payloads accordingly.

**Meta normalization:** All keys in the `meta` object are normalized: any character that is not alphanumeric or `_` is replaced with `_`. Keys that normalize to empty strings are dropped. This prevents JSON-RPC transport issues from unusual key names.

**Pre-built notifications:** If your producer already emits valid `notifications/claude/channel` JSON-RPC objects, the webhook endpoint passes them through unmodified. This lets you drive the bridge from another EdgePlane agent or any MCP-aware producer.

**Debug mode:** Add `--debug-protocol` to see every MCP frame on stderr — both what arrives from the transport and what is written to Claude. Useful when Claude is not receiving messages as expected.

**Health check:** Before wiring up a real webhook source, verify the bridge is up with `curl http://127.0.0.1:8788/healthz`. It returns `{"ok":true}` when the server is accepting connections.
