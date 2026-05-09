# ── mc-controlplane builder ───────────────────────────────────────────────────
FROM rust:1.88-slim AS mc-controlplane-builder
WORKDIR /build

COPY integrations/mc-controlplane/Cargo.toml integrations/mc-controlplane/Cargo.lock ./integrations/mc-controlplane/
RUN mkdir -p integrations/mc-controlplane/src && \
    echo 'fn main() {}' > integrations/mc-controlplane/src/main.rs && \
    echo 'pub fn stub() {}' > integrations/mc-controlplane/src/lib.rs
RUN cargo build --release --manifest-path integrations/mc-controlplane/Cargo.toml

COPY integrations/mc-controlplane ./integrations/mc-controlplane
RUN touch integrations/mc-controlplane/src/main.rs integrations/mc-controlplane/src/lib.rs && \
    cargo build --release --manifest-path integrations/mc-controlplane/Cargo.toml

# ── prod: pure Rust binary ────────────────────────────────────────────────────
FROM debian:trixie-slim AS prod
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 3000 missioncontrol \
    && useradd --uid 3000 --gid 3000 --no-create-home --shell /usr/sbin/nologin missioncontrol
COPY --from=mc-controlplane-builder \
    /build/integrations/mc-controlplane/target/release/mc-controlplane \
    /usr/local/bin/mc-controlplane
USER missioncontrol
EXPOSE 8008
CMD ["mc-controlplane", "--bind", "0.0.0.0:8008"]
