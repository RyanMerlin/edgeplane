---
title: Security Model
description: Authentication, authorization, audit trail, and trust boundaries in EdgePlane.
---

EdgePlane uses three distinct authentication modes depending on who (or what) is calling the API. All write operations are recorded in an immutable ledger.

---

## Authentication Modes

| Mode | Identity | Issued by | Stored at |
|------|----------|-----------|-----------|
| **OIDC session** | Human operator (browser) | IdP → edgeplane-tower | `~/.edgeplane/session.json`, `ep_session_token` cookie |
| **Node JWT** (RS256) | Machine / daemon | `edgeplaned register` via tower | `/etc/edgeplane/node.json` |
| **Service account** | CI / scripted automation | Tower API (`mcs_sa_*` prefix) | Caller-managed |

> **Note:** The static shared-secret `EP_TOKEN` was removed in v0.11.0. Any deployment still using it must migrate to one of the three modes above before upgrading.

---

## OIDC Flow (Human Operators)

```
edgeplane auth login
  │
  ▼
Browser opens https://your-tower-host/api/auth/oidc/login
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
Tower issues opaque session token (mcs_* prefix)
  │  Sets HttpOnly cookie (ep_session_token) for browser requests
  │  Also returned in response body for CLI storage
  ▼
~/.edgeplane/session.json written (mode 600)
```

The CLI uses the stored token as a Bearer header. The web dashboard uses the cookie. Both paths go through the same middleware in edgeplane-tower.

Session tokens can be refreshed via `edgeplane auth refresh` before they expire. Token TTL is configurable on the tower.

---

## Node JWT (Machine-to-Machine)

edgeplaned registers a node using a short-lived join token:

```bash
# Admin creates a join token (expires in 10 minutes)
edgeplane agent node join-token create --ttl 600

# On the target machine
edgeplaned register --join-token <TOKEN> --endpoint https://edgeplane.example.com
```

Tower issues an RS256-signed JWT and writes it to `/etc/edgeplane/node.json` (root-readable only). edgeplaned reads this file at startup — no environment variable injection is required. JTI-based revocation is tracked in the `nodetoken` table; compromised node credentials can be revoked without rotating the signing key.

Token rotation: `edgeplane agent node join-token rotate`.

---

## Service Account Tokens (Scripted / CI)

For non-interactive callers (CI pipelines, automation scripts):

```bash
# Create a service account token
edgeplane auth service-accounts create --name ci-pipeline
```

Tokens have the `mcs_sa_` prefix. They are validated against the `serviceaccount` + `serviceaccounttoken` tables in Postgres. Revocation is immediate: `edgeplane auth service-accounts delete <id>`.

---

## Authorization Model

Once authenticated, tower enforces authorization at three layers:

1. **Session scope** — sessions belong to an owner. WebSocket attach is owner-scoped: a user can only attach to agents they registered or have been explicitly granted access to via domain membership.

2. **Domain membership** — task and artifact writes require the caller to be a member of the domain the mission belongs to. Membership is explicit and manageable via `edgeplane domain members`.

3. **Governance policies** — specific operations (publishing mutations, creating certain entity types) can be gated by governance policies. When a policy requires human approval, the mutation enters the ledger as `pending` and a notification goes out. No data moves until the approval is granted.

---

## Governance Approvals

When a governance policy requires approval before a mutation proceeds:

1. Mutation is written to the ledger with `status = pending`.
2. An `ApprovalRequest` record is created; an SSE event notifies subscribers.
3. Tower generates an HMAC-SHA256 signed approval token (`base64url(payload).base64url(hmac_sig)`), returned to the caller and included in the notification payload.
4. An approver acts via the web dashboard (`POST /api/approvals/{id}/approve` or `/reject`) or CLI.
5. On approval: the mutation is promoted and the original operation proceeds. The ledger entry is updated with `approved_by` and `approved_at`.
6. On rejection: the mutation is abandoned; the ledger records the rejection.

The approval token can also be used for programmatic approval flows — sign the payload with the shared HMAC key to prove identity without a full session.

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
