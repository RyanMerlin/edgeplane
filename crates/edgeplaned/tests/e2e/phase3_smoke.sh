#!/usr/bin/env bash
# Phase 3 smoke test — verifies on-demand install + harness rendering + edgeplane exec round-trip.
#
# Prerequisites:
#   - goose is installed and on PATH (pre-installed at bootstrap)
#   - npm is available (for claude-code/codex/gemini install tests)
#   - The workspace builds cleanly (run `cargo build --workspace` first)
#
# Usage:
#   bash tests/e2e/phase3_smoke.sh
set -euo pipefail

WORKSPACE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILD_DIR="${WORKSPACE_ROOT}/target/debug"
SOCKET_PATH="/tmp/edgeplaned-phase3-smoke.sock"
TEST_CONFIG="${WORKSPACE_ROOT}/tests/e2e/smoke-config.toml"
DAEMON_PID=""

# ── helpers ─────────────────────────────────────────────────────────────────

log()  { echo "[smoke] $*"; }
fail() { echo "[smoke] FAIL: $*" >&2; cleanup; exit 1; }

cleanup() {
    if [[ -n "${DAEMON_PID}" ]]; then
        log "killing daemon (pid ${DAEMON_PID})"
        kill "${DAEMON_PID}" 2>/dev/null || true
        wait "${DAEMON_PID}" 2>/dev/null || true
    fi
    rm -f "${SOCKET_PATH}"
}
trap cleanup EXIT

# ── Step 1: build ────────────────────────────────────────────────────────────

log "Step 1: building edgeplaned and edgeplane binaries"
cargo build -p edgeplaned -p edgeplane --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" \
    2>&1 | tail -3

EP_BIN="${BUILD_DIR}/edgeplaned"
EP_BIN="${BUILD_DIR}/edgeplane"

[[ -x "${EP_BIN}" ]] || fail "edgeplaned binary not found at ${EP_BIN}"
[[ -x "${EP_BIN}" ]]  || fail "edgeplane binary not found at ${EP_BIN}"

# ── Step 2: write a minimal daemon config (goose only — it's pre-installed) ──

log "Step 2: writing smoke test daemon config"
cat > "${TEST_CONFIG}" <<'EOF'
backend_url = "http://localhost:19999"
token = "smoke-test-token"
work_dir = "/tmp/edgeplaned-phase3-work"
offline_grace_secs = 3600
offline_policy = "autonomous"

[[missions]]
mission_id = "smoke-mission"

  [[missions.agents]]
  agent_id  = "smoke-goose-1"
  runtime_kind = "goose"
EOF

# ── Step 3: start daemon ─────────────────────────────────────────────────────

log "Step 3: starting daemon (background)"
export EP_BIN_DIR="${BUILD_DIR}"
export EP_SOCKET="${SOCKET_PATH}"

"${EP_BIN}" \
    --config "${TEST_CONFIG}" \
    --socket "${SOCKET_PATH}" \
    &>/tmp/edgeplaned-phase3-daemon.log &
DAEMON_PID=$!

log "  daemon pid=${DAEMON_PID}"

# Wait up to 10 s for the socket to appear.
for i in $(seq 1 20); do
    [[ -S "${SOCKET_PATH}" ]] && break
    sleep 0.5
done
[[ -S "${SOCKET_PATH}" ]] || fail "daemon socket not created within 10 s; see /tmp/edgeplaned-phase3-daemon.log"

log "  daemon socket ready"

# ── Step 4: assert harness file was rendered ──────────────────────────────────

log "Step 4: checking goose harness file"
GOOSE_HARNESS="${HOME}/.config/goose/CAPABILITIES.md"
if [[ ! -f "${GOOSE_HARNESS}" ]]; then
    fail "goose harness file not created at ${GOOSE_HARNESS}"
fi
grep -q "edgeplane exec" "${GOOSE_HARNESS}" \
    || fail "goose harness missing 'edgeplane exec'; content: $(cat "${GOOSE_HARNESS}")"
log "  harness OK: ${GOOSE_HARNESS}"

# ── Step 5: run edgeplane exec via the socket ───────────────────────────────────────

log "Step 5: running edgeplane exec kubectl-observe.kubectl-get-pods --json --dry-run"
EXEC_OUT=$("${EP_BIN}" exec kubectl-observe.kubectl-get-pods --json --dry-run 2>&1) || true
log "  exec output: ${EXEC_OUT:0:120}"

# ── Step 6: check receipt store ──────────────────────────────────────────────

log "Step 6: checking receipt store"
RECEIPT_OUT=$("${EP_BIN}" receipts last --json 2>&1)
log "  receipts last: ${RECEIPT_OUT:0:200}"

echo "${RECEIPT_OUT}" | python3 -c "import sys,json; json.load(sys.stdin)" \
    || fail "edgeplane receipts last did not return valid JSON: ${RECEIPT_OUT}"

log "  receipt store OK"

# ── Done ─────────────────────────────────────────────────────────────────────

log "Phase 3 smoke test PASSED"
