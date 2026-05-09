# MissionControl Web UI (SvelteKit)

The front-end lives in `web/`. It is a SvelteKit 2 application with an AI-first console landing experience plus secondary dashboard tabs (matrix telemetry, explorer, onboarding, governance).

## Development

```bash
cd web
npm install
npm run dev -- --host 0.0.0.0 --port 5173
```

`npm run dev` starts the SvelteKit dev server (default port 5173). The UI uses MissionControl session tokens (`mcs_*`) stored in `localStorage`. Production sign-in uses backend OIDC browser flow (`/auth/oidc/start` -> callback -> `/auth/oidc/exchange`), while static token login remains available for testing.

## Building for production

```bash
cd web
npm run build
```

`npm run build` emits static files under `web/build`. The backend already serves `/ui/` from that build directory, so once you run this command the API automatically exposes the new UI (e.g., `http://localhost:8008/ui/`). Feel free to host the `build/` output with any static file server if you prefer.

## Features

- **AI Console (default)** — chat-first transcript + composer, natural-language planning, compact tool/event cards, and approval cards for write actions.
- **Matrix timeline** — SSE-driven feed shows approvals, inbox events, and the rate-limit status described in [`docs/reference/REAL-TIME.md`](../docs/reference/REAL-TIME.md).
- **Explorer panel** — mission/kluster tree plus detail view, leveraging `/explorer/tree` and `/explorer/node/{type}/{id}`.
- **Onboarding** — generate manifest endpoints, bootstrap commands, and config snippets for agent swarms and `mc doctor`.
- **Governance** — view active policy, inspect policy events, and refresh drafts without leaving the UI.

Theme defaults to dark mode, with a top-right moon/sun toggle.
See [`docs/reference/AI-CONSOLE.md`](../docs/reference/AI-CONSOLE.md) for event schema, planner settings, and API details.
