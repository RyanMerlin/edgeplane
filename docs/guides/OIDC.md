# Edgeplane OIDC (Authentik)

This repo supports OIDC JWT validation in the Edgeplane API while keeping token auth for MCP compatibility.

Production requirement: manage secret values through Kubernetes Secrets (no inline literals, no checked-in `.env`).

## Edgeplane API env

Set these on the `edgeplane-api` deployment:

```env
AUTH_MODE=oidc
OIDC_REQUIRED=false
OIDC_ISSUER_URL=https://<authentik-host>/application/o/<provider-slug>/
OIDC_AUDIENCE=<oidc-client-id>
OIDC_CLIENT_ID=<oidc-client-id>
OIDC_CLIENT_SECRET=<optional-for-confidential-clients>
OIDC_REDIRECT_URI=https://<edgeplane-host>/auth/oidc/callback
OIDC_SCOPES=openid profile email
EP_ADMIN_SUBJECTS=<comma-separated-subjects>
EP_ADMIN_EMAILS=<comma-separated-emails>
# optional
# OIDC_JWKS_URL=https://<authentik-host>/application/o/<provider-slug>/jwks/
```

### Split internal/public issuer (dev/cluster)

When the API container can only reach the IdP via a private address (e.g. a
Kubernetes ClusterIP) but browsers must redirect to a public hostname, set two
extra vars:

```env
# Server-side: used for token discovery + JWKS fetch + iss validation
OIDC_ISSUER_URL=http://<cluster-ip>/application/o/<slug>/
OIDC_INTERNAL_ISSUER_URL=http://<cluster-ip>/application/o/<slug>/

# Browser-side: authorize_url in CLI initiate / web login is rewritten to this origin
OIDC_PUBLIC_ISSUER_URL=https://<public-authentik-host>/application/o/<slug>/
```

Rule: `OIDC_ISSUER_URL` must match the `iss` claim in issued JWTs (set to the
ClusterIP URL when Authentik is inside the cluster). `OIDC_PUBLIC_ISSUER_URL`
is only used to rewrite the `authorize_url` returned to browsers/CLI — it is
never validated against JWT claims.

## CLI login flow

```bash
# 1. Initiate — get a browser URL and a cli_nonce
curl -s http://<edgeplane-host>/auth/oidc/cli-initiate
# → {"authorize_url": "https://...", "cli_nonce": "...", "expires_at": "..."}

# 2. Open authorize_url in your browser and complete login.
#    The success page shows a grant_id (olg_…).

# 3. Exchange the grant_id for a session token
curl -s -X POST http://<edgeplane-host>/auth/oidc/exchange \
  -H "Content-Type: application/json" \
  -d '{"grant_id": "olg_…"}'
# → {"token": "mcs_…", "subject": "…", "expires_at": "…"}

# 4. Write token to ~/.edgeplane/session.json (edgeplane reads this automatically)
# session.json format:
# {"token":"mcs_…","subject":"…","email":"…","expires_at":"…","base_url":"http://<edgeplane-host>","session_id":1}
```

Alternatively, poll instead of copy-paste:
```bash
curl -s http://<edgeplane-host>/auth/oidc/cli-poll/<cli_nonce>
# → {"status":"ready","grant_id":"olg_…"}  (404 until login completes)
```

## Browser login flow (production)

Edgeplane web login uses backend PKCE flow:

1. Browser sends user to `GET /auth/oidc/start`.
2. Edgeplane redirects to IdP authorize endpoint with PKCE challenge.
3. IdP returns to `GET /auth/oidc/callback`.
4. Edgeplane exchanges auth code, validates token, and issues one-time grant.
5. Browser calls `POST /auth/oidc/exchange` to receive `mcs_*` session token.

The web UI should treat OIDC as primary and static token login as testing fallback.

Modes:
- `AUTH_MODE=token`: static bearer token only.
- `AUTH_MODE=oidc`: OIDC JWT only.
- `AUTH_MODE=dual`: accept token and OIDC.

`OIDC_REQUIRED=true` in dual mode enforces OIDC for non-`/mcp` paths.
If `AUTH_MODE` is unset, runtime defaults to OIDC when OIDC vars are present.

## Kubernetes secret guidance

- Source all auth settings from Kubernetes Secrets.
- Do not commit client secrets or service-account tokens.
- Mount/inject only secret refs in manifests (`envFrom.secretRef` / `env.valueFrom.secretKeyRef`).
- Roll out with:
  1. `AUTH_MODE=oidc`, `OIDC_REQUIRED=false`
  2. validate Edgeplane user flows
  3. optionally use `AUTH_MODE=dual` for staged MCP migration
  4. optionally set `OIDC_REQUIRED=true`
  5. later migrate MCP to service-account OIDC and remove static token
