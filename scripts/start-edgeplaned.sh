#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${EP_BASE_URL:-http://localhost:8008}"
TOKEN="${EP_TOKEN:-}"
SHIM_HOST="${EP_DAEMON_HOST:-127.0.0.1}"
SHIM_PORT="${EP_DAEMON_PORT:-8765}"
MATRIX_ENDPOINT="${EP_MATRIX_ENDPOINT:-/events/stream}"
FANOUT_PORT="${EP_FANOUT_PORT:-}"
ENABLE_MATRIX="${EP_ENABLE_MATRIX:-0}"

if ! command -v edgeplane >/dev/null 2>&1; then
  echo "edgeplane binary not found on PATH." >&2
  echo "Install it first:" >&2
  echo "  bash scripts/install-edgeplane.sh" >&2
  exit 127
fi

if [[ -z "$TOKEN" ]]; then
  echo "EP_TOKEN is required" >&2
  exit 2
fi

echo "starting edgeplane daemon at ${SHIM_HOST}:${SHIM_PORT} (base_url=${BASE_URL})"

args=(
  daemon
  --shim-host "$SHIM_HOST"
  --shim-port "$SHIM_PORT"
)
if [[ "$ENABLE_MATRIX" == "1" ]]; then
  args+=(--matrix-endpoint "$MATRIX_ENDPOINT")
else
  args+=(--disable-matrix)
fi
if [[ -n "$FANOUT_PORT" ]]; then
  args+=(--fanout-port "$FANOUT_PORT")
fi

EP_BASE_URL="$BASE_URL" EP_TOKEN="$TOKEN" exec edgeplane "${args[@]}"
