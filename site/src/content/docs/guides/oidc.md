---
title: OIDC Authentication
description: Configure SSO and OIDC JWT authentication for EdgePlane.
---

EdgePlane uses OIDC for human operator authentication. This guide covers server configuration, CLI login flows, and Kubernetes secret management.

## Server Environment Variables

```bash
OIDC_ISSUER_URL=https://<your-idp>/application/o/<provider-slug>/
OIDC_CLIENT_ID=<oidc-client-id>
OIDC_CLIENT_SECRET=<oidc-client-secret>
OIDC_REDIRECT_URI=https://<edgeplane-host>/api/auth/oidc/callback
OIDC_SCOPES=openid profile email
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
curl -s https://<edgeplane-host>/api/auth/oidc/cli-initiate
# → {"authorize_url": "https://...", "cli_nonce": "...", "expires_at": "..."}

# 2. Open authorize_url in your browser and complete login.
#    The success page shows a grant_id (olg_…).

# 3. Exchange the grant_id for a session token
curl -s -X POST https://<edgeplane-host>/api/auth/oidc/exchange \
  -H "Content-Type: application/json" \
  -d '{"grant_id": "olg_…"}'
# → {"token": "ep_…", "subject": "…", "expires_at": "…"}

# 4. Save the session token (edgeplane reads this automatically)
cat > ~/.edgeplane/session.json <<EOF
{"token":"ep_…","subject":"…","email":"…","expires_at":"…","base_url":"https://<edgeplane-host>","session_id":1}
EOF
chmod 600 ~/.edgeplane/session.json
```

**Polling instead of copy-paste:**

```bash
curl -s https://<edgeplane-host>/api/auth/oidc/cli-poll/<cli_nonce>
# → {"status":"ready","grant_id":"olg_…"}  (404 until login completes)
```

**With `edgeplane auth login`:**

```bash
export EP_BASE_URL="https://<edgeplane-host>"
edgeplane auth login --ttl-hours 8
edgeplane auth whoami
```

## Browser Login Flow

EdgePlane uses a backend PKCE flow:

1. Browser requests `GET /api/auth/oidc/start`
2. Server redirects to IdP authorize endpoint with PKCE challenge
3. IdP returns to `GET /api/auth/oidc/callback`
4. Server exchanges auth code, validates token, issues one-time grant
5. Browser calls `POST /api/auth/oidc/exchange` to receive `ep_*` session token

## Auth Paths

EdgePlane has three authentication paths — no static API token (`EP_TOKEN` was removed in v0.11.0):

| Path | Who | How |
|------|-----|-----|
| **OIDC session** | Human operators | `edgeplane auth login` → browser flow → `ep_*` session token |
| **Service account** | CI / scripted | Create via API → `ep_sa_*` token → pass via `EP_AGENT_TOKEN` |
| **Node JWT** | `edgeplaned` daemon | `edgeplane agent node register` → RS256 JWT at `/etc/edgeplane/node.json` |

When `OIDC_ISSUER_URL` and `OIDC_CLIENT_ID` are set, the server enables OIDC login automatically.

## Production Setup

1. Set `OIDC_ISSUER_URL`, `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` in the server environment
2. Configure `OIDC_REDIRECT_URI` to `https://<edgeplane-host>/api/auth/oidc/callback` in your IdP
3. Validate the CLI login flow: `edgeplane auth login` → browser → `edgeplane auth whoami`
4. Validate the web dashboard login (browser → tower root → IdP → back to dashboard)

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
  OIDC_ISSUER_URL: "https://..."
  OIDC_CLIENT_ID: "..."
  OIDC_CLIENT_SECRET: "..."
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
| Session token (`ep_*`) | DB-backed, revocable, expiring | Interactive CLI/web use |
| Service account (`ep_sa_*`) | Long-lived, programmatic | MCP clients, CI pipelines |
| Node JWT | Per-node RS256 JWT | `edgeplaned` daemon, machine-to-machine |
| OIDC JWT | Short-lived, identity-bound | SSO environments (exchanged for session token) |

Session tokens are the recommended auth mechanism for interactive use. They are revocable, expiring, and never written to agent config files on disk.

## See Also

- [Deployment](/guides/deployment/) — server configuration
- [Getting Started: Agent Setup](/getting-started/agent-setup/) — wiring auth into agent launches
