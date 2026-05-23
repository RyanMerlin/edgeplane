---
title: AI Console
description: Edgeplane's chat-first web UI — sessions, approval flows, and planner configuration.
---

Edgeplane ships an AI-first web console at `/ui/`. The default experience is a chat-style transcript with a command composer. Dashboard tabs (missions, agents, approvals) remain available.

## Behavior

- Natural-language prompt entry
- Planner maps prompts to MCP tools
- **Read tools** execute immediately
- **Write tools** create pending approval actions before execution — no mutations happen without explicit approval
- Events are stored for replay and audit

## API

### Session Management

```
POST /ai/sessions                          # create a new AI session
GET  /ai/sessions                          # list your sessions
GET  /ai/sessions/{session_id}             # fetch turns, events, pending actions
```

### Turn Submission

```
POST /ai/sessions/{session_id}/turns       # submit a user turn
```

Body:

```json
{
  "content": "list all missions in the engineering namespace"
}
```

### Approval Flow

Write operations create pending actions that require explicit approval:

```
POST /ai/sessions/{session_id}/actions/{action_id}/approve   # execute
POST /ai/sessions/{session_id}/actions/{action_id}/reject    # discard
```

### Event Stream

```
GET /ai/sessions/{session_id}/stream       # SSE event stream for this session
```

## Dynamic View Schema

The planner can emit `view_spec` objects for structured visual output. The backend validates these against a safe declarative schema — arbitrary runtime JavaScript is not allowed.

Allowed `type` values:

| Type | Description |
|------|-------------|
| `cards` | Card grid layout |
| `kv` | Key-value pairs |
| `table` | Tabular data |
| `timeline` | Chronological event list |
| `log_stream` | Scrolling log output |
| `action_bar` | Inline action buttons |

## Planner Configuration

The planner is the component that maps natural-language prompts to MCP tool calls. Configure via environment variables on the `edgeplane-tower` server:

| Variable | Values / Description |
|----------|---------------------|
| `MC_AI_PROVIDER` | `openai` \| `anthropic` \| unset (heuristic fallback) |
| `MC_AI_MODEL` | Provider model name (e.g. `claude-opus-4-5`, `gpt-4.1`) |
| `MC_AI_BASE_URL` | Optional API base override — works with OpenAI-compatible gateways |
| `OPENAI_API_KEY` | Required when `MC_AI_PROVIDER=openai` |
| `ANTHROPIC_API_KEY` | Required when `MC_AI_PROVIDER=anthropic` |
| `MC_CENTRAL_RUNTIME_DEFAULT` | Default runtime for AI sessions (`claude_code` recommended) |
| `MC_CLAUDE_MODEL` | Anthropic model for `claude_code` runtime |
| `MC_CLAUDE_MAX_TOKENS` | Max output tokens for `claude_code` runtime |
| `MC_CLAUDE_TIMEOUT_SECONDS` | Request timeout for `claude_code` runtime |

`MC_AI_BASE_URL` examples:

- Standard: `https://api.openai.com`
- OpenAI-compatible gateway: `https://my-gateway.example.com`
- Full endpoint: `https://my-gateway.example.com/v1/chat/completions`

If no provider config is set, Edgeplane uses a local heuristic planner — the console remains usable in dev without an API key.

## Theme

The web UI is dark-mode first. A light/dark toggle is available in the top-right header.

## See Also

- [Reference: Real-Time Events](/edgeplane/reference/real-time/) — SSE event stream
- [Concepts: Philosophy](/edgeplane/concepts/philosophy/) — why writes require approval
