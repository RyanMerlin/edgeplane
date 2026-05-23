# opencode HTTP service — Edgeplane AI Console runtime
#
# Downloads the official pre-built opencode binary from GitHub releases
# and runs it in HTTP server mode so OpencodeAdapter can reach it at
# EP_OPENCODE_URL (default: http://opencode:4096).
#
# Build args:
#   OPENCODE_VERSION — exact release tag to download (default: 1.2.26)
#
# opencode serve API:
#   GET  /global/health      → {healthy: true, version: "x.y.z"}
#   POST /session            → {id: "..."}
#   POST /session/{id}/prompt_async → 204
#   GET  /permission         → pending permission requests
#   POST /permission/{id}/reply    → {reply: "once"|"always"|"reject"}
#   DELETE /session/{id}     → true

ARG OPENCODE_VERSION=1.2.26

# ── Stage 1: download & verify the binary ────────────────────────────────────
FROM alpine AS downloader
ARG OPENCODE_VERSION
ARG TARGETARCH

RUN apk add --no-cache curl ca-certificates

RUN set -ex; \
    # Map Docker TARGETARCH → opencode release arch name \
    case "${TARGETARCH:-amd64}" in \
        amd64) ARCH="x64" ;; \
        arm64) ARCH="arm64" ;; \
        *)     ARCH="x64" ;; \
    esac; \
    TARBALL="opencode-linux-${ARCH}-musl.tar.gz"; \
    URL="https://github.com/sst/opencode/releases/download/v${OPENCODE_VERSION}/${TARBALL}"; \
    echo "Downloading ${URL}"; \
    curl -fsSL "${URL}" -o /tmp/opencode.tar.gz; \
    mkdir -p /tmp/oc; \
    tar -xzf /tmp/opencode.tar.gz -C /tmp/oc; \
    install -m 755 /tmp/oc/opencode /usr/local/bin/opencode

# ── Stage 2: minimal runtime ─────────────────────────────────────────────────
FROM alpine

RUN apk add --no-cache libgcc libstdc++ ripgrep ca-certificates

COPY --from=downloader /usr/local/bin/opencode /usr/local/bin/opencode

RUN opencode --version

# opencode sessions create workspaces under HOME
ENV HOME=/root
WORKDIR /workspace

EXPOSE 4096

HEALTHCHECK --interval=15s --timeout=5s --retries=8 --start-period=30s \
  CMD wget -qO- http://localhost:4096/global/health 2>/dev/null | grep -q '"healthy":true' || exit 1

ENTRYPOINT ["opencode"]
CMD ["serve", "--hostname", "0.0.0.0", "--port", "4096"]
