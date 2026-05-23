#!/usr/bin/env bash
# demo-mesh-goose.sh — end-to-end edgeplane mesh work loop demo using Goose as the runtime.
#
# Creates a 3-task A → B → C dependency chain, starts 3 Goose workers via
# `edgeplane run goose --mission`, then polls until all tasks reach "finished".
#
# Requirements:
#   - Backend running (EP_BASE_URL, default http://localhost:8008)
#   - `edgeplane` and `goose` binaries on PATH
#   - Goose reachable LiteLLM at EP_LITELLM_HOST (default http://litellm:4000)
#   - EP_TOKEN set or backend accepts unauthenticated requests
#
# Usage:
#   EP_BASE_URL=http://localhost:8008 EP_TOKEN=<token> ./scripts/demo-mesh-goose.sh

set -euo pipefail

BASE_URL="${EP_BASE_URL:-http://localhost:8008}"
TIMEOUT="${DEMO_TIMEOUT:-120}"
TOKEN="${EP_TOKEN:-}"
PROFILE="${EP_PROFILE:-default}"

cleanup_pids=()

cleanup() {
    for pid in "${cleanup_pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

log() { echo "[demo-goose] $*"; }

# ---- REST helper ----
ep_api() {
    local method="$1" path="$2" body="${3:-}"
    if [[ "$method" == "GET" ]]; then
        curl -sf \
            ${TOKEN:+-H "Authorization: Bearer $TOKEN"} \
            "${BASE_URL}${path}"
    else
        curl -sf -X POST \
            -H "Content-Type: application/json" \
            ${TOKEN:+-H "Authorization: Bearer $TOKEN"} \
            ${body:+-d "$body"} \
            "${BASE_URL}${path}"
    fi
}

task_status() {
    ep_api GET "/work/tasks/${1}" \
      | python3 -c "import sys,json; print(json.load(sys.stdin).get('status','?'))"
}

# ---- Preflight ----
command -v edgeplane >/dev/null || { echo "edgeplane binary not found on PATH"; exit 1; }
command -v goose >/dev/null || { echo "goose binary not found on PATH"; exit 1; }

# ---- 1. Create domain ----
log "Creating domain…"
DOMAIN=$(ep_api POST "/domains" \
    "{\"name\":\"demo-mesh-goose-$(date +%s)\",\"owners\":\"demo@example.com\"}")
DOMAIN_ID=$(echo "$DOMAIN" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
log "Domain: $DOMAIN_ID"

# ---- 2. Create mission ----
log "Creating mission…"
MISSION=$(ep_api POST "/domains/${DOMAIN_ID}/m" \
    "{\"name\":\"demo-m\",\"owners\":\"demo@example.com\"}")
MISSION_ID=$(echo "$MISSION" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
log "Mission: $MISSION_ID"

# ---- 3. Seed tasks A → B → C ----
log "Creating task A (no deps)…"
A_ID=$(ep_api POST "/work/missions/${MISSION_ID}/tasks" \
    "{\"title\":\"A - foundation\",\"description\":\"First task — write a haiku about distributed systems.\"}" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
log "Task A: $A_ID"

log "Creating task B (depends on A)…"
B_ID=$(ep_api POST "/work/missions/${MISSION_ID}/tasks" \
    "{\"title\":\"B - middle\",\"description\":\"Second task — write a limerick about message queues.\",\"depends_on\":[\"${A_ID}\"]}" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
log "Task B: $B_ID"

log "Creating task C (depends on B)…"
C_ID=$(ep_api POST "/work/missions/${MISSION_ID}/tasks" \
    "{\"title\":\"C - final\",\"description\":\"Third task — write one sentence summarising distributed systems in plain English.\",\"depends_on\":[\"${B_ID}\"]}" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
log "Task C: $C_ID"

log "Initial state: A=$(task_status $A_ID)  B=$(task_status $B_ID)  C=$(task_status $C_ID)"

# ---- 4. Start 3 Goose workers ----
log "Starting 3 Goose workers (edgeplane run goose --domain ${DOMAIN_ID})…"
for i in 1 2 3; do
    EP_BASE_URL="$BASE_URL" \
    EP_TOKEN="$TOKEN" \
    edgeplane run goose --domain "$DOMAIN_ID" -p "$PROFILE" \
        > "/tmp/demo-goose-worker-${i}.log" 2>&1 &
    cleanup_pids+=($!)
    log "Worker $i PID ${cleanup_pids[-1]}"
done

# ---- 5. Poll until all finished ----
log "Watching for completion (timeout ${TIMEOUT}s)…"
START=$SECONDS
DONE=0

while (( SECONDS - START < TIMEOUT )); do
    A_S=$(task_status "$A_ID" 2>/dev/null || echo "?")
    B_S=$(task_status "$B_ID" 2>/dev/null || echo "?")
    C_S=$(task_status "$C_ID" 2>/dev/null || echo "?")
    log "  A=$A_S  B=$B_S  C=$C_S  ($(( SECONDS - START ))s elapsed)"

    if [[ "$A_S" == "finished" && "$B_S" == "finished" && "$C_S" == "finished" ]]; then
        DONE=1
        break
    fi
    sleep 5
done

# ---- 6. Print worker logs ----
log "--- worker logs ---"
for i in 1 2 3; do
    f="/tmp/demo-goose-worker-${i}.log"
    [[ -f "$f" ]] && sed "s/^/  [w$i] /" "$f"
done

if [[ $DONE -eq 1 ]]; then
    log "SUCCESS: A → B → C all finished in $(( SECONDS - START ))s"
    exit 0
else
    log "TIMEOUT after ${TIMEOUT}s — A=$A_S  B=$B_S  C=$C_S"
    exit 1
fi
