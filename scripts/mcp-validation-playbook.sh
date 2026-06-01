#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${EP_BASE_URL:-http://localhost:8008}"
TOKEN="${EP_AGENT_TOKEN:-}"
ACTOR="${EP_PLAYBOOK_ACTOR:-token-client}"
RUN_ID="${EP_PLAYBOOK_RUN_ID:-$(date +%Y%m%d%H%M%S)}"
SCENARIO_FILE="${EP_PLAYBOOK_SCENARIO_FILE:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/scripts/pressure-scenarios/reliability-trio.json}"
SKIP_CLEANUP="${EP_PLAYBOOK_SKIP_CLEANUP:-0}"

if [[ -z "$TOKEN" ]]; then
  echo "EP_AGENT_TOKEN is required" >&2
  exit 2
fi
if [[ ! -f "$SCENARIO_FILE" ]]; then
  echo "EP_PLAYBOOK_SCENARIO_FILE not found: $SCENARIO_FILE" >&2
  exit 2
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

API_AUTH=(-H "Authorization: Bearer ${TOKEN}")
JSON_HDR=(-H "Content-Type: application/json")
HTTP_RETRIES="${EP_PLAYBOOK_HTTP_RETRIES:-4}"
HTTP_RETRY_SLEEP_SEC="${EP_PLAYBOOK_HTTP_RETRY_SLEEP_SEC:-0.5}"
HTTP_RETRY_MAX_SLEEP_SEC="${EP_PLAYBOOK_HTTP_RETRY_MAX_SLEEP_SEC:-5}"

http_request() {
  local method="$1"
  local url="$2"
  local data="${3:-}"
  local attempt=1
  local max_attempts="$HTTP_RETRIES"
  if [[ "$max_attempts" -lt 1 ]]; then
    max_attempts=1
  fi
  while true; do
    local response http_code body hdr_file retry_after next_sleep
    hdr_file="$(mktemp)"
    if [[ -n "$data" ]]; then
      response="$(curl -sS -D "$hdr_file" "${API_AUTH[@]}" "${JSON_HDR[@]}" -X "$method" "$url" -d "$data" -w $'\n%{http_code}')"
    else
      response="$(curl -sS -D "$hdr_file" "${API_AUTH[@]}" -X "$method" "$url" -w $'\n%{http_code}')"
    fi
    http_code="${response##*$'\n'}"
    body="${response%$'\n'*}"
    if [[ "$http_code" =~ ^2[0-9][0-9]$ ]]; then
      printf '%s' "$body"
      rm -f "$hdr_file"
      return 0
    fi
    if [[ "$http_code" == "429" || "$http_code" =~ ^5[0-9][0-9]$ ]]; then
      if [[ "$attempt" -lt "$max_attempts" ]]; then
        retry_after="$(awk 'BEGIN{IGNORECASE=1} /^retry-after:/ {gsub("\r","",$2); print $2; exit}' "$hdr_file" 2>/dev/null || true)"
        if [[ -n "$retry_after" && "$retry_after" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
          sleep "$retry_after"
        else
          next_sleep="$(awk "BEGIN{v=${HTTP_RETRY_SLEEP_SEC} * (2^(${attempt}-1)); if (v>${HTTP_RETRY_MAX_SLEEP_SEC}) v=${HTTP_RETRY_MAX_SLEEP_SEC}; printf \"%.3f\", v}")"
          sleep "$next_sleep"
        fi
        attempt=$((attempt + 1))
        rm -f "$hdr_file"
        continue
      fi
    fi
    echo "HTTP ${http_code}: ${url}" >&2
    if [[ -n "$body" ]]; then
      echo "$body" >&2
    fi
    rm -f "$hdr_file"
    return 22
  done
}

mcp_call() {
  local tool="$1"
  local args_json="$2"
  http_request "POST" "${BASE_URL}/mcp/call" "{\"tool\":\"${tool}\",\"args\":${args_json}}"
}

api_call() {
  local method="$1"
  local path="$2"
  local payload="${3:-}"
  http_request "$method" "${BASE_URL}${path}" "$payload"
}

assert_ok() {
  local label="$1"
  local resp="$2"
  local ok
  ok="$(jq -r '.ok // false' <<<"$resp")"
  if [[ "$ok" != "true" ]]; then
    echo "[FAIL] ${label}: $(jq -c '.' <<<"$resp")" >&2
    exit 1
  fi
  echo "[OK] ${label}"
}

domain_name="mcp-playbook-${RUN_ID}"
mission_name="mcp-playbook-mission-${RUN_ID}"
scenario_name="$(jq -r '.name // "reliability-trio"' "$SCENARIO_FILE")"
scenario_version="$(jq -r '.version // "1.0.0"' "$SCENARIO_FILE")"
expected_task_count="$(jq -r '.expected.task_count // 3' "$SCENARIO_FILE")"
domain_desc="$(jq -r '.domain.description // "MCP validation domain"' "$SCENARIO_FILE")"
mission_desc="$(jq -r '.mission.description // "MCP validation mission"' "$SCENARIO_FILE")"

echo "== MCP validation playbook run_id=${RUN_ID} base_url=${BASE_URL} scenario=${scenario_name} skip_cleanup=${SKIP_CLEANUP}"

create_domain_resp="$(mcp_call create_domain "$(jq -cn --arg name "$domain_name" --arg owners "$ACTOR" --arg description "$domain_desc" '{name:$name,owners:$owners,description:$description}')")"
assert_ok "create_domain" "$create_domain_resp"
domain_id="$(jq -r '.result.id' <<<"$create_domain_resp")"
echo "domain_id=${domain_id}"

create_mission_resp="$(mcp_call create_mission "$(jq -cn --arg domain_id "$domain_id" --arg name "$mission_name" --arg owners "$ACTOR" --arg description "$mission_desc" '{domain_id:$domain_id,name:$name,owners:$owners,description:$description}')")"
assert_ok "create_mission" "$create_mission_resp"
mission_id="$(jq -r '.result.id' <<<"$create_mission_resp")"
echo "mission_id=${mission_id}"

declare -a task_ids=()
while IFS=$'\t' read -r title description status; do
  create_task_resp="$(mcp_call create_task "$(jq -cn --arg mission_id "$mission_id" --arg title "$title" --arg owner "$ACTOR" --arg description "$description" '{mission_id:$mission_id,title:$title,description:$description,owner:$owner}')")"
  assert_ok "create_task:${title}" "$create_task_resp"
  tid="$(jq -r '.result.id' <<<"$create_task_resp")"
  task_ids+=("$tid")
  # Apply status if not default
  if [[ -n "$status" && "$status" != "todo" && "$status" != "null" ]]; then
    mcp_call update_task "$(jq -cn --arg task_id "$tid" --arg status "$status" '{task_id:$task_id,status:$status}')" >/dev/null
    echo "[OK] set_task_status:${title} → ${status}"
  fi
done < <(jq -r '.tasks[] | [.title, (.description // ""), (.status // "todo")] | @tsv' "$SCENARIO_FILE")
echo "task_ids=${task_ids[*]}"

list_tasks_resp="$(mcp_call list_tasks "$(jq -cn --arg mission_id "$mission_id" '{mission_id:$mission_id}')")"
assert_ok "list_tasks" "$list_tasks_resp"
actual_task_count="$(jq -r '.result.tasks | length' <<<"$list_tasks_resp")"
if [[ "$actual_task_count" -lt "$expected_task_count" ]]; then
  echo "[FAIL] scenario_assertion: expected >=${expected_task_count} tasks, got ${actual_task_count}" >&2
  exit 1
fi

update_task_resp="$(mcp_call update_task "$(jq -cn --arg task_id "${task_ids[0]}" '{task_id:$task_id,status:"in_progress"}')")"
assert_ok "update_task" "$update_task_resp"

# Create docs from scenario (falls back to a single default doc if none defined)
declare -a doc_ids=()
doc_count="$(jq '.docs | length // 0' "$SCENARIO_FILE")"
if [[ "$doc_count" -gt 0 ]]; then
  while IFS=$'\t' read -r title doc_type body; do
    create_doc_resp="$(mcp_call create_doc "$(jq -cn --arg mission_id "$mission_id" --arg title "$title" --arg doc_type "$doc_type" --arg body "$body" '{mission_id:$mission_id,title:$title,doc_type:$doc_type,body:$body,status:"draft"}')")"
    assert_ok "create_doc:${title}" "$create_doc_resp"
    doc_ids+=("$(jq -r '.result.id' <<<"$create_doc_resp")")
    echo "doc_id=${doc_ids[-1]} type=${doc_type}"
  done < <(jq -r '.docs[] | [.title, (.doc_type // "narrative"), (.body // "# doc")] | @tsv' "$SCENARIO_FILE")
else
  create_doc_resp="$(mcp_call create_doc "$(jq -cn --arg mission_id "$mission_id" '{mission_id:$mission_id,title:"playbook-doc",body:"# playbook\ndoc body",doc_type:"narrative",status:"draft"}')")"
  assert_ok "create_doc" "$create_doc_resp"
  doc_ids+=("$(jq -r '.result.id' <<<"$create_doc_resp")")
  echo "doc_id=${doc_ids[0]}"
fi
doc_id="${doc_ids[0]}"

# Create artifacts from scenario (falls back to a single default if none defined)
declare -a artifact_ids=()
artifact_count="$(jq '.artifacts | length // 0' "$SCENARIO_FILE")"
if [[ "$artifact_count" -gt 0 ]]; then
  while IFS=$'\t' read -r name artifact_type uri mime_type provenance; do
    create_artifact_resp="$(mcp_call create_artifact "$(jq -cn \
      --arg mission_id "$mission_id" --arg name "$name" \
      --arg artifact_type "$artifact_type" --arg uri "$uri" \
      --arg mime_type "$mime_type" --arg provenance "$provenance" \
      '{mission_id:$mission_id,name:$name,artifact_type:$artifact_type,uri:$uri,mime_type:$mime_type,provenance:$provenance,status:"draft"}')")"
    assert_ok "create_artifact:${name}" "$create_artifact_resp"
    artifact_ids+=("$(jq -r '.result.id' <<<"$create_artifact_resp")")
    echo "artifact_id=${artifact_ids[-1]} type=${artifact_type}"
  done < <(jq -r '.artifacts[] | [.name, (.artifact_type // "file"), (.uri // ""), (.mime_type // ""), (.provenance // "")] | @tsv' "$SCENARIO_FILE")
else
  create_artifact_resp="$(mcp_call create_artifact "$(jq -cn --arg mission_id "$mission_id" '{mission_id:$mission_id,name:"playbook-artifact",artifact_type:"file",uri:"https://example.com/playbook",status:"draft"}')")"
  assert_ok "create_artifact" "$create_artifact_resp"
  artifact_ids+=("$(jq -r '.result.id' <<<"$create_artifact_resp")")
  echo "artifact_id=${artifact_ids[0]}"
fi
artifact_id="${artifact_ids[0]}"

load_ws_resp="$(mcp_call load_mission_workspace "$(jq -cn --arg mission_id "$mission_id" '{mission_id:$mission_id}')")"
assert_ok "load_mission_workspace" "$load_ws_resp"
lease_id="$(jq -r '.result.lease.id // .result.lease_id' <<<"$load_ws_resp")"
doc_path="$(jq -r '.result.workspace_snapshot.docs[0].path // empty' <<<"$load_ws_resp")"
if [[ -n "$doc_path" ]]; then
  commit_ws_resp="$(mcp_call commit_mission_workspace "$(jq -cn --arg lease_id "$lease_id" --arg doc_path "$doc_path" '{lease_id:$lease_id,change_set:[{path:$doc_path,content:"# playbook\nworkspace commit"}]}')")"
  assert_ok "commit_mission_workspace" "$commit_ws_resp"
fi
release_ws_resp="$(mcp_call release_mission_workspace "$(jq -cn --arg lease_id "$lease_id" '{lease_id:$lease_id,reason:"playbook done"}')")"
assert_ok "release_mission_workspace" "$release_ws_resp"

mission_deleted=false
domain_deleted=false

if [[ "$SKIP_CLEANUP" == "1" ]]; then
  echo "== Cleanup skipped (EP_PLAYBOOK_SKIP_CLEANUP=1) — objects preserved for review"
else
  for task_id in "${task_ids[@]}"; do
    delete_task_resp="$(mcp_call delete_task "$(jq -cn --arg task_id "$task_id" '{task_id:$task_id}')")"
    assert_ok "delete_task:${task_id}" "$delete_task_resp"
  done

  echo "== Cleanup attempt"
  for did in "${doc_ids[@]}"; do
    delete_doc_resp="$(api_call DELETE "/docs/${did}")"
    echo "[OK] delete_doc_api id=$(jq -r '.deleted_id // empty' <<<"$delete_doc_resp")"
  done

  for aid in "${artifact_ids[@]}"; do
    delete_artifact_resp="$(api_call DELETE "/artifacts/${aid}")"
    echo "[OK] delete_artifact_api id=$(jq -r '.deleted_id // empty' <<<"$delete_artifact_resp")"
  done

  cleanup_err_file="$(mktemp)"
  set +e
  delete_mission_resp="$(api_call DELETE "/domains/${domain_id}/m/${mission_id}" 2>"$cleanup_err_file")"
  cleanup_rc=$?
  set -e
  if [[ $cleanup_rc -eq 0 ]]; then
    echo "[OK] delete_mission"
    mission_deleted=true
    api_call DELETE "/domains/${domain_id}" >/dev/null
    echo "[OK] delete_domain"
    domain_deleted=true
  else
    echo "[WARN] cleanup blocked"
    echo "[WARN] delete_mission stderr: $(tr '\n' ' ' <"$cleanup_err_file")"
  fi
  rm -f "$cleanup_err_file"
fi

task_id_csv="$(IFS=,; echo "${task_ids[*]}")"
doc_id_csv="$(IFS=,; echo "${doc_ids[*]}")"
artifact_id_csv="$(IFS=,; echo "${artifact_ids[*]}")"
result_json="$(
  jq -cn \
    --arg run_id "$RUN_ID" \
    --arg scenario "$scenario_name" \
    --arg scenario_version "$scenario_version" \
    --arg domain_id "$domain_id" \
    --arg mission_id "$mission_id" \
    --arg doc_id "$doc_id" \
    --arg artifact_id "$artifact_id" \
    --arg doc_ids_csv "$doc_id_csv" \
    --arg artifact_ids_csv "$artifact_id_csv" \
    --arg task_ids_csv "$task_id_csv" \
    --argjson expected_task_count "$expected_task_count" \
    --argjson actual_task_count "$actual_task_count" \
    --argjson mission_deleted "$mission_deleted" \
    --argjson domain_deleted "$domain_deleted" \
    '{
      run_id:$run_id,
      scenario:$scenario,
      scenario_version:$scenario_version,
      domain_id:$domain_id,
      mission_id:$mission_id,
      doc_id:$doc_id,
      artifact_id:$artifact_id,
      doc_ids: ($doc_ids_csv | split(",") | map(select(length > 0))),
      artifact_ids: ($artifact_ids_csv | split(",") | map(select(length > 0))),
      task_ids: ($task_ids_csv | split(",") | map(select(length > 0))),
      expected_task_count:$expected_task_count,
      actual_task_count:$actual_task_count,
      cleanup:{mission_deleted:$mission_deleted, domain_deleted:$domain_deleted}
    }'
)"

cat <<EOF
== RESULT
run_id=${RUN_ID}
domain_id=${domain_id}
mission_id=${mission_id}
doc_ids=${doc_id_csv}
artifact_ids=${artifact_id_csv}
task_ids=${task_id_csv}
scenario=${scenario_name}
PLAYBOOK_RESULT_JSON=${result_json}
EOF
