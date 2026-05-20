FROM rust:1.86-slim AS builder
WORKDIR /build

COPY crates/mc/Cargo.toml crates/mc/Cargo.lock ./crates/mc/
RUN mkdir -p crates/mc/src && echo 'fn main() {}' > crates/mc/src/main.rs
RUN cargo build --release --manifest-path crates/mc/Cargo.toml

COPY crates/mc ./crates/mc
COPY distribution/templates ./distribution/templates
COPY distribution/hooks ./distribution/hooks
RUN cargo build --release --manifest-path crates/mc/Cargo.toml

FROM debian:bookworm-slim
RUN apt-get update     && apt-get install -y --no-install-recommends ca-certificates curl     && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/crates/mc/target/release/mc /usr/local/bin/mc

ENTRYPOINT ["mc", "daemon"]
