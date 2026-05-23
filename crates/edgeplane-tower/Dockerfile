FROM node:22-slim AS web-builder
WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM rust:1.88-slim AS builder
WORKDIR /build

COPY crates/edgeplane-tower/Cargo.toml crates/edgeplane-tower/Cargo.lock ./crates/edgeplane-tower/
RUN mkdir -p crates/edgeplane-tower/src && \
    echo 'fn main() {}' > crates/edgeplane-tower/src/main.rs && \
    echo 'pub fn stub() {}' > crates/edgeplane-tower/src/lib.rs
RUN cargo build --release --manifest-path crates/edgeplane-tower/Cargo.toml

COPY crates/edgeplane-tower ./crates/edgeplane-tower
RUN touch crates/edgeplane-tower/src/main.rs crates/edgeplane-tower/src/lib.rs && \
    cargo build --release --manifest-path crates/edgeplane-tower/Cargo.toml

FROM debian:bookworm-slim AS prod
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/crates/edgeplane-tower/target/release/edgeplane-tower /usr/local/bin/edgeplane-tower
COPY --from=web-builder /build/web/build /usr/local/share/edgeplane-web

EXPOSE 8008
ENTRYPOINT ["edgeplane-tower"]
CMD []
