FROM rust:1.88-slim AS builder
WORKDIR /build

COPY integrations/mc-controlplane/Cargo.toml integrations/mc-controlplane/Cargo.lock ./integrations/mc-controlplane/
RUN mkdir -p integrations/mc-controlplane/src && \
    echo 'fn main() {}' > integrations/mc-controlplane/src/main.rs && \
    echo 'pub fn stub() {}' > integrations/mc-controlplane/src/lib.rs
RUN cargo build --release --manifest-path integrations/mc-controlplane/Cargo.toml

COPY integrations/mc-controlplane ./integrations/mc-controlplane
RUN touch integrations/mc-controlplane/src/main.rs integrations/mc-controlplane/src/lib.rs && \
    cargo build --release --manifest-path integrations/mc-controlplane/Cargo.toml

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/integrations/mc-controlplane/target/release/mc-controlplane /usr/local/bin/mc-controlplane

EXPOSE 8008
ENTRYPOINT ["mc-controlplane"]
CMD []
