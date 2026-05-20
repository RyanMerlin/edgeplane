FROM node:22-slim AS web-builder
WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM rust:1.88-slim AS builder
WORKDIR /build

COPY crates/mc-controlplane/Cargo.toml crates/mc-controlplane/Cargo.lock ./crates/mc-controlplane/
RUN mkdir -p crates/mc-controlplane/src && \
    echo 'fn main() {}' > crates/mc-controlplane/src/main.rs && \
    echo 'pub fn stub() {}' > crates/mc-controlplane/src/lib.rs
RUN cargo build --release --manifest-path crates/mc-controlplane/Cargo.toml

COPY crates/mc-controlplane ./crates/mc-controlplane
RUN touch crates/mc-controlplane/src/main.rs crates/mc-controlplane/src/lib.rs && \
    cargo build --release --manifest-path crates/mc-controlplane/Cargo.toml

FROM debian:bookworm-slim AS prod
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/crates/mc-controlplane/target/release/mc-controlplane /usr/local/bin/mc-controlplane
COPY --from=web-builder /build/web/build /usr/local/share/mc-web

EXPOSE 8008
ENTRYPOINT ["mc-controlplane"]
CMD []
