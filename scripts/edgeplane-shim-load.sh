#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_ROOT="${EP_SHIM_LOAD_OUT_ROOT:-$ROOT_DIR/artifacts/shim-load}"
RUN_ID="${EP_SHIM_LOAD_RUN_ID:-$(date +%Y%m%d%H%M%S)}"
WORKERS="${EP_SHIM_LOAD_WORKERS:-3}"
DURATION_SEC="${EP_SHIM_LOAD_DURATION_SEC:-60}"
BASE_URL="${EP_BASE_URL:-http://localhost:8008}"
SHIM_HOST="${EP_DAEMON_HOST:-127.0.0.1}"
SHIM_PORT="${EP_DAEMON_PORT:-8765}"
TOKEN="${EP_AGENT_TOKEN:-}"

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi
if [[ -z "$TOKEN" ]]; then
  echo "EP_AGENT_TOKEN is required" >&2
  exit 2
fi

mkdir -p "$OUT_ROOT"
RUN_DIR="$OUT_ROOT/$RUN_ID"
mkdir -p "$RUN_DIR/workers"

echo "== MC shim load test =="
echo "run_id=$RUN_ID workers=$WORKERS duration_sec=$DURATION_SEC shim=${SHIM_HOST}:${SHIM_PORT}"
echo "out_dir=$RUN_DIR"

curl -fsS -H "Authorization: Bearer ${TOKEN}" "$BASE_URL/mcp/health" >/dev/null
curl -fsS "http://${SHIM_HOST}:${SHIM_PORT}/v1/health" >/dev/null

worker() {
  local worker_id="$1"
  local wdir="$RUN_DIR/workers/w${worker_id}"
  mkdir -p "$wdir"
  local end_ts=$(( $(date +%s) + DURATION_SEC ))
  local attempts=0 ok=0 fail=0

  while [[ "$(date +%s)" -lt "$end_ts" ]]; do
    attempts=$((attempts + 1))
    local init_code tools_code
    init_code="$(curl -sS -o "$wdir/init-${attempts}.json" -w "%{http_code}" -X POST "http://${SHIM_HOST}:${SHIM_PORT}/v1/initialize" || true)"
    tools_code="$(curl -sS -o "$wdir/tools-${attempts}.json" -w "%{http_code}" "http://${SHIM_HOST}:${SHIM_PORT}/v1/tools" || true)"
    if [[ "$init_code" == "200" && "$tools_code" == "200" ]]; then
      ok=$((ok + 1))
    else
      fail=$((fail + 1))
      echo "attempt=${attempts} init=${init_code} tools=${tools_code}" >> "$wdir/errors.log"
    fi
  done

  jq -n \
    --arg worker_id "$worker_id" \
    --argjson attempts "$attempts" \
    --argjson ok "$ok" \
    --argjson fail "$fail" \
    '{worker_id:$worker_id,attempts:$attempts,ok:$ok,fail:$fail}' > "$wdir/status.json"
}

for i in $(seq 1 "$WORKERS"); do
  worker "$i" &
done
wait

jq -s '{
  workers: length,
  attempts: (map(.attempts)|add),
  ok: (map(.ok)|add),
  fail: (map(.fail)|add),
  success_rate: (if (map(.attempts)|add) == 0 then 0 else ((map(.ok)|add)/(map(.attempts)|add)) end)
}' "$RUN_DIR"/workers/*/status.json > "$RUN_DIR/summary.json"

echo "== summary =="
cat "$RUN_DIR/summary.json"
