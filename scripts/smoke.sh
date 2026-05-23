#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PROFILE="both"
KEEP_UP="false"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile)
      PROFILE="${2:-both}"
      shift 2
      ;;
    --keep-up)
      KEEP_UP="true"
      shift
      ;;
    *)
      echo "Unknown argument: $1" >&2
      echo "Usage: $0 [--profile quickstart|full|both] [--keep-up]" >&2
      exit 2
      ;;
  esac
done

profiles=()
case "$PROFILE" in
  quickstart) profiles=("quickstart") ;;
  full) profiles=("full") ;;
  both) profiles=("quickstart" "full") ;;
  *)
    echo "Invalid profile: $PROFILE" >&2
    exit 2
    ;;
esac

wait_for_api() {
  local tries=60
  while [[ $tries -gt 0 ]]; do
    if curl -fsS "http://localhost:8008/" >/dev/null 2>&1; then
      return 0
    fi
    tries=$((tries - 1))
    sleep 2
  done
  return 1
}

api_get() {
  local url="$1"
  if [[ -n "${EP_TOKEN:-}" ]]; then
    curl -fsS -H "Authorization: Bearer ${EP_TOKEN}" "$url" >/dev/null
  else
    curl -fsS "$url" >/dev/null
  fi
}

expect_unauthorized() {
  local url="$1"
  local status
  status="$(curl -sS -o /dev/null -w "%{http_code}" "$url")"
  if [[ "$status" != "401" && "$status" != "403" ]]; then
    echo "[smoke] expected unauthorized response from $url, got $status" >&2
    return 1
  fi
}

check_protected_endpoint() {
  local url="$1"
  if [[ -n "${EP_TOKEN:-}" ]]; then
    api_get "$url"
  else
    expect_unauthorized "$url"
  fi
}

run_profile() {
  local profile="$1"
  local compose_args=()
  if [[ "$profile" == "quickstart" ]]; then
    compose_args=(-f docker-compose.quickstart.yml)
  fi

  echo "[smoke] starting profile: $profile"
  docker compose "${compose_args[@]}" up -d --build

  if ! wait_for_api; then
    echo "[smoke] API did not become ready for profile $profile" >&2
    docker compose "${compose_args[@]}" logs --tail 200 || true
    return 1
  fi

  curl -fsS "http://localhost:8008/api/openapi.json" >/dev/null
  # Ensure the SvelteKit UI build was produced (`cd web && npm run build`)
  curl -fsS "http://localhost:8008/ui/" >/dev/null
  curl -fsS "http://localhost:8008/agent-onboarding.json" >/dev/null
  check_protected_endpoint "http://localhost:8008/schema-pack"
  check_protected_endpoint "http://localhost:8008/explorer/tree"
  check_protected_endpoint "http://localhost:8008/missions"

  if [[ "$profile" == "full" ]]; then
    if ! docker exec edgeplane-postgres psql -U edgeplane -d edgeplane -tAc \
      "SELECT extname FROM pg_extension WHERE extname='vector';" | grep -q "vector"; then
      echo "[smoke] pgvector extension is not enabled in full profile" >&2
      return 1
    fi
  fi

  echo "[smoke] profile passed: $profile"
  if [[ "$KEEP_UP" != "true" ]]; then
    docker compose "${compose_args[@]}" down
  fi
}

for p in "${profiles[@]}"; do
  run_profile "$p"
done

echo "[smoke] completed"
