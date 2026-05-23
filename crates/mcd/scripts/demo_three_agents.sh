#!/usr/bin/env bash
# demo_three_agents.sh — three-agent dependency chain demo
#
# Creates a domain + mission with three tasks in a linear dependency chain:
#
#   T1 (claude_code) → T2 (codex) → T3 (gemini)
#
# Verifies that T2 and T3 stay "pending" until their predecessor is finished,
# then simulates each agent claiming and completing its task via the REST API.
# Uses the mc-controlplane API directly — no running daemons required.
#
# Prerequisites:
#   export MC_BASE_URL=http://missioncontrol:8008
#   export MC_TOKEN=<your-token>

set -euo pipefail

: "${MC_BASE_URL:?Set MC_BASE_URL to the mc-controlplane base URL}"
: "${MC_TOKEN:?Set MC_TOKEN to a valid bearer token}"

BASE="${MC_BASE_URL%/}"
AUTH="Authorization: Bearer $MC_TOKEN"
RUN_ID="$(date +%s)"
PASS=0
FAIL=0

# ── Helpers ───────────────────────────────────────────────────────────────────

mc_api() {
    local method="$1" path="$2"
    shift 2
    curl -sf -X "$method" "$BASE$path" \
        -H "Content-Type: application/json" \
        -H "$AUTH" \
        "$@"
}

assert_status() {
    local task_id="$1" expected="$2"
    local actual
    actual=$(mc_api GET "/work/tasks/$task_id" | jq -r '.status')
    if [[ "$actual" == "$expected" ]]; then
        echo "  [PASS] $task_id is $expected"
        PASS=$((PASS + 1))
    else
        echo "  [FAIL] $task_id: expected $expected, got $actual" >&2
        FAIL=$((FAIL + 1))
    fi
}

header() { echo; echo "══ $* ══"; }

# ── Setup ─────────────────────────────────────────────────────────────────────

header "Creating domain"
DOMAIN=$(mc_api POST /domains -d "{
    \"name\":        \"three-agent-demo-${RUN_ID}\",
    \"description\": \"Dependency chain demo: design to implement to test\",
    \"owners\":      \"demo\",
    \"visibility\":  \"private\",
    \"status\":      \"active\"
}")
DOMAIN_ID=$(echo "$DOMAIN" | jq -r '.id')
echo "  domain: $DOMAIN_ID"

header "Creating mission"
MISSION=$(mc_api POST "/domains/$DOMAIN_ID/m" -d "{
    \"name\":    \"demo-mission\",
    \"owners\":  \"demo\",
    \"status\":  \"active\"
}")
MISSION_ID=$(echo "$MISSION" | jq -r '.id')
echo "  mission: $MISSION_ID"

# ── Task creation ─────────────────────────────────────────────────────────────

header "Creating tasks"

T1=$(mc_api POST "/work/missions/$MISSION_ID/tasks" -d '{
    "title":                "Design spec",
    "description":          "Produce a spec document for the feature",
    "claim_policy":         "first_claim",
    "required_capabilities": ["claude_code"],
    "produces":             {"spec_doc": {}},
    "depends_on":           [],
    "priority":             10
}')
T1_ID=$(echo "$T1" | jq -r '.id')
T1_STATUS=$(echo "$T1" | jq -r '.status')
echo "  T1 ($T1_ID): $T1_STATUS"

T2=$(mc_api POST "/work/missions/$MISSION_ID/tasks" -d "{
    \"title\":                \"Implement feature\",
    \"description\":          \"Write the implementation from the spec\",
    \"claim_policy\":         \"first_claim\",
    \"required_capabilities\": [\"codex\"],
    \"consumes\":             {\"spec_doc\": {}},
    \"produces\":             {\"impl_patch\": {}},
    \"depends_on\":           [\"$T1_ID\"],
    \"priority\":             10
}")
T2_ID=$(echo "$T2" | jq -r '.id')
T2_STATUS=$(echo "$T2" | jq -r '.status')
echo "  T2 ($T2_ID): $T2_STATUS"

T3=$(mc_api POST "/work/missions/$MISSION_ID/tasks" -d "{
    \"title\":                \"Test implementation\",
    \"description\":          \"Run tests against the implementation\",
    \"claim_policy\":         \"first_claim\",
    \"required_capabilities\": [\"gemini\"],
    \"consumes\":             {\"impl_patch\": {}},
    \"depends_on\":           [\"$T2_ID\"],
    \"priority\":             10
}")
T3_ID=$(echo "$T3" | jq -r '.id')
T3_STATUS=$(echo "$T3" | jq -r '.status')
echo "  T3 ($T3_ID): $T3_STATUS"

# ── Verify initial dependency blocking ───────────────────────────────────────

header "Verifying dependency blocking"
if [[ "$T1_STATUS" != "ready" ]]; then
    echo "  [FAIL] T1 should be ready immediately (no deps), got: $T1_STATUS" >&2
    FAIL=$((FAIL + 1))
else
    echo "  [PASS] T1 starts ready (no dependencies)"
    PASS=$((PASS + 1))
fi
assert_status "$T2_ID" "pending"
assert_status "$T3_ID" "pending"

# ── Agent A: claim and complete T1 ───────────────────────────────────────────

header "Agent A (claude_code) completes T1"
CLAIM1=$(mc_api POST "/work/tasks/$T1_ID/claim" -d '{}')
LEASE1=$(echo "$CLAIM1" | jq -r '.claim_lease_id // ""')
echo "  claimed T1 (lease: ${LEASE1:-none})"

mc_api POST "/work/tasks/$T1_ID/complete" -d "{\"claim_lease_id\":\"$LEASE1\"}" > /dev/null
echo "  completed T1"

header "Verifying T2 unblocked"
assert_status "$T1_ID" "finished"
assert_status "$T2_ID" "ready"
assert_status "$T3_ID" "pending"

# ── Agent B: claim and complete T2 ───────────────────────────────────────────

header "Agent B (codex) completes T2"
CLAIM2=$(mc_api POST "/work/tasks/$T2_ID/claim" -d '{}')
LEASE2=$(echo "$CLAIM2" | jq -r '.claim_lease_id // ""')
echo "  claimed T2 (lease: ${LEASE2:-none})"

mc_api POST "/work/tasks/$T2_ID/complete" -d "{\"claim_lease_id\":\"$LEASE2\"}" > /dev/null
echo "  completed T2"

header "Verifying T3 unblocked"
assert_status "$T2_ID" "finished"
assert_status "$T3_ID" "ready"

# ── Agent C: claim and complete T3 ───────────────────────────────────────────

header "Agent C (gemini) completes T3"
CLAIM3=$(mc_api POST "/work/tasks/$T3_ID/claim" -d '{}')
LEASE3=$(echo "$CLAIM3" | jq -r '.claim_lease_id // ""')
echo "  claimed T3 (lease: ${LEASE3:-none})"

mc_api POST "/work/tasks/$T3_ID/complete" -d "{\"claim_lease_id\":\"$LEASE3\"}" > /dev/null
echo "  completed T3"

assert_status "$T3_ID" "finished"

# ── Final graph ───────────────────────────────────────────────────────────────

header "Final task graph"
mc_api GET "/work/missions/$MISSION_ID/graph" | jq '{
    domain:   "'"$DOMAIN_ID"'",
    mission:  "'"$MISSION_ID"'",
    nodes:    .nodes
}'

# ── Result ────────────────────────────────────────────────────────────────────

echo
echo "══ Results ══"
echo "  Passed: $PASS"
echo "  Failed: $FAIL"
echo
if (( FAIL > 0 )); then
    echo "DEMO FAILED — see failures above" >&2
    exit 1
fi
echo "Demo complete — dependency chain verified end-to-end"
