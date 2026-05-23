#!/usr/bin/env bash
# e2e-test.sh — canonical three-runtime demo / acceptance test for edgeplaned v1
#
# Exercises the full acceptance criteria:
#   1. edgeplane daemon up (daemon starts)
#   2. Runtime smoke-tests (claude-code, codex, gemini)
#   3. Enroll three agents in one mission
#   4. Submit a three-task DAG to a mission (A→B→C, different runtimes)
#   5. Watch progress until all tasks finish
#   6. Verify all tasks reached status=finished
#   7. Inter-agent message flow (check message count > 0 after tasks complete)
#
# Prerequisites:
#   - edgeplane and edgeplaned installed (run install.sh first)
#   - EP_BACKEND_URL and EP_TOKEN set, or a ~/.edgeplane/edgeplaned.yaml present
#   - A mission and mission already created in Edgeplane (set IDs below)
#   - claude, codex, and gemini CLIs installed
#
# Usage:
#   DOMAIN_ID=<uuid> MISSION_ID=<uuid> bash e2e-test.sh

set -euo pipefail

: "${DOMAIN_ID:?Set DOMAIN_ID to the target mission UUID}"
: "${MISSION_ID:?Set MISSION_ID to the target mission UUID}"
: "${EP_BACKEND_URL:=http://localhost:8000}"
: "${EP_TOKEN:=}"

PASS=0
FAIL=0

green()  { printf '\033[0;32m✓  %s\033[0m\n' "$*"; ((PASS++)); }
red()    { printf '\033[0;31m✗  %s\033[0m\n' "$*"; ((FAIL++)); }
info()   { printf '   %s\n' "$*"; }
header() { printf '\n\033[1;34m── %s ──\033[0m\n' "$*"; }

mc_cmd() { edgeplane --backend-url "$EP_BACKEND_URL" ${EP_TOKEN:+--token "$EP_TOKEN"} "$@"; }
api()    { curl -sf -H "Authorization: Bearer $EP_TOKEN" "$EP_BACKEND_URL$1"; }

# ---------------------------------------------------------------------------
# Step 1: Daemon health
# ---------------------------------------------------------------------------
header "Step 1: Daemon health"

mc_cmd mesh up --yes 2>/dev/null || true
sleep 1

if mc_cmd mesh status 2>&1 | grep -q "running"; then
    green "daemon is running"
else
    red "daemon not running after edgeplane daemon up"
fi

# ---------------------------------------------------------------------------
# Step 2: Runtime smoke-tests
# ---------------------------------------------------------------------------
header "Step 2: Runtime smoke-tests"

for rt in claude codex gemini; do
    if command -v "$rt" >/dev/null 2>&1; then
        green "$rt CLI found"
    else
        red "$rt CLI not found — install with: edgeplane daemon runtime install ${rt/claude/claude-code}"
    fi
done

# ---------------------------------------------------------------------------
# Step 3: Enroll three agents
# ---------------------------------------------------------------------------
header "Step 3: Enroll agents in mission $DOMAIN_ID"

enroll_agent() {
    local runtime="$1"
    local result
    result=$(mc_cmd mesh agent enroll --mission "$DOMAIN_ID" --runtime "$runtime" 2>&1)
    local agent_id
    agent_id=$(echo "$result" | grep -oE '[0-9a-f-]{36}' | head -1)
    if [[ -n "$agent_id" ]]; then
        green "enrolled $runtime agent $agent_id"
        echo "$agent_id"
    else
        red "failed to enroll $runtime agent"
        echo ""
    fi
}

CLAUDE_AGENT=$(enroll_agent claude_code)
CODEX_AGENT=$(enroll_agent codex)
GEMINI_AGENT=$(enroll_agent gemini)

info "claude-code agent: ${CLAUDE_AGENT:-<failed>}"
info "codex agent:       ${CODEX_AGENT:-<failed>}"
info "gemini agent:      ${GEMINI_AGENT:-<failed>}"

# ---------------------------------------------------------------------------
# Step 4: Submit a three-task DAG
# ---------------------------------------------------------------------------
header "Step 4: Submit three-task DAG to mission $MISSION_ID"

# Task A: claude-code — design doc (no deps)
TASK_A=$(mc_cmd mesh task run "$MISSION_ID" \
    --title "Write a design doc for a simple key-value store" \
    --description "Produce a concise design doc (design.md) for an in-memory key-value store in Rust. Cover: API surface, data structure choice, error handling. Keep it under 200 lines." \
    --runtime claude_code \
    --claim-policy first_claim \
    --priority 10 \
    2>&1 | grep -oE '[0-9a-f-]{36}' | head -1)

if [[ -n "$TASK_A" ]]; then
    green "Task A created: $TASK_A"
else
    red "Failed to create Task A"; TASK_A=""
fi

# Task B: codex — implement (depends on A)
if [[ -n "$TASK_A" ]]; then
    TASK_B=$(mc_cmd mesh task run "$MISSION_ID" \
        --title "Implement the key-value store from the design doc" \
        --description "Read design.md and implement the described key-value store in src/kvstore.rs. Include unit tests." \
        --runtime codex \
        --depends-on "$TASK_A" \
        --claim-policy first_claim \
        --priority 5 \
        2>&1 | grep -oE '[0-9a-f-]{36}' | head -1)
    [[ -n "$TASK_B" ]] && green "Task B created: $TASK_B (depends on A)" || red "Failed to create Task B"
else
    TASK_B=""
    red "Skipping Task B (Task A failed)"
fi

# Task C: gemini — review and summarise (depends on B)
if [[ -n "$TASK_B" ]]; then
    TASK_C=$(mc_cmd mesh task run "$MISSION_ID" \
        --title "Review the implementation and write a summary" \
        --description "Review src/kvstore.rs for correctness, style, and completeness. Post a mission message summarising your findings and any recommended changes." \
        --runtime gemini \
        --depends-on "$TASK_B" \
        --claim-policy first_claim \
        --priority 0 \
        2>&1 | grep -oE '[0-9a-f-]{36}' | head -1)
    [[ -n "$TASK_C" ]] && green "Task C created: $TASK_C (depends on B)" || red "Failed to create Task C"
else
    TASK_C=""
    red "Skipping Task C (Task B failed)"
fi

# ---------------------------------------------------------------------------
# Step 5: Watch until all tasks finish (timeout: 10 min)
# ---------------------------------------------------------------------------
header "Step 5: Watching mission until all tasks finish (timeout: 10m)"

DEADLINE=$(( $(date +%s) + 600 ))
ALL_DONE=false

while [[ $(date +%s) -lt $DEADLINE ]]; do
    task_statuses=$(api "/work/missions/$MISSION_ID/tasks" 2>/dev/null \
        | python3 -c "import sys,json; tasks=json.load(sys.stdin); print(' '.join(t['status'] for t in tasks))" 2>/dev/null || echo "error")

    info "task statuses: $task_statuses"

    # Count finished vs total
    total=$(echo "$task_statuses" | wc -w)
    finished=$(echo "$task_statuses" | tr ' ' '\n' | grep -c "^finished$" || true)
    failed=$(echo "$task_statuses" | tr ' ' '\n' | grep -c "^failed$" || true)

    if [[ "$total" -gt 0 && "$finished" -eq "$total" ]]; then
        ALL_DONE=true
        break
    fi
    if [[ "$failed" -gt 0 ]]; then
        info "One or more tasks failed — stopping early."
        break
    fi
    sleep 10
done

# ---------------------------------------------------------------------------
# Step 6: Verify final task statuses
# ---------------------------------------------------------------------------
header "Step 6: Verify task statuses"

check_task() {
    local task_id="$1"
    local label="$2"
    if [[ -z "$task_id" ]]; then
        red "$label: task was not created"
        return
    fi
    local status
    status=$(api "/work/tasks/$task_id" 2>/dev/null \
        | python3 -c "import sys,json; print(json.load(sys.stdin)['status'])" 2>/dev/null || echo "unknown")
    if [[ "$status" == "finished" ]]; then
        green "$label ($task_id): finished"
    else
        red "$label ($task_id): $status (expected finished)"
    fi
}

check_task "$TASK_A" "Task A (claude-code)"
check_task "$TASK_B" "Task B (codex)"
check_task "$TASK_C" "Task C (gemini)"

# ---------------------------------------------------------------------------
# Step 7: Inter-agent messages
# ---------------------------------------------------------------------------
header "Step 7: Inter-agent messages"

msg_count=$(api "/work/missions/$MISSION_ID/messages" 2>/dev/null \
    | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "0")

if [[ "$msg_count" -gt 0 ]]; then
    green "$msg_count message(s) in mission stream"
else
    red "No messages in mission stream (expected at least one from gemini review)"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
header "Result"
echo ""
if [[ $FAIL -eq 0 ]]; then
    printf '\033[0;32mAll %d checks passed — v1 acceptance demo complete.\033[0m\n' "$PASS"
else
    printf '\033[0;31m%d/%d checks failed.\033[0m\n' "$FAIL" "$((PASS + FAIL))"
fi
echo ""
exit $FAIL
