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
