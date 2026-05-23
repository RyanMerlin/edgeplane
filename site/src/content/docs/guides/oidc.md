---
title: OIDC Authentication
description: Configure SSO and OIDC JWT authentication for Edgeplane.
---

Edgeplane supports OIDC JWT validation alongside static token auth for MCP compatibility. This guide covers server configuration, CLI login flows, and Kubernetes secret management.

## Server Environment Variables

```bash
AUTH_MODE=oidc
OIDC_REQUIRED=false
EP_TOKEN=<static-token-for-mcp>
OIDC_ISSUER_URL=https://<your-idp>/application/o/<provider-slug>/
OIDC_AUDIENCE=<oidc-client-id>
OIDC_CLIENT_ID=<oidc-client-id>
OIDC_CLIENT_SECRET=<optional-for-confidential-clients>
OIDC_REDIRECT_URI=https://<edgeplane-host>/auth/oidc/callback
OIDC_SCOPES=openid profile email
EP_ADMIN_SUBJECTS=<comma-separated-subjects>
EP_ADMIN_EMAILS=<comma-separated-emails>
# optional — auto-discovered if omitted
# OIDC_JWKS_URL=https://<your-idp>/application/o/<provider-slug>/jwks/
```

## Split Internal / Public Issuer

When the API container reaches the IdP via a private address (e.g. a Kubernetes ClusterIP) but browsers must redirect to a public hostname:

```bash
# Server-side: token discovery, JWKS fetch, iss validation
OIDC_ISSUER_URL=http://<cluster-ip>/application/o/<slug>/
OIDC_INTERNAL_ISSUER_URL=http://<cluster-ip>/application/o/<slug>/

# Browser-side: authorize_url returned to CLI and web — never validated against JWT claims
OIDC_PUBLIC_ISSUER_URL=https://<public-idp-host>/application/o/<slug>/
```

**Rule:** `OIDC_ISSUER_URL` must match the `iss` claim in issued JWTs. `OIDC_PUBLIC_ISSUER_URL` only rewrites the `authorize_url` returned to browsers and CLI clients.

## CLI Login Flow

```bash
# 1. Initiate — get a browser URL and a cli_nonce
curl -s https://<edgeplane-host>/auth/oidc/cli-initiate
# → {"authorize_url": "https://...", "cli_nonce": "...", "expires_at": "..."}

# 2. Open authorize_url in your browser and complete login.
#    The success page shows a grant_id (olg_…).

# 3. Exchange the grant_id for a session token
curl -s -X POST https://<edgeplane-host>/auth/oidc/exchange \
  -H "Content-Type: application/json" \
  -d '{"grant_id": "olg_…"}'
# → {"token": "mcs_…", "subject": "…", "expires_at": "…"}

# 4. Save the session token (edgeplane reads this automatically)
cat > ~/.edgeplane/session.json <<EOF
{"token":"mcs_…","subject":"…","email":"…","expires_at":"…","base_url":"https://<edgeplane-host>","session_id":1}
EOF
chmod 600 ~/.edgeplane/session.json
```

**Polling instead of copy-paste:**

```bash
curl -s https://<edgeplane-host>/auth/oidc/cli-poll/<cli_nonce>
# → {"status":"ready","grant_id":"olg_…"}  (404 until login completes)
```

**With `edgeplane auth login`:**

```bash
export EP_BASE_URL="https://<edgeplane-host>"
# Exchange OIDC JWT for a session token
EP_TOKEN="$(get-oidc-token)" edgeplane auth login --ttl-hours 8
edgeplane auth whoami
```

## Browser Login Flow

Edgeplane uses a backend PKCE flow:

1. Browser requests `GET /auth/oidc/start`
2. Server redirects to IdP authorize endpoint with PKCE challenge
3. IdP returns to `GET /auth/oidc/callback`
4. Server exchanges auth code, validates token, issues one-time grant
5. Browser calls `POST /auth/oidc/exchange` to receive `mcs_*` session token

## Auth Mode Reference

| `AUTH_MODE` | Behavior |
|-------------|---------|
| `token` | Static bearer token only |
| `oidc` | OIDC JWT only |
| `dual` | Accept both — recommended for staged migration |

`OIDC_REQUIRED=true` in `dual` mode enforces OIDC for non-`/mcp` paths, while still allowing the static `EP_TOKEN` for MCP agent connections.

## Staged Rollout (Recommended)

For production deployments adopting OIDC on an existing instance:

1. Set `AUTH_MODE=oidc`, `OIDC_REQUIRED=false` — OIDC available but not enforced
2. Validate CLI and web login flows work end-to-end
3. Optionally switch to `AUTH_MODE=dual` for a transition period
4. Optionally set `OIDC_REQUIRED=true` when all clients have migrated
5. Later: migrate MCP clients to service-account OIDC and remove static token

## Kubernetes Secret Management

Never commit OIDC client secrets, service tokens, or static tokens to Git.

```yaml
# Create a Secret with all auth env vars
apiVersion: v1
kind: Secret
metadata:
  name: edgeplane-auth
type: Opaque
stringData:
  AUTH_MODE: "oidc"
  OIDC_ISSUER_URL: "https://..."
  OIDC_CLIENT_ID: "..."
  OIDC_CLIENT_SECRET: "..."
  EP_TOKEN: "..."
  EP_ADMIN_EMAILS: "..."
```

Mount in the deployment:

```yaml
envFrom:
- secretRef:
    name: edgeplane-auth
```

## Token Types

| Token type | Description | Recommended for |
|------------|-------------|----------------|
| Static `EP_TOKEN` | Shared secret, never expires | MCP clients, CI, local dev |
| Session token (`mcs_*`) | DB-backed, revocable, expiring | Interactive CLI/web use |
| OIDC JWT | Short-lived, identity-bound | SSO environments |

Session tokens are the recommended auth mechanism for interactive use. They are revocable, expiring, and never written to agent config files on disk.

## See Also

- [Deployment](/edgeplane/guides/deployment/) — server configuration
- [Getting Started: Agent Setup](/edgeplane/getting-started/agent-setup/) — wiring auth into agent launches
