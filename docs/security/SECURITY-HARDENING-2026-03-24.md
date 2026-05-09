# Security Hardening Update — 2026-03-24

Mission: `3e8e22e78ab0`
Kluster: `326cf69b71fb` (`web ui 1.0`)

## Delivered commits
- `44955e0` — harden token fallback and secret handling
- `9c904a3` — harden proxy trust and OIDC grant transport
- `92f0edc` — move web auth to cookie-backed sessions
- `2fdf88b` — signed timestamp verification for chat webhooks
- `f8d0ffd` — RFC8628 device flow + CSRF protections + security headers

## Key controls now in place
1. Session token handling
- Session tokens (`mcs_*`) are no longer embedded into agent config files.
- CLI token env help output is redacted.

2. Auth hardening
- OIDC-required mode no longer defaults to static-token fallback for `/mcp`.
- Optional fallback paths are explicit via `MC_ALLOW_TOKEN_FALLBACK_PATHS`.

3. Browser/session security
- Web auth moved off localStorage token persistence.
- OIDC exchange sets HttpOnly session cookie.
- Auth middleware accepts cookie-backed session tokens.

4. CSRF protection
- Cookie-authenticated mutating requests require CSRF token match:
  - cookie: `mc_csrf_token`
  - header: `X-CSRF-Token`

5. Security headers
- `Content-Security-Policy`
- `Referrer-Policy`
- `X-Frame-Options`
- `Permissions-Policy`

6. Webhook integrity
- Google Chat / Teams now support signed timestamp HMAC verification when signing secrets are configured.

7. Device authentication
- Added RFC8628-style device endpoints:
  - `POST /auth/oidc/device/authorize`
  - `GET /auth/oidc/device/verify`
  - `POST /auth/oidc/device/token`

## Tests executed
- `cd backend && uv run --python ../.venv/bin/python -m unittest -q tests.test_oidc_web tests.test_request_tracing`
- `cd web && npm run build`

## Repo operator note
- Added `AGENTS.md` with explicit Codex instruction to run backend tests via `uv` and parent `.venv`.
