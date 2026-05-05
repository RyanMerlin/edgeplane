# ── mc-server builder ────────────────────────────────────────────────────────
FROM rust:1.88-slim AS mc-server-builder
WORKDIR /build

COPY integrations/mc-server/Cargo.toml integrations/mc-server/Cargo.lock ./integrations/mc-server/
RUN mkdir -p integrations/mc-server/src && \
    echo 'fn main() {}' > integrations/mc-server/src/main.rs && \
    echo 'pub fn stub() {}' > integrations/mc-server/src/lib.rs
RUN cargo build --release --manifest-path integrations/mc-server/Cargo.toml

COPY integrations/mc-server ./integrations/mc-server
RUN touch integrations/mc-server/src/main.rs integrations/mc-server/src/lib.rs && \
    cargo build --release --manifest-path integrations/mc-server/Cargo.toml

# ── migrate: minimal Python image for Alembic ────────────────────────────────
FROM python:3.11-slim AS migrate
WORKDIR /app
ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_NO_CACHE_DIR=1
COPY backend/requirements-migrate.txt /app/
RUN pip install --no-cache-dir -r requirements-migrate.txt
COPY backend/alembic /app/alembic
COPY backend/alembic.ini /app/alembic.ini
COPY backend/app/models.py \
     backend/app/db.py \
     backend/app/config_paths.py \
     /app/app/
COPY backend/app/services/secrets.py /app/app/services/
RUN touch /app/app/__init__.py /app/app/services/__init__.py
CMD ["python", "-m", "alembic", "upgrade", "head"]

# ── prod: pure Rust binary ────────────────────────────────────────────────────
FROM debian:trixie-slim AS prod
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 3000 missioncontrol \
    && useradd --uid 3000 --gid 3000 --no-create-home --shell /usr/sbin/nologin missioncontrol
COPY --from=mc-server-builder \
    /build/integrations/mc-server/target/release/mc-server \
    /usr/local/bin/mc-server
USER missioncontrol
EXPOSE 8008
CMD ["mc-server", "--bind", "0.0.0.0:8008"]
