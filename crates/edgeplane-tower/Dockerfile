FROM node:24-slim AS web-builder
WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web ./
RUN npm run build

FROM rust:1.88-slim AS builder
WORKDIR /build

# Workspace Cargo.lock lives at repo root — copy both workspace manifest and lock file
COPY Cargo.toml Cargo.lock ./

# Copy all workspace member Cargo.toml files (required for workspace resolver)
COPY crates/edgeplane/Cargo.toml ./crates/edgeplane/Cargo.toml
COPY crates/edgeplane-tower/Cargo.toml ./crates/edgeplane-tower/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-bin/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-bin/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-acp/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-acp/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-core/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-core/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-packs/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-packs/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-receipts/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-receipts/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-runtimes/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-runtimes/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-sandbox/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-sandbox/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-secrets/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-secrets/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-sync/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-sync/Cargo.toml
COPY crates/edgeplaned/crates/edgeplaned-work/Cargo.toml ./crates/edgeplaned/crates/edgeplaned-work/Cargo.toml

# Stub all workspace members so the resolver is satisfied, then cache edgeplane-tower deps
RUN for d in \
      crates/edgeplane \
      crates/edgeplane-tower \
      crates/edgeplaned/crates/edgeplaned-bin \
      crates/edgeplaned/crates/edgeplaned-acp \
      crates/edgeplaned/crates/edgeplaned-core \
      crates/edgeplaned/crates/edgeplaned-packs \
      crates/edgeplaned/crates/edgeplaned-receipts \
      crates/edgeplaned/crates/edgeplaned-runtimes \
      crates/edgeplaned/crates/edgeplaned-sandbox \
      crates/edgeplaned/crates/edgeplaned-secrets \
      crates/edgeplaned/crates/edgeplaned-sync \
      crates/edgeplaned/crates/edgeplaned-work; do \
      mkdir -p $d/src && \
      echo 'fn main() {}' > $d/src/main.rs && \
      printf 'pub fn stub() {}\n' > $d/src/lib.rs; \
    done
# edgeplane-tower declares a second [[bin]] (gen-openapi, src/bin/gen_openapi.rs),
# used only for OpenAPI codegen in CI. The dependency-cache build below compiles
# ALL of the crate's bins, so the stub stage must also stub this one or it fails
# with "couldn't read src/bin/gen_openapi.rs". The real source replaces it at the
# COPY below, so this stub only exists to satisfy the dep-caching build.
RUN mkdir -p crates/edgeplane-tower/src/bin \
    && echo 'fn main() {}' > crates/edgeplane-tower/src/bin/gen_openapi.rs
RUN cargo build --release -p edgeplane-tower

# Replace stub source with real edgeplane-tower source and rebuild
COPY crates/edgeplane-tower ./crates/edgeplane-tower
RUN touch crates/edgeplane-tower/src/main.rs crates/edgeplane-tower/src/lib.rs && \
    cargo build --release -p edgeplane-tower

FROM debian:bookworm-slim AS prod
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
# Binary is at workspace target root, not inside the crate directory
COPY --from=builder /build/target/release/edgeplane-tower /usr/local/bin/edgeplane-tower
COPY --from=web-builder /build/web/dist /usr/local/share/edgeplane-web

EXPOSE 8008
ENTRYPOINT ["edgeplane-tower"]
CMD []
