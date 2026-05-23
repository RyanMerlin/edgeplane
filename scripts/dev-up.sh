#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ ! -f .env ]]; then
  cp .env.example .env
fi

WIPE_DATA="${EP_DEV_WIPE_DATA:-0}"
STACK_PROFILE="${EP_STACK_PROFILE:-full}"

compose_args=()
case "$STACK_PROFILE" in
  full)
    compose_args=()
    ;;
  quickstart)
    compose_args=(-f docker-compose.quickstart.yml)
    ;;
  *)
    echo "Invalid EP_STACK_PROFILE=${STACK_PROFILE} (expected: full|quickstart)" >&2
    exit 2
    ;;
esac

if [[ "$WIPE_DATA" == "1" ]]; then
  docker compose "${compose_args[@]}" down --remove-orphans --volumes
else
  docker compose "${compose_args[@]}" down --remove-orphans
fi
docker compose "${compose_args[@]}" up -d --build --remove-orphans

# Cleanup rustfs-init container after 5 minutes so it does not linger.
nohup bash -lc "sleep 300 && docker rm -f edgeplane-rustfs-init >/dev/null 2>&1 || true" >/dev/null 2>&1 &

# Keep only the 3 most recent dev-tagged builds; prune the rest to prevent disk bloat.
_old_dev_images=$(docker images --format "{{.Repository}}:{{.Tag}}" \
  | grep -E "edgeplane.*:dev-[0-9]" | sort -r | tail -n +4)
if [[ -n "$_old_dev_images" ]]; then
  echo "Pruning old dev images..."
  echo "$_old_dev_images" | xargs docker rmi --force 2>/dev/null || true
fi

echo "Edgeplane API: http://localhost:8008 (profile=${STACK_PROFILE})"
