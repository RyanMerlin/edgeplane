---
title: Deployment
description: Deploy MissionControl on a Linux VM, with Docker Compose, or on Kubernetes.
---

This guide covers three deployment paths: Linux VM with systemd, Docker Compose, and Kubernetes.

## Prerequisites

- PostgreSQL 14+
- S3-compatible object storage (AWS S3, MinIO, or compatible self-hosted)
- `mc-controlplane` binary (see [Installation](/missioncontrol/getting-started/installation/))

## Linux VM / systemd

### 1. Place the binary

```bash
cp target/release/mc-controlplane /usr/local/bin/mc-controlplane
```

### 2. Environment file

Create `/etc/missioncontrol/env`:

```bash
# Auth
AUTH_MODE=dual
OIDC_REQUIRED=false
MC_TOKEN=<static-token-for-mcp>
OIDC_ISSUER_URL=https://<your-idp-host>/application/o/<provider-slug>/
OIDC_AUDIENCE=<oidc-client-id>
MC_ADMIN_EMAILS=<comma-separated-admin-emails>

# Database
DATABASE_URL=postgresql://mc:password@localhost/missioncontrol
DB_POOL_SIZE=20
DB_MAX_OVERFLOW=10
DB_POOL_PRE_PING=true
DB_POOL_RECYCLE_SECONDS=3600
MC_DB_RUNTIME_MIGRATIONS=false

# S3-compatible object storage (optional, for artifact/doc content)
MC_OBJECT_STORAGE_ENDPOINT=http://<s3-host>:<port>
MC_OBJECT_STORAGE_REGION=us-east-1
MC_OBJECT_STORAGE_BUCKET=missioncontrol
MC_OBJECT_STORAGE_SECURE=false
MC_OBJECT_STORAGE_ACCESS_KEY=<access-key>
MC_OBJECT_STORAGE_ACCESS_SECRET=<secret>

# Request limits (optional)
MC_REQUEST_TIMEOUT_SECONDS=30
MC_RATE_LIMIT_DEFAULT_CAPACITY=240
MC_RATE_LIMIT_SEARCH_CAPACITY=60
MC_RATE_LIMIT_WRITE_CAPACITY=120
MC_RATE_LIMIT_APPROVAL_CAPACITY=30
```

### 3. systemd service

Create `/etc/systemd/system/missioncontrol.service`:

```ini
[Unit]
Description=MissionControl Control Plane
After=network.target postgresql.service

[Service]
Type=simple
ExecStart=/usr/local/bin/mc-controlplane --serve --bind 0.0.0.0:8008
Restart=on-failure
EnvironmentFile=/etc/missioncontrol/env

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now missioncontrol
```

### 4. Verify

```bash
curl http://localhost:8008/health
curl http://localhost:8008/raft/status
```

## Docker Compose

The repo ships a production-oriented Compose stack and a quickstart variant.

**Quickstart (local dev — SQLite, no object storage):**

```bash
docker compose -f docker-compose.quickstart.yml up
```

**Full stack (Postgres + S3-compatible storage):**

Provide secrets via environment before startup:

```bash
export POSTGRES_PASSWORD=<password>
export MC_OBJECT_STORAGE_ACCESS_KEY=<key>
export MC_OBJECT_STORAGE_ACCESS_SECRET=<secret>
docker compose up
```

Health endpoints:
- `/health` — process alive (no auth required)
- `/readyz` — DB ready, object storage reachable when configured

## Kubernetes

When running on Kubernetes, source all secrets via platform secret objects — do not commit credentials to Git.

```yaml
# Recommended pattern: envFrom + secretRef
spec:
  containers:
  - name: mc-controlplane
    image: ghcr.io/ryanmerlin/missioncontrol:<version>
    envFrom:
    - secretRef:
        name: missioncontrol-env
    ports:
    - containerPort: 8008
```

Store all auth settings (OIDC secrets, static token, DB credentials, S3 credentials) as Kubernetes Secrets and mount via `envFrom.secretRef` or `env.valueFrom.secretKeyRef`.

See [Helm chart](https://github.com/RyanMerlin/missioncontrol/tree/main/infra/helm/missioncontrol) in the repo for a complete Kubernetes deployment.

## Auth Modes

| `AUTH_MODE` | Behavior |
|-------------|---------|
| `token` | Static bearer token only |
| `oidc` | OIDC JWT only |
| `dual` | Accept both token and OIDC |

`OIDC_REQUIRED=true` in `dual` mode enforces OIDC for non-`/mcp` paths. If `AUTH_MODE` is unset, the server defaults to OIDC when OIDC vars are present, and falls back to token mode when only `MC_TOKEN` is configured.

## Database Migrations

`mc-controlplane` runs migrations automatically on startup. To run manually:

```bash
cd crates/mc-controlplane && sqlx migrate run
```

Confirm migration state:

```bash
sqlx migrate info
```

## Validation Checklist

After deployment:

- [ ] `GET /health` returns 200 without auth
- [ ] `GET /readyz` returns 200 (DB ready, S3 reachable if configured)
- [ ] `mc health --json` returns connected from operator workstation
- [ ] Bearer token callers are not admins unless their subject/email is in `MC_ADMIN_SUBJECTS` or `MC_ADMIN_EMAILS`
- [ ] Create + delete mission paths work with expected authorization

## See Also

- [OIDC Authentication](/missioncontrol/guides/oidc/) — configure SSO
- [Upgrading](/missioncontrol/guides/upgrading/) — release upgrade checklist
