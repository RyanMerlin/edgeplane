#!/usr/bin/env bash
# Sync the vendored ACP schema with the latest npm release.
#
# Run from anywhere; resolves paths relative to the script.
# - Fetches @zed-industries/agent-client-protocol@latest into a tempdir.
# - Diffs the new schema against the vendored copy.
# - If different: updates schema.json + VERSION, then runs `cargo test -p mc-mesh-acp`.
#   The build.rs regenerates Rust types from the new schema; any breaking
#   change in upstream surfaces as a Rust compile error in dependent code.
# - If unchanged: prints "up to date" and exits 0.

set -euo pipefail

CRATE_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SCHEMA_FILE="$CRATE_DIR/schema/schema.json"
VERSION_FILE="$CRATE_DIR/schema/VERSION"
WORKSPACE_DIR="$(cd "$CRATE_DIR/../.." && pwd)"

# The protocol moved namespaces in early 2026: was
# `@zed-industries/agent-client-protocol`, now `@agentclientprotocol/sdk`.
# We track whatever version `@zed-industries/claude-code-acp` (the agent
# we drive) depends on — pulling "latest" can desync us from the agent.
# Override with NPM_PKG_VERSION env var to force a specific version.
NPM_PKG="@agentclientprotocol/sdk"
# Default: pinned at 0.14.1 — last version where typify 0.6 generates clean
# Rust types for all schemas we use. Newer schemas (0.21+) use additional
# discriminator/oneOf patterns on UNSTABLE types (SessionConfigOption,
# SessionModelState) that typify lowers as `Variant0/Variant1/...` — see
# the wire.rs hand-rolled enums. To upgrade, either extend wire.rs with
# replacements for those enums or switch typify to a newer release with
# better discriminator support. Override via `NPM_PKG_VERSION=0.21.0 make sync-acp`.
NPM_PKG_VERSION="${NPM_PKG_VERSION:-0.14.1}"
# Both renamed in early 2026; old names work via deprecation aliases.
# `@zed-industries/claude-code-acp` → `@agentclientprotocol/claude-agent-acp`
AGENT_PKG="${AGENT_PKG:-@agentclientprotocol/claude-agent-acp}"

if ! command -v npm >/dev/null 2>&1; then
    echo "error: npm is required to sync the ACP schema" >&2
    exit 2
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> fetching $NPM_PKG@$NPM_PKG_VERSION"
(cd "$WORK" && npm init -y >/dev/null 2>&1 && npm install --no-audit --no-fund "$NPM_PKG@$NPM_PKG_VERSION" >/dev/null)

NEW_SCHEMA="$WORK/node_modules/$NPM_PKG/schema/schema.json"
NEW_VERSION=$(node -e "console.log(require('$WORK/node_modules/$NPM_PKG/package.json').version)")

if [[ ! -f "$NEW_SCHEMA" ]]; then
    echo "error: $NEW_SCHEMA missing in fetched package" >&2
    exit 3
fi

CURRENT_VERSION=$(cat "$VERSION_FILE" 2>/dev/null || echo "<none>")
echo "==> current vendored: $CURRENT_VERSION"
echo "==> upstream latest:  $NEW_VERSION"

if cmp -s "$NEW_SCHEMA" "$SCHEMA_FILE"; then
    echo "==> schema up to date — nothing to do"
    if [[ "$CURRENT_VERSION" != "$NEW_VERSION" ]]; then
        echo "==> bumping VERSION file ($CURRENT_VERSION -> $NEW_VERSION)"
        echo "$NEW_VERSION" > "$VERSION_FILE"
    fi
    exit 0
fi

echo "==> schema changed — diff:"
diff -u "$SCHEMA_FILE" "$NEW_SCHEMA" || true
echo
echo "==> updating vendored schema and VERSION"
cp "$NEW_SCHEMA" "$SCHEMA_FILE"
echo "$NEW_VERSION" > "$VERSION_FILE"

echo "==> rebuilding + testing mc-mesh-acp (build.rs will regenerate types)"
(cd "$WORKSPACE_DIR" && cargo test -p mc-mesh-acp)

echo
echo "Sync complete. Review the changes in:"
echo "  $SCHEMA_FILE"
echo "  $VERSION_FILE"
echo "  (and the regenerated types via \`cargo expand -p mc-mesh-acp schema\`)"
