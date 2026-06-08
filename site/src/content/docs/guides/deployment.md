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
# Auth
AUTH_MODE=dual
OIDC_REQUIRED=false
OIDC_ISSUER_URL=https://<your-idp-host>/application/o/<provider-slug>/
OIDC_AUDIENCE=<oidc-client-id>
EP_ADMIN_EMAILS=<comma-separated-admin-emails>

# Database
DATABASE_URL=postgresql://edgeplane:password@localhost/edgeplane
DB_POOL_SIZE=20
DB_MAX_OVERFLOW=10
DB_POOL_PRE_PING=true
DB_POOL_RECYCLE_SECONDS=3600
EP_DB_RUNTIME_MIGRATIONS=false

# S3-compatible object storage (optional, for artifact/doc content)
EP_OBJECT_STORAGE_ENDPOINT=http://<s3-host>:<port>
EP_OBJECT_STORAGE_REGION=us-east-1
EP_OBJECT_STORAGE_BUCKET=edgeplane
EP_OBJECT_STORAGE_SECURE=false
EP_OBJECT_STORAGE_ACCESS_KEY=<access-key>
EP_OBJECT_STORAGE_ACCESS_SECRET=<secret>

# Request limits (optional)
EP_REQUEST_TIMEOUT_SECONDS=30
EP_RATE_LIMIT_DEFAULT_CAPACITY=240
EP_RATE_LIMIT_SEARCH_CAPACITY=60
EP_RATE_LIMIT_WRITE_CAPACITY=120
EP_RATE_LIMIT_APPROVAL_CAPACITY=30
```

### 3. systemd service

Create `/etc/systemd/system/edgeplane.service`:

```ini
[Unit]
Description=EdgePlane Control Plane
After=network.target postgresql.service

[Service]
Type=simple
ExecStart=/usr/local/bin/edgeplane-tower --serve --bind 0.0.0.0:8008
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
- `/api/readyz` — DB ready, object storage reachable when configured

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

Store all auth settings (OIDC secrets, static token, DB credentials, S3 credentials) as Kubernetes Secrets and mount via `envFrom.secretRef` or `env.valueFrom.secretKeyRef`.

See [Helm chart](https://github.com/RyanMerlin/edgeplane/tree/main/infra/helm/edgeplane) in the repo for a complete Kubernetes deployment.

## Auth Modes

| `AUTH_MODE` | Behavior |
|-------------|---------|
| `token` | Static bearer token only |
| `oidc` | OIDC JWT only |
| `dual` | Accept both token and OIDC |

`OIDC_REQUIRED=true` in `dual` mode enforces OIDC for non-`/mcp` paths. If `AUTH_MODE` is unset, the server defaults to OIDC when OIDC vars are present.

## Database Migrations

`edgeplane-tower` runs migrations automatically on startup. To run manually:

```bash
cd crates/edgeplane-tower && sqlx migrate run
```

Confirm migration state:

```bash
sqlx migrate info
```

## Validation Checklist

After deployment:

- [ ] `GET /api/health` returns 200 without auth
- [ ] `GET /api/readyz` returns 200 (DB ready, S3 reachable if configured)
- [ ] `edgeplane health --json` returns connected from operator workstation
- [ ] Bearer token callers are not admins unless their subject/email is in `EP_ADMIN_SUBJECTS` or `EP_ADMIN_EMAILS`
- [ ] Create + delete mission paths work with expected authorization

## See Also

- [OIDC Authentication](/guides/oidc/) — configure SSO
- [Upgrading](/guides/upgrading/) — release upgrade checklist
