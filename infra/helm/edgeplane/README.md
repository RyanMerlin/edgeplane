# Edgeplane Helm deployment

This chart deploys the `edgeplane-tower` API server. The tower auto-migrates its
Postgres schema on startup and serves the bundled web dashboard.

## Configuration

- `config.<KEY>` entries render into a `ConfigMap` and are injected as
  environment variables. Use it for **non-secret** settings (base URL, Postgres
  host/user, object-storage endpoint, OIDC issuer/client-id, admin emails).
- `secrets.<KEY>` entries render into an `Opaque` Secret and are injected the
  same way. Use it for **every credential** — never place a credential in
  `config`. For production, prefer referencing an external secret store
  (Infisical, External Secrets Operator, Vault) over inline values.

### Required and recommended values

| Key | Where | Notes |
|-----|-------|-------|
| `POSTGRES_PASSWORD` | `secrets` | Required. |
| `EP_OBJECT_STORAGE_ACCESS_KEY` / `_SECRET` | `secrets` | Required for artifact storage. |
| `EP_JWT_SIGNING_KEY` | `secrets` | Strongly recommended. Base64 RSA PKCS#8 PEM. **If unset the tower mints an ephemeral keypair on every start, so each restart invalidates every node's JWT.** Generate once: `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -outform PEM \| base64 -w0`. |
| `OIDC_ISSUER_URL` / `OIDC_CLIENT_ID` / `OIDC_REDIRECT_URI` | `config` | Required for web login. |
| `OIDC_CLIENT_SECRET` | `secrets` | Required for web login. |
| `EP_ADMIN_EMAILS` / `EP_ADMIN_GROUPS` | `config` | At least one is needed for anyone to hold admin. |

`EP_TOKEN` / `EP_AGENT_TOKEN` are **not** used by the tower — authentication is
OIDC (users) plus node JWTs (daemons). Do not pass them.

## Install

```bash
helm upgrade --install edgeplane infra/helm/edgeplane \
  --values infra/helm/edgeplane/values.yaml \
  --set config.POSTGRES_HOST="${POSTGRES_HOST}" \
  --set-string secrets.POSTGRES_PASSWORD="${POSTGRES_PASSWORD}" \
  --set-string secrets.EP_JWT_SIGNING_KEY="${EP_JWT_SIGNING_KEY}"
```

Prefer wiring `secrets.*` from your secret store rather than `--set-string` on
the command line, which leaks values into shell history and process listings.

## Health probes

The tower serves liveness at `/healthz` (process up, no I/O) and readiness at
`/readyz` (gated on a live database round-trip). The chart wires both; tune
timing under `probes` in `values.yaml`.
