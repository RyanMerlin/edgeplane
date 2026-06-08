---
title: Deployment
description: Deploy EdgePlane on a Linux VM, with Docker Compose, or on Kubernetes.
---

This guide covers three deployment paths: Linux VM with systemd, Docker Compose, and Kubernetes.

## Prerequisites

- PostgreSQL 14+
- S3-compatible object storage (RustFS (bundled), MinIO, AWS S3, or other S3-compatible storage)
- `edgeplane-tower` binary (see [Installation](/getting-started/installation/))

## Linux VM / systemd

### 1. Place the binary

```bash
cp target/release/edgeplane-tower /usr/local/bin/edgeplane-tower
```

### 2. Environment file

Create `/etc/edgeplane/env`:

```bash
# Database
DATABASE_URL=postgresql://edgeplane:password@localhost/edgeplane

# OIDC authentication (required)
OIDC_ISSUER_URL=https://<your-idp-host>/application/o/<provider-slug>/
OIDC_CLIENT_ID=<oidc-client-id>
OIDC_CLIENT_SECRET=<oidc-client-secret>

# JWT signing key for node identity tokens (generate a persistent key for production)
# If unset, an ephemeral key is generated on each restart — node JWTs will be invalidated on restart
# Generate: openssl genrsa 2048 | openssl pkcs8 -topk8 -nocrypt | base64 -w 0
EP_JWT_SIGNING_KEY=<base64-encoded-rsa-pkcs8-pem>

# Governance approval HMAC token signing secret (required for governance features)
EP_APPROVAL_TOKEN_SECRET=<random-secret>

# S3-compatible object storage (optional — artifact/doc content storage)
EP_OBJECT_STORAGE_ENDPOINT=http://<s3-host>:<port>
EP_OBJECT_STORAGE_REGION=us-east-1
EP_OBJECT_STORAGE_BUCKET=edgeplane
EP_OBJECT_STORAGE_ACCESS_KEY=<access-key>
EP_OBJECT_STORAGE_ACCESS_SECRET=<secret>
```

### 3. systemd service

Create `/etc/systemd/system/edgeplane.service`:

```ini
[Unit]
Description=EdgePlane Control Plane
After=network.target postgresql.service

[Service]
Type=simple
ExecStart=/usr/local/bin/edgeplane-tower --bind 0.0.0.0:8008
Restart=on-failure
EnvironmentFile=/etc/edgeplane/env

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now edgeplane
```

### 4. Add a Reverse Proxy / TLS

Terminate TLS at a reverse proxy and forward plain HTTP to tower on port 8008. The tower itself does not handle TLS.

**nginx example:**

```nginx
server {
    listen 443 ssl;
    server_name your-tower-host;

    ssl_certificate     /etc/ssl/certs/edgeplane.crt;
    ssl_certificate_key /etc/ssl/private/edgeplane.key;

    location / {
        proxy_pass         http://localhost:8008;
        proxy_http_version 1.1;
        proxy_set_header   Host              $host;
        proxy_set_header   X-Real-IP         $remote_addr;
        proxy_set_header   X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header   X-Forwarded-Proto $scheme;
        # WebSocket support (attach-ws)
        proxy_set_header   Upgrade    $http_upgrade;
        proxy_set_header   Connection "upgrade";
        proxy_read_timeout 3600s;
    }
}
```

**Caddy example:**

```
your-tower-host {
    reverse_proxy localhost:8008 {
        header_up X-Forwarded-Proto {scheme}
    }
}
```

### 5. Verify

```bash
curl https://your-tower-host/api/health
curl https://your-tower-host/api/raft/status
```

## Docker Compose

The repo ships a production-oriented Compose stack and a quickstart variant.

**Quickstart (local dev — Postgres + RustFS, no external infrastructure required):**

```bash
docker compose -f docker-compose.quickstart.yml up
```

**Full stack (Postgres + S3-compatible storage):**

Provide secrets via environment before startup:

```bash
export POSTGRES_PASSWORD=<password>
export EP_OBJECT_STORAGE_ACCESS_KEY=<key>
export EP_OBJECT_STORAGE_ACCESS_SECRET=<secret>
docker compose up
```

Health endpoints:
- `/api/health` — process alive (no auth required)

## Kubernetes

When running on Kubernetes, source all secrets via platform secret objects — do not commit credentials to Git.

```yaml
# Recommended pattern: envFrom + secretRef
spec:
  containers:
  - name: edgeplane-tower
    image: ghcr.io/ryanmerlin/edgeplane:<version>
    envFrom:
    - secretRef:
        name: edgeplane-env
    ports:
    - containerPort: 8008
```

Store all auth settings (OIDC credentials, DB credentials, object storage credentials) as Kubernetes Secrets and mount via `envFrom.secretRef` or `env.valueFrom.secretKeyRef`.

See [Helm chart](https://github.com/RyanMerlin/edgeplane/tree/main/infra/helm/edgeplane) in the repo for a complete Kubernetes deployment.

## Authentication

EdgePlane uses three auth paths — no static API token:

| Mode | Who | How |
|------|-----|-----|
| **OIDC session** | Human operators | `edgeplane auth login` → browser flow → `~/.edgeplane/session.json` |
| **Node JWT** | Daemon machines | `edgeplane agent node register` → `/etc/edgeplane/node.json` |
| **Service account** | CI / scripted | Create via API; pass as `EP_AGENT_TOKEN` |

See [OIDC Authentication](/guides/oidc/) and [Security Model](/architecture/security/) for setup details.

## Database Migrations

`edgeplane-tower` runs migrations automatically on startup. To skip and run them separately, use the `--no-migrate` flag. The embedded migrations are applied via sqlx on the configured `DATABASE_URL`.

## Validation Checklist

After deployment:

- [ ] `GET /api/health` returns 200 without auth
- [ ] `edgeplane health --json` returns connected from operator workstation
- [ ] Create + delete mission paths work with expected authorization

## See Also

- [OIDC Authentication](/guides/oidc/) — configure SSO
- [Upgrading](/guides/upgrading/) — release upgrade checklist
