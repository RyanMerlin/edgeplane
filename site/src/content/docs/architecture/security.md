---
title: Security Model
description: Authentication, authorization, audit trail, and trust boundaries in EdgePlane.
---

EdgePlane uses three distinct authentication modes depending on who (or what) is calling the API. All write operations are recorded in an immutable ledger.

---

## Authentication Modes

| Mode | Identity | Issued by | Stored at |
|------|----------|-----------|-----------|
| **OIDC session** | Human operator (browser) | IdP → edgeplane-tower | `~/.edgeplane/session.json`, `ep_session_token` cookie (token prefix: `ep_`, not `ep_sa_`) |
| **Node JWT** (RS256) | Machine / daemon | `edgeplane agent node register` via tower | `/etc/edgeplane/node.json` |
| **Service account** | CI / scripted automation | Tower API (`ep_sa_*` prefix) | Caller-managed; pass via `EP_AGENT_TOKEN` env var |

> **Note:** The static shared-secret `EP_TOKEN` was removed in v0.11.0. Any deployment still using it must migrate to one of the three modes above before upgrading.

![Authentication flows](/diagrams/auth-flows.svg)

---

## OIDC Flow (Human Operators)

```
edgeplane auth login
  │
  ▼
Browser opens https://your-tower-host/api/auth/oidc/start
  │  Redirect to IdP (Authentik, Okta, etc.)
  ▼
IdP issues authorization code → user authenticates
  │
  ▼
Browser redirected to https://your-tower-host/api/auth/oidc/callback?code=...
  │  Tower exchanges code for id_token + userinfo
  │  Claims verified via provider's userinfo endpoint (not unverified JWT parsing)
  │  `preferred_username` captured (falls back to `name`)
  ▼
Tower issues opaque session token (`ep_` prefix — not `ep_sa_`; service-account tokens use `ep_sa_`)
  │  Sets HttpOnly cookie (ep_session_token) for browser requests
  │  Also returned in response body for CLI storage
  ▼
~/.edgeplane/session.json written (mode 600)
```

The CLI uses the stored token as a Bearer header. The web dashboard uses the cookie. Both paths go through the same middleware in edgeplane-tower.

Token TTL is configurable on the tower; re-run `edgeplane auth login` to get a new session when needed.

---

## Node JWT (Machine-to-Machine)

edgeplaned registers a node using a short-lived join token:

```bash
# Admin creates a join token (expires in 10 minutes)
edgeplane agent node join-token create --ttl-seconds 600

# On the target machine — point EP_BASE_URL at the tower first
export EP_BASE_URL=https://edgeplane.example.com
edgeplane agent node register --hostname <node-hostname>
```

Tower issues an RS256-signed JWT and writes it to `/etc/edgeplane/node.json` (root-readable only). edgeplaned reads this file at startup — no environment variable injection is required. JTI-based revocation is tracked in the `nodetoken` table; compromised node credentials can be revoked without rotating the signing key.

Token rotation: `edgeplane agent node join-token rotate`.

---

## Service Account Tokens (Scripted / CI)

For non-interactive callers (CI pipelines, automation scripts):

Service account tokens are created via the tower API:

```bash
# Create via API
curl -X POST https://your-tower-host/api/auth/service-accounts \
  -H "Authorization: Bearer $SESSION_TOKEN" \
  -d '{"name": "ci-pipeline"}'
```

Tokens carry the `ep_sa_` prefix. They are validated against the `serviceaccount` + `serviceaccounttoken` tables in Postgres. Pass them to agent processes via the `EP_AGENT_TOKEN` environment variable. Revocation is immediate via the API or web dashboard.

---

## Authorization Model

Once authenticated, tower enforces authorization at three layers:

1. **Session scope** — sessions belong to an owner. WebSocket attach is owner-scoped: a user can only attach to agents they registered or have been explicitly granted access to via domain membership.

2. **Domain ownership** — task and artifact writes require the caller to appear in the domain's `owners` or `contributors` column. Admins (subjects listed in `EP_ADMIN_EMAILS`) bypass this check.

---

## Audit Trail

Every mutation that flows through edgeplane-tower produces an immutable ledger entry:

- **Who:** authenticated subject (user, node, service account)
- **What:** operation type + entity reference
- **When:** UTC timestamp
- **Outcome:** immediate / pending-approval / rejected

Artifact provenance records additionally include:
- `agent_id` and `session_id` of the creating agent
- SHA-256 content hash (computed server-side on upload, stored alongside the artifact record)
- S3 URI and storage backend

Ledger events are queryable via `get_entity_history` MCP tool and via `edgeplane task history` / `edgeplane mission history` CLI subcommands.

---

## Network Security

- **TLS termination** at your reverse proxy (nginx, Caddy, Cloudflare). edgeplane-tower speaks plain HTTP internally — do not expose it directly to the public internet without TLS.
- **edgeplaned management socket** (`mgmt.sock`) is a Unix domain socket — not network-exposed. Only processes running as the same user can connect.
- **Secrets broker socket** (`secrets.sock`) is a Unix domain socket accessible only to agent subprocesses spawned by edgeplaned. Agents receive `EP_SECRETS_SOCKET` and `EP_SECRETS_SESSION` at launch and cannot access secrets outside their session scope.
- **Node JWT private key** is stored in Postgres (encrypted at rest by your database configuration) and never leaves tower. edgeplaned holds only the signed JWT, not the signing key.

---

## What's Next

- [Guides: OIDC Setup](/guides/oidc/) — configure an IdP and wire up the callback
- [Guides: Deployment](/guides/deployment/) — production deployment with TLS and a reverse proxy
