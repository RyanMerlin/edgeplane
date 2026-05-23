#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASE_URL="${MC_BASE_URL:-http://localhost:8008}"
TOKEN="${MC_TOKEN:-}"
RUN_ID="${MC_COLLAB_RUN_ID:-$(date +%Y%m%d%H%M%S)}"
OUT_ROOT="${MC_COLLAB_OUT_ROOT:-$ROOT_DIR/artifacts/collab}"
SCENARIO_FILE="${MC_COLLAB_SCENARIO_FILE:-$ROOT_DIR/scripts/pressure-scenarios/reliability-trio.json}"
DURATION_SEC="${MC_COLLAB_DURATION_SEC:-600}"
POLL_SEC="${MC_COLLAB_POLL_SEC:-5}"
STACK_PROFILE="${MC_STACK_PROFILE:-full}"
DOMAIN_ID="${MC_COLLAB_DOMAIN_ID:-}"
MISSION_ID="${MC_COLLAB_MISSION_ID:-}"
ACTOR="${MC_COLLAB_ACTOR:-token-client}"

if [[ -z "$TOKEN" ]]; then
  echo "MC_TOKEN is required" >&2
  exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required" >&2
  exit 1
fi
if [[ ! -f "$SCENARIO_FILE" ]]; then
  echo "MC_COLLAB_SCENARIO_FILE not found: $SCENARIO_FILE" >&2
  exit 2
fi

mkdir -p "$OUT_ROOT"
RUN_DIR="$OUT_ROOT/$RUN_ID"
mkdir -p "$RUN_DIR"

http_post() {
  local path="$1"
  local data="$2"
  curl -fsS \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    -X POST "${BASE_URL}${path}" \
    -d "$data"
}

mcp_call() {
  local tool="$1"
  local args_json="$2"
  http_post "/mcp/call" "{\"tool\":\"${tool}\",\"args\":${args_json}}"
}

create_seed_if_needed() {
  if [[ -n "$DOMAIN_ID" && -n "$MISSION_ID" ]]; then
    return 0
  fi

  local scenario_name domain_name mission_name create_domain_resp create_mission_resp
  scenario_name="$(jq -r '.name // "reliability-trio"' "$SCENARIO_FILE")"
  domain_name="missioncontrol-improves-missioncontrol-${scenario_name}-${RUN_ID}"
  mission_name="collab-${scenario_name}-${RUN_ID}"

  create_domain_resp="$(mcp_call create_domain "$(jq -cn --arg name "$domain_name" --arg owners "$ACTOR" '{name:$name,owners:$owners,description:"Live collaboration domain"}')")"
  if [[ "$(jq -r '.ok' <<<"$create_domain_resp")" != "true" ]]; then
    echo "create_domain failed: $create_domain_resp" >&2
    exit 1
  fi
  DOMAIN_ID="$(jq -r '.result.id' <<<"$create_domain_resp")"

  create_mission_resp="$(mcp_call create_mission "$(jq -cn --arg domain_id "$DOMAIN_ID" --arg name "$mission_name" --arg owners "$ACTOR" '{domain_id:$domain_id,name:$name,owners:$owners,description:"Live collaboration workstream"}')")"
  if [[ "$(jq -r '.ok' <<<"$create_mission_resp")" != "true" ]]; then
    echo "create_mission failed: $create_mission_resp" >&2
    exit 1
  fi
  MISSION_ID="$(jq -r '.result.id' <<<"$create_mission_resp")"

  while IFS=$'\t' read -r title description; do
    mcp_call create_task "$(jq -cn --arg mission_id "$MISSION_ID" --arg title "$title" --arg owner "$ACTOR" --arg description "$description" '{mission_id:$mission_id,title:$title,description:$description,owner:$owner}')" >/dev/null
  done < <(jq -r '.tasks[] | [.title, (.description // "")] | @tsv' "$SCENARIO_FILE")
}

create_seed_if_needed

if [[ "$STACK_PROFILE" == "full" ]]; then
  if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    if [[ "$(docker inspect --format '{{.State.Running}}' missioncontrol-postgres 2>/dev/null || true)" != "true" ]]; then
      echo "full profile requires running postgres service; run MC_STACK_PROFILE=full bash scripts/dev-up.sh" >&2
      exit 1
    fi
    if ! docker exec missioncontrol-postgres psql -U "${POSTGRES_USER:-missioncontrol}" -d "${POSTGRES_DB:-missioncontrol}" -tAc \
      "SELECT extname FROM pg_extension WHERE extname='vector';" | rg -q "vector"; then
      echo "full profile preflight failed: pgvector extension is not enabled/reachable" >&2
      exit 1
    fi
  else
    echo "warning: docker socket not accessible; skipping postgres/pgvector container assertions" >&2
  fi
else
  echo "warning: running non-gating quickstart profile (sqlite)" >&2
fi

echo "== MC collab driver =="
echo "run_id=$RUN_ID"
echo "base_url=$BASE_URL"
echo "stack_profile=$STACK_PROFILE"
echo "domain_id=$DOMAIN_ID"
echo "mission_id=$MISSION_ID"
echo "duration_sec=$DURATION_SEC poll_sec=$POLL_SEC"
echo "out_dir=$RUN_DIR"

cat > "$RUN_DIR/run-context.env" <<EOF
RUN_ID=${RUN_ID}
MC_BASE_URL=${BASE_URL}
MC_STACK_PROFILE=${STACK_PROFILE}
MC_COLLAB_DOMAIN_ID=${DOMAIN_ID}
MC_COLLAB_MISSION_ID=${MISSION_ID}
EOF
echo
echo "Other sessions can now collaborate on this mission by updating task status/description:"
echo "  1) Stabilize codex MCP handshake under concurrency"
echo "  2) Instrument pressure harness with codex stall diagnostics"
echo "  3) Draft operator runbook for agent-vs-playbook gates"
echo

start_ts="$(date +%s)"
end_ts=$((start_ts + DURATION_SEC))

initial_tasks="$(mcp_call list_tasks "$(jq -cn --arg mission_id "$MISSION_ID" '{mission_id:$mission_id}')")"
echo "$initial_tasks" > "$RUN_DIR/initial-tasks.json"

changes=0
sample_count=0
while [[ "$(date +%s)" -lt "$end_ts" ]]; do
  sample_count=$((sample_count + 1))
  current="$(mcp_call list_tasks "$(jq -cn --arg mission_id "$MISSION_ID" '{mission_id:$mission_id}')")"
  echo "$current" > "$RUN_DIR/sample-${sample_count}.json"

  current_updates="$(jq -r '.result.tasks[]?.updated_at // ""' <<<"$current" | sort -u)"
  previous_updates="$(jq -r '.result.tasks[]?.updated_at // ""' "$RUN_DIR/initial-tasks.json" | sort -u)"
  if [[ "$current_updates" != "$previous_updates" ]]; then
    changes=$((changes + 1))
  fi
  sleep "$POLL_SEC"
done

final_tasks_file="$RUN_DIR/final-tasks.json"
mcp_call list_tasks "$(jq -cn --arg mission_id "$MISSION_ID" '{mission_id:$mission_id}')" > "$final_tasks_file"

summary_file="$RUN_DIR/summary.json"
jq -n \
  --arg run_id "$RUN_ID" \
  --arg base_url "$BASE_URL" \
  --arg domain_id "$DOMAIN_ID" \
  --arg mission_id "$MISSION_ID" \
  --arg scenario_file "$SCENARIO_FILE" \
  --argjson duration_sec "$DURATION_SEC" \
  --argjson poll_sec "$POLL_SEC" \
  --argjson samples "$sample_count" \
  --argjson observed_changes "$changes" \
  --slurpfile final "$final_tasks_file" \
  '{
    run_id:$run_id,
    base_url:$base_url,
    domain_id:$domain_id,
    mission_id:$mission_id,
    scenario_file:$scenario_file,
    duration_sec:$duration_sec,
    poll_sec:$poll_sec,
    samples:$samples,
    observed_changes:$observed_changes,
    task_count:(($final[0].result.tasks // []) | length),
    status_counts:(($final[0].result.tasks // [])
      | reduce .[] as $t ({}; .[$t.status] = ((.[$t.status] // 0) + 1))),
    tasks:(($final[0].result.tasks // []) | map({id,title,status,updated_at,owner}))
  }' > "$summary_file"

echo "== summary =="
cat "$summary_file"
