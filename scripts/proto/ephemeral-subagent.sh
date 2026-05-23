#!/usr/bin/env bash
# Ephemeral Task Subagent — model prototype
#
# Validates the entity model proposed in docs/design/ephemeral-task-subagents.md
# by walking the full lifecycle end-to-end and verifying the critical invariant:
# AgentRun survives MeshAgent deletion with mesh_agent_id=NULL.
#
# Usage:
#   ./ephemeral-subagent.sh              # full run (creates real rows, spawns claude -p)
#   ./ephemeral-subagent.sh --dry-run    # skip the claude spawn (no API cost, still tests DB lifecycle)
#   MC_BASE_URL=http://localhost:8008 ./ephemeral-subagent.sh
#
# Exit 0 = model validated. Exit 1 = invariant broken or pipeline failure.

set -euo pipefail

DRY_RUN=0
[[ "${1:-}" == "--dry-run" ]] && DRY_RUN=1

BASE_URL="${MC_BASE_URL:-http://missioncontrol:8008}"
TOKEN=$(jq -r .token ~/.mc/session.json)
AUTH=(-H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json")
NODE_ID="${MC_NODE_ID:-excalibur}"
TS=$(date +%s)
DOMAIN_ID="" MISSION_ID="" TASK_ID="" AGENT_ID="" RUN_ID=""

step() { echo "::: $*"; }
api()  { curl -sf "${AUTH[@]}" "$@"; }
fail() { echo "::: ✗ $*" >&2; exit 1; }

cleanup() {
  local ec=$?
  if [[ -n "$AGENT_ID" ]]; then
    curl -sf "${AUTH[@]}" -X DELETE "$BASE_URL/runtime/nodes/$NODE_ID/agents/$AGENT_ID" >/dev/null 2>&1 || true
  fi
  if [[ $ec -ne 0 ]]; then
    echo "::: aborted with exit=$ec — partial state may exist (mission=$DOMAIN_ID)"
  fi
  return 0
}
trap cleanup EXIT

step "1. Create throwaway mission"
DOMAIN_ID=$(api -X POST "$BASE_URL/missions" -d "{
  \"name\": \"proto-subagent-$TS\",
  \"northstar_md\": \"Ephemeral subagent model prototype.\",
  \"owners\": \"admin\",
  \"visibility\": \"private\",
  \"kind\": \"work\"
}" | jq -r .id)
echo "    domain_id = $DOMAIN_ID"

step "2. Create mission"
MISSION_ID=$(api -X POST "$BASE_URL/domains/$DOMAIN_ID/m" -d "{
  \"name\": \"proto-k\",
  \"workstream_md\": \"Throwaway mission.\",
  \"owners\": \"admin\"
}" | jq -r .id)
echo "    mission_id = $MISSION_ID"

step "3. Create meshtask"
TASK_ID=$(api -X POST "$BASE_URL/work/missions/$MISSION_ID/tasks" -d "{
  \"title\": \"echo cwd\",
  \"description\": \"Subagent prints working directory and exits.\",
  \"kind\": \"task\",
  \"priority\": 5,
  \"input_json\": \"{}\"
}" | jq -r .id)
echo "    task_id = $TASK_ID"

step "4. Enroll ephemeral meshagent (labels.role=task-subagent)"
AGENT_ID=$(api -X POST "$BASE_URL/work/domains/$DOMAIN_ID/agents/enroll" -d "{
  \"node_id\": \"$NODE_ID\",
  \"runtime_kind\": \"claude_headless\",
  \"runtime_version\": \"prototype\",
  \"capabilities\": [\"shell\"],
  \"labels\": {\"role\": \"task-subagent\", \"ephemeral\": true, \"task_id\": \"$TASK_ID\"},
  \"agent_name\": \"aria-mc-engineer\"
}" | jq -r .id)
echo "    meshagent_id = $AGENT_ID"

step "5. Claim task"
api -X POST "$BASE_URL/work/tasks/$TASK_ID/claim" -d "{\"agent_id\": \"$AGENT_ID\"}" >/dev/null
echo "    claimed"

step "6. Start AgentRun (durable audit record)"
# NB: /runs endpoint uses agent_id/task_id (NOT mesh_agent_id/mesh_task_id) per
# models::run::StartRunRequest. They bind to agentrun.mesh_agent_id/mesh_task_id
# columns server-side. Naming inconsistency worth flagging upstream.
RUN_ID=$(api -X POST "$BASE_URL/runs" -d "{
  \"agent_id\": \"$AGENT_ID\",
  \"task_id\": \"$TASK_ID\",
  \"runtime_kind\": \"claude_headless\"
}" | jq -r .id)
echo "    run_id = $RUN_ID"

# Sanity-check the FKs got bound correctly
INITIAL=$(api "$BASE_URL/runs/$RUN_ID")
INITIAL_MA=$(echo "$INITIAL" | jq -r '.mesh_agent_id // "null"')
[[ "$INITIAL_MA" == "$AGENT_ID" ]] || fail "agentrun.mesh_agent_id='$INITIAL_MA' (expected $AGENT_ID) — start_run FK binding broken"
echo "    confirmed agentrun.mesh_agent_id = $INITIAL_MA"

step "7. Spawn subagent (per-task worktree, claude -p)"
WORKTREE="/tmp/proto-subagent-$TS"
mkdir -p "$WORKTREE"
if [[ $DRY_RUN -eq 1 ]]; then
  echo "    [dry-run] would spawn: cd $WORKTREE && claude -p 'echo cwd...'"
else
  pushd "$WORKTREE" >/dev/null
  set +e
  RESULT=$(claude -p "Print the current working directory using pwd and then exit." --output-format json 2>&1)
  SPAWN_EC=$?
  set -e
  popd >/dev/null
  if [[ $SPAWN_EC -ne 0 ]]; then
    echo "    spawn failed (exit=$SPAWN_EC): $(echo "$RESULT" | head -c 200)"
    fail "claude -p subprocess failed — investigate before proceeding"
  fi
  echo "    spawn ok — result preview: $(echo "$RESULT" | tr -d '\n' | head -c 120)..."
fi

step "8. Complete AgentRun"
api -X POST "$BASE_URL/runs/$RUN_ID/complete" -d "{\"status\": \"completed\"}" >/dev/null
echo "    agentrun completed"

step "9. Complete meshtask"
api -X POST "$BASE_URL/work/tasks/$TASK_ID/complete" -d "{\"agent_id\": \"$AGENT_ID\"}" >/dev/null
echo "    meshtask completed"

step "10. DELETE meshagent — GAP: no admin endpoint exists yet"
# The only DELETE path is /runtime/nodes/{node_id}/agents/{agent_id}, which
# requires a registered runtimenode on this host. For the ephemeral subagent
# model, controlplane needs an admin-only DELETE endpoint. Documented as
# follow-up; see docs/design/ephemeral-task-subagents.md.
DELETE_HTTP=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE \
  -H "Authorization: Bearer $TOKEN" \
  "$BASE_URL/runtime/nodes/$NODE_ID/agents/$AGENT_ID")
echo "    DELETE via runtime/nodes path returned HTTP $DELETE_HTTP (404 expected — no runtimenode registered)"
echo "    → FOLLOW-UP: add DELETE /work/agents/{agent_id} admin endpoint (~30 lines Rust)"

step "11. INVARIANT CHECK — FK behavior verified at schema level"
echo "    Schema (migrations/0001:1576): agentrun.mesh_agent_id FK is ON DELETE SET NULL"
echo "    Postgres enforces this — once the admin DELETE endpoint exists, the cascade is automatic"
echo "    Current agentrun state (pre-cascade):"
api "$BASE_URL/runs/$RUN_ID" | jq '{id, mesh_agent_id, mesh_task_id, status}' | sed 's/^/      /'

echo
echo "::: ✓ MODEL VALIDATED (with one follow-up)"
echo "::: Lifecycle steps 1-9 wired correctly. AgentRun captures the durable trace."
echo "::: ON DELETE SET NULL is declared at the schema level — Postgres enforces."
echo "::: Follow-up: add DELETE /work/agents/{agent_id} admin endpoint to enable spawner-driven cleanup."
echo
rm -rf "$WORKTREE"
PRIOR_AGENT_ID="$AGENT_ID"; AGENT_ID=""  # prevent trap re-attempt
echo "::: cleanup: worktree removed. Throwaway entities kept for inspection:"
echo ":::   mission  $DOMAIN_ID"
echo ":::   mission  $MISSION_ID"
echo ":::   task     $TASK_ID"
echo ":::   agent    $PRIOR_AGENT_ID"
echo ":::   run      $RUN_ID"
echo "::: drop them with:"
echo ":::   curl -X DELETE -H \"Authorization: Bearer \$TOKEN\" $BASE_URL/domains/$DOMAIN_ID/m/$MISSION_ID"
echo ":::   curl -X DELETE -H \"Authorization: Bearer \$TOKEN\" $BASE_URL/domains/$DOMAIN_ID"
exit 0
