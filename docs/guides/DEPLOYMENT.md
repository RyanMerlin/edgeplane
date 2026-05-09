# MissionControl Deployment Guide (Linux VM / Compose)

This guide covers deploying MissionControl API on a Linux VM or with the hardened Compose stack in this repo.

## Objectives
- Run MissionControl API as a system service.
- Configure auth via OIDC and/or static MC token without implicit token-admin elevation.
- Configure optional RustFS/S3 object storage for docs/artifacts.
- Expose health endpoints and bounded runtime settings suitable for production.

## MissionControl API (Linux VM)

### 1) Place code
Recommended path:
- `/opt/missioncontrol/backend`

### 2) Python venv + deps
```bash
cd /opt/missioncontrol/backend
python3 -m venv /opt/missioncontrol/.venv
/opt/missioncontrol/.venv/bin/pip install -r requirements.txt
```

### 3) Environment file
Create `/opt/missioncontrol/.env` (minimum example):

```env
AUTH_MODE=dual
OIDC_REQUIRED=false
MC_TOKEN=<static-token-for-mcp>
OIDC_ISSUER_URL=https://<authentik-host>/application/o/<provider-slug>/
OIDC_AUDIENCE=<oidc-client-id>
MC_ADMIN_EMAILS=<comma-separated-admin-emails>
DB_POOL_SIZE=20
DB_MAX_OVERFLOW=10
DB_POOL_PRE_PING=true
DB_POOL_RECYCLE_SECONDS=3600
MC_DB_RUNTIME_MIGRATIONS=false
MC_REQUEST_TIMEOUT_SECONDS=30
```

Optional RustFS/S3-backed doc/artifact content:

```env
MC_OBJECT_STORAGE_ENDPOINT=http://<rustfs-host>:<port>
MC_OBJECT_STORAGE_REGION=us-east-1
MC_OBJECT_STORAGE_BUCKET=missioncontrol
MC_OBJECT_STORAGE_SECURE=false
MC_OBJECT_STORAGE_ACCESS_KEY=<access-key>
MC_OBJECT_STORAGE_ACCESS_SECRET=<secret>
```

Optional request guards:

```env
MC_RATE_LIMIT_DEFAULT_CAPACITY=240
MC_RATE_LIMIT_SEARCH_CAPACITY=60
MC_RATE_LIMIT_WRITE_CAPACITY=120
MC_RATE_LIMIT_APPROVAL_CAPACITY=30
```

### 4) systemd service
Create `/etc/systemd/system/missioncontrol.service`:

```ini
[Unit]
Description=MissionControl API
After=network.target

[Service]
Type=simple
WorkingDirectory=/opt/missioncontrol/backend
ExecStart=/opt/missioncontrol/.venv/bin/uvicorn app.main:app --host 0.0.0.0 --port 8000
Restart=on-failure
Environment=PYTHONUNBUFFERED=1
EnvironmentFile=/opt/missioncontrol/.env

[Install]
WantedBy=multi-user.target
```

Enable + start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable --now missioncontrol
```

Verify:
```bash
curl http://localhost:8000/
curl http://localhost:8000/healthz
curl http://localhost:8000/readyz
```

## Docker Compose

- `docker-compose.yml` is now the production-oriented stack.
- Bucket bootstrap is handled by the one-shot `rustfs-init` service using Python/boto3 (no `minio/mc` dependency).
- Provide secrets through the environment before startup:
  - `POSTGRES_PASSWORD`
  - `MQTT_PASSWORD`
  - `MC_OBJECT_STORAGE_ACCESS_KEY`
  - `MC_OBJECT_STORAGE_ACCESS_SECRET`
- Create a Mosquitto password file at `docker/mosquitto/passwords/passwords` before starting the production stack.
- `docker-compose.quickstart.yml` keeps the local/dev posture with sqlite and anonymous MQTT.
- API health contracts:
  - `/healthz`: process alive
  - `/readyz`: DB ready, object storage reachable when configured, MQTT connected when required

## Kubernetes note

When deploying on Kubernetes, provide secrets via the platform’s secret objects and avoid checked-in `.env` files. Mount values through `envFrom`/`env.valueFrom` references so runtime credentials stay outside Git history.

## Validation Checklist
- `curl http://localhost:8000/` returns status ok.
- `curl http://localhost:8000/healthz` returns status ok without auth.
- `curl http://localhost:8000/readyz` returns status ok only when dependencies are ready.
- `curl -H "Authorization: Bearer <MC_TOKEN>" http://localhost:8000/mcp/health` returns ok.
- Bearer token callers are not platform admins unless their subject/email is allowlisted in `MC_ADMIN_SUBJECTS` or `MC_ADMIN_EMAILS`.
- Docs/artifacts create + delete paths work with expected mission authz.
