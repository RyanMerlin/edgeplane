#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MC_MANIFEST_PATH="${ROOT_DIR}/crates/mc/Cargo.toml"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

TEST_HOME="$WORKDIR/home"
TEST_MC_HOME="$WORKDIR/mc-home"
TEST_BIN="$WORKDIR/bin"
mkdir -p "$TEST_HOME" "$TEST_MC_HOME" "$TEST_BIN"

# Preserve rust toolchain resolution after overriding HOME for isolation.
ORIG_HOME="${HOME:-}"
export CARGO_HOME="${CARGO_HOME:-$ORIG_HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$ORIG_HOME/.rustup}"

export HOME="$TEST_HOME"
export MC_HOME="$TEST_MC_HOME"
export MC_BASE_URL="${MC_BASE_URL:-http://127.0.0.1:8008}"
export MC_TOKEN="${MC_TOKEN:-launch-regression-token}"

# Stub binaries — exit 0 so mc run completes without launching a real agent.
for agent in codex claude gemini; do
    cat >"$TEST_BIN/$agent" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TEST_BIN/$agent"
done
export PATH="$TEST_BIN:$PATH"

run_mc() {
    cargo run --quiet --manifest-path "$MC_MANIFEST_PATH" -- "$@"
}

assert_exists() {
    local path="$1"
    [[ -e "$path" ]] || { echo "[launch-regression] FAIL: missing expected path: $path" >&2; exit 1; }
}

assert_not_exists() {
    local path="$1"
    [[ ! -e "$path" ]] || { echo "[launch-regression] FAIL: unexpected path exists: $path" >&2; exit 1; }
}

# ── codex: configs land in profile dir, not global home ──────────────────────
echo "[launch-regression] codex profile isolation"
run_mc run codex
assert_exists  "$MC_HOME/profiles/codex/default/codex-home/config.toml"
assert_not_exists "$TEST_HOME/.codex/config.toml"

# ── claude: configs land in profile dir, not global home ─────────────────────
echo "[launch-regression] claude profile isolation"
run_mc run claude
assert_exists  "$MC_HOME/profiles/default/claude/runtime/home/.claude.json"
assert_not_exists "$TEST_HOME/.claude.json"

echo "[launch-regression] ok"
# Note: gemini uses the legacy launch::run path which fetches the onboarding
# manifest from MC_BASE_URL. It cannot be tested without a live server.
