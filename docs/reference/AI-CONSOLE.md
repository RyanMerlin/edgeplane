# AI Console Protocol (Web UI)

MissionControl now ships an AI-first web console at `/ui/`.
The dashboard tabs remain available, but the default experience is a chat-first transcript plus command composer.

## Behavior

- Natural-language prompt entry.
- Planner maps prompts to MCP tools.
- Read tools execute immediately.
- Write tools create pending approval actions before execution.
- Events are stored for replay and audit.

## API Endpoints

- `POST /ai/sessions` create a new AI session.
- `GET /ai/sessions` list your sessions.
- `GET /ai/sessions/{session_id}` fetch turns/events/pending actions.
- `POST /ai/sessions/{session_id}/turns` submit a user turn.
- `POST /ai/sessions/{session_id}/actions/{action_id}/approve` approve and execute pending write action.
- `POST /ai/sessions/{session_id}/actions/{action_id}/reject` reject pending write action.
- `GET /ai/sessions/{session_id}/stream` optional SSE event stream.

## Dynamic View Schema (v1)

The planner may emit `view_spec` objects. The backend validates a safe declarative schema.

Allowed `type` values:

- `cards`
- `kv`
- `table`
- `timeline`
- `log_stream`
- `action_bar`

Arbitrary runtime JS/component generation is not allowed.

## Planner Configuration

Planner selection is controlled by environment variables:

- `MC_AI_PROVIDER`: `openai` | `anthropic` | unset (heuristic fallback)
- `MC_AI_MODEL`: provider model name
- `MC_AI_BASE_URL`: optional API base override (for provider-compatible gateways/proxies)
- `OPENAI_API_KEY`: required when `MC_AI_PROVIDER=openai`
- `ANTHROPIC_API_KEY`: required when `MC_AI_PROVIDER=anthropic`
- `MC_CENTRAL_RUNTIME_DEFAULT`: default runtime for AI sessions/evolve/scheduled jobs (`claude_code` recommended)
- `MC_CLAUDE_MODEL`: Anthropic model used by `claude_code` runtime
- `MC_CLAUDE_MAX_TOKENS`: max output tokens for `claude_code` runtime
- `MC_CLAUDE_TIMEOUT_SECONDS`: request timeout for `claude_code` runtime

`MC_AI_BASE_URL` examples:

- OpenAI-compatible base: `https://api.openai.com`
- OpenAI-compatible gateway: `https://my-gateway.example.com`
- Full endpoint also works: `https://my-gateway.example.com/v1/chat/completions`

If provider config is missing, MissionControl uses a local heuristic planner so the console remains usable in dev.

## Theme

Web UI is dark-mode first.
A global moon/sun toggle is available in the top-right header.
