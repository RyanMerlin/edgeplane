FROM rust:1.86-slim AS builder
WORKDIR /build

COPY crates/edgeplane/Cargo.toml crates/edgeplane/Cargo.lock ./crates/edgeplane/
RUN mkdir -p crates/edgeplane/src && echo 'fn main() {}' > crates/edgeplane/src/main.rs
RUN cargo build --release --manifest-path crates/edgeplane/Cargo.toml

COPY crates/edgeplane ./crates/edgeplane
COPY distribution/templates ./distribution/templates
COPY distribution/hooks ./distribution/hooks
RUN cargo build --release --manifest-path crates/edgeplane/Cargo.toml

FROM debian:bookworm-slim
RUN apt-get update     && apt-get install -y --no-install-recommends ca-certificates curl     && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/crates/edgeplane/target/release/edgeplane /usr/local/bin/edgeplane

ENTRYPOINT ["edgeplane", "daemon"]
