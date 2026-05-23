#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_ROOT="${MC_PRESSURE_OUT_ROOT:-$ROOT_DIR/artifacts/pressure}"
RUN_ID="${MC_PRESSURE_RUN_ID:-$(date +%Y%m%d%H%M%S)}"
MODE="${MC_PRESSURE_MODE:-agent}" # agent | playbook
WORKERS="${MC_PRESSURE_WORKERS:-5}"
DURATION_SEC="${MC_PRESSURE_DURATION_SEC:-600}"
MODEL="${MC_PRESSURE_MODEL:-gpt-5.1-codex-mini}"
BASE_URL="${EP_BASE_URL:-http://localhost:8008}"
MCP_CMD="${MC_PRESSURE_MCP_COMMAND:-edgeplane}"
SHIM_HOST="${MC_DAEMON_HOST:-127.0.0.1}"
SHIM_PORT="${MC_DAEMON_PORT:-8765}"
STACK_PROFILE="${EP_STACK_PROFILE:-full}"
UNSANDBOXED="${MC_PRESSURE_UNSANDBOXED:-0}"
AUTOSTART_DAEMON="${MC_PRESSURE_AUTOSTART_DAEMON:-1}"
DAEMON_PID=""
WORKER_PIDS=()
INTERVAL_MS="${MC_PRESSURE_INTERVAL_MS:-200}"
ON_429_SLEEP_MS="${MC_PRESSURE_ON_429_SLEEP_MS:-1000}"
TOKENS_CSV="${MC_PRESSURE_TOKENS:-}"
REPORT_VERSION="1.0.0"
SCENARIO_FILE="${MC_PRESSURE_SCENARIO_FILE:-$ROOT_DIR/scripts/pressure-scenarios/reliability-trio.json}"
AGENT_STARTUP_TIMEOUT_SEC="${MC_PRESSURE_AGENT_STARTUP_TIMEOUT_SEC:-120}"
AGENT_TOOL_TIMEOUT_SEC="${MC_PRESSURE_AGENT_TOOL_TIMEOUT_SEC:-120}"
AGENT_ITER_SLEEP_MS="${MC_PRESSURE_AGENT_ITER_SLEEP_MS:-500}"
AGENT_EXEC_TIMEOUT_SEC="${MC_PRESSURE_AGENT_EXEC_TIMEOUT_SEC:-300}"
AGENT_DRIVER="${MC_PRESSURE_AGENT_DRIVER:-daemon}" # daemon | codex
DIAGNOSTIC_SAMPLE_LIMIT="${MC_PRESSURE_DIAGNOSTIC_SAMPLE_LIMIT:-5}"
AGENT_ON_429_SLEEP_MS="${MC_PRESSURE_AGENT_ON_429_SLEEP_MS:-1000}"
PLAYBOOK_SKIP_CLEANUP="${EP_PLAYBOOK_SKIP_CLEANUP:-0}"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required" >&2
  exit 1
fi
if [[ -z "${EP_TOKEN:-}" ]]; then
  echo "EP_TOKEN is required" >&2
  exit 2
fi

if [[ "$MODE" == "agent" && "$AGENT_DRIVER" == "codex" ]] && ! command -v codex >/dev/null 2>&1; then
  echo "codex is required for MC_PRESSURE_MODE=agent" >&2
  exit 1
fi
if [[ "$MODE" == "agent" && "$AGENT_DRIVER" == "codex" ]] && ! command -v timeout >/dev/null 2>&1; then
  echo "timeout is required for MC_PRESSURE_MODE=agent" >&2
  exit 1
fi
if [[ ! -f "$SCENARIO_FILE" ]]; then
  echo "MC_PRESSURE_SCENARIO_FILE not found: $SCENARIO_FILE" >&2
  exit 2
fi

mkdir -p "$OUT_ROOT"
RUN_DIR="$OUT_ROOT/$RUN_ID"
mkdir -p "$RUN_DIR/workers"

cleanup() {
  trap - EXIT INT TERM
  if [[ "${#WORKER_PIDS[@]}" -gt 0 ]]; then
    kill "${WORKER_PIDS[@]}" >/dev/null 2>&1 || true
  fi
  if [[ -n "$DAEMON_PID" ]]; then
    kill "$DAEMON_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

collect_log_samples() {
  local pattern="$1"
  local limit="${2:-$DIAGNOSTIC_SAMPLE_LIMIT}"
  local matches
  matches="$(rg --no-heading -e "$pattern" "$RUN_DIR/workers" -g '*.stderr' -g '*.log' -g '*.txt' -g '*.jsonl' 2>/dev/null || true)"
  if [[ -z "$matches" ]]; then
    echo '[]'
    return
  fi
  printf '%s\n' "$matches" | head -n "$limit" | jq -R -s 'split("\n") | map(select(length > 0))'
}

echo "== MC pressure test =="
echo "run_id=$RUN_ID"
echo "mode=$MODE workers=$WORKERS duration_sec=$DURATION_SEC model=$MODEL"
echo "base_url=$BASE_URL shim=${SHIM_HOST}:${SHIM_PORT}"
echo "stack_profile=$STACK_PROFILE"
echo "out_dir=$RUN_DIR"

echo "== preflight checks =="
if [[ "$STACK_PROFILE" == "full" ]]; then
  if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    if [[ "$(docker inspect --format '{{.State.Running}}' edgeplane-postgres 2>/dev/null || true)" != "true" ]]; then
      echo "full profile requires running postgres service; run EP_STACK_PROFILE=full bash scripts/dev-up.sh" >&2
      exit 1
    fi
    if ! docker exec edgeplane-postgres psql -U "${POSTGRES_USER:-edgeplane}" -d "${POSTGRES_DB:-edgeplane}" -tAc \
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

if ! curl -fsS "http://${SHIM_HOST}:${SHIM_PORT}/v1/health" >/dev/null 2>&1; then
  if [[ "$AUTOSTART_DAEMON" == "1" ]]; then
    if ! command -v edgeplane >/dev/null 2>&1; then
      echo "edgeplane is required when MC_PRESSURE_AUTOSTART_DAEMON=1" >&2
      exit 1
    fi
    echo "shim not reachable; auto-starting edgeplane daemon"
    EP_BASE_URL="$BASE_URL" EP_TOKEN="${EP_TOKEN}" \
      edgeplane daemon --disable-matrix --shim-host "$SHIM_HOST" --shim-port "$SHIM_PORT" \
      >"$RUN_DIR/edgeplane-daemon.log" 2>&1 &
    DAEMON_PID="$!"
    for _ in $(seq 1 40); do
      if curl -fsS "http://${SHIM_HOST}:${SHIM_PORT}/v1/health" >/dev/null 2>&1; then
        break
      fi
      sleep 0.25
    done
  fi
fi
curl -fsS -H "Authorization: Bearer ${EP_TOKEN}" "$BASE_URL/mcp/health" >/dev/null
curl -fsS "http://${SHIM_HOST}:${SHIM_PORT}/v1/health" >/dev/null
echo "preflight=ok"

run_agent_worker() {
  local worker_id="$1"
  local worker_token="$2"
  local worker_dir="$RUN_DIR/workers/agent-${worker_id}"
  mkdir -p "$worker_dir"
  local status_file="$worker_dir/status.json"
  local start_ts
  start_ts="$(date +%s)"
  local deadline=$((start_ts + DURATION_SEC))
  local attempts=0
  local successes=0
  local failures=0
  local consecutive_429s=0

  while [[ "$(date +%s)" -lt "$deadline" ]]; do
    attempts=$((attempts + 1))
    local iter_log="$worker_dir/iter-${attempts}.jsonl"
    local iter_msg="$worker_dir/iter-${attempts}.txt"
    local prompt
    prompt=$(
      cat <<EOF
You are pressure-test worker ${worker_id} run ${RUN_ID} attempt ${attempts}.
Use the edgeplane MCP server.
1) Validate MCP connectivity by invoking one direct Edgeplane read tool call (\`list_missions\`).
2) If the Edgeplane tool call succeeds, return RESULT: ok even when it returns empty data.
3) If write/read resources are available, execute one write action and one read action.
4) End with exactly one line: RESULT: ok | RESULT: fail
EOF
    )

    local iter_start_ms
    iter_start_ms="$(date +%s%3N)"
    if [[ "$AGENT_DRIVER" == "daemon" ]]; then
      local out_file="$worker_dir/iter-${attempts}.log"
      local http_code
      http_code="$(curl -sS -o "$out_file" -w "%{http_code}" \
        -H "Authorization: Bearer ${worker_token}" \
        -H "Content-Type: application/json" \
        -X POST "$BASE_URL/mcp/call" \
        -d '{"tool":"list_missions","args":{}}' || true)"
      if [[ "$http_code" == "200" ]] && jq -e '.ok == true' "$out_file" >/dev/null 2>&1; then
        echo "RESULT: ok" >"$iter_msg"
        successes=$((successes + 1))
        consecutive_429s=0
      else
        echo "RESULT: fail" >"$iter_msg"
        failures=$((failures + 1))
        if [[ "$http_code" == "429" ]]; then
          consecutive_429s=$((consecutive_429s + 1))
          local exp=$(( consecutive_429s > 5 ? 5 : consecutive_429s - 1 ))
          local exp_ms=$(( AGENT_ON_429_SLEEP_MS * (1 << exp) ))
          [[ $exp_ms -gt 30000 ]] && exp_ms=30000
          local backoff_ms=$(( exp_ms + (RANDOM % 1000) ))
          sleep "$(awk "BEGIN{printf \"%.3f\", ${backoff_ms}/1000}")"
        else
          consecutive_429s=0
        fi
      fi
      echo "{\"driver\":\"daemon\",\"http_code\":\"$http_code\"}" >"$iter_log"
    else
      codex_args=(exec --json --skip-git-repo-check -C "$ROOT_DIR" -m "$MODEL")
      if [[ "$UNSANDBOXED" == "1" ]]; then
        codex_args+=(--dangerously-bypass-approvals-and-sandbox)
      else
        codex_args+=(--sandbox workspace-write)
      fi
      if timeout "${AGENT_EXEC_TIMEOUT_SEC}" codex "${codex_args[@]}" \
        -c "mcp_servers.edgeplane.command=\"$MCP_CMD\"" \
        -c "mcp_servers.edgeplane.args=[\"serve\"]" \
        -c "mcp_servers.edgeplane.startup_timeout_sec=${AGENT_STARTUP_TIMEOUT_SEC}" \
        -c "mcp_servers.edgeplane.tool_timeout_sec=${AGENT_TOOL_TIMEOUT_SEC}" \
        -c "mcp_servers.edgeplane.env={MC_MCP_MODE=\"shim\",MC_DAEMON_HOST=\"$SHIM_HOST\",MC_DAEMON_PORT=\"$SHIM_PORT\",MC_FAIL_OPEN_ON_LIST=\"1\",MC_STARTUP_PREFLIGHT=\"none\",EP_BASE_URL=\"$BASE_URL\",EP_TOKEN=\"${worker_token}\",EDGEPLANE_BASE_URL=\"$BASE_URL\",EDGEPLANE_TOKEN=\"${worker_token}\",EDGEPLANE_STARTUP_PREFLIGHT=\"none\",EDGEPLANE_FAIL_OPEN_ON_LIST=\"1\"}" \
        -o "$iter_msg" "$prompt" >"$iter_log" 2>"$worker_dir/iter-${attempts}.stderr"; then
        if rg -q "RESULT: ok" "$iter_msg"; then
          successes=$((successes + 1))
        else
          failures=$((failures + 1))
        fi
      else
        failures=$((failures + 1))
      fi
    fi
    echo $(( $(date +%s%3N) - iter_start_ms )) >> "$worker_dir/latencies.txt"
    sleep "$(awk "BEGIN{printf \"%.3f\", ${AGENT_ITER_SLEEP_MS}/1000}")"
  done

  local end_ts
  end_ts="$(date +%s)"
  jq -n \
    --arg worker_id "$worker_id" \
    --arg mode "$MODE" \
    --argjson attempts "$attempts" \
    --argjson successes "$successes" \
    --argjson failures "$failures" \
    --argjson start_ts "$start_ts" \
    --argjson end_ts "$end_ts" \
    '{worker_id:$worker_id,mode:$mode,attempts:$attempts,successes:$successes,failures:$failures,start_ts:$start_ts,end_ts:$end_ts}' \
    > "$status_file"
}

run_playbook_worker() {
  local worker_id="$1"
  local worker_token="$2"
  local worker_dir="$RUN_DIR/workers/playbook-${worker_id}"
  mkdir -p "$worker_dir"
  local status_file="$worker_dir/status.json"
  local start_ts
  start_ts="$(date +%s)"
  local deadline=$((start_ts + DURATION_SEC))
  local attempts=0
  local successes=0
  local failures=0
  local consecutive_429s=0

  while [[ "$(date +%s)" -lt "$deadline" ]]; do
    attempts=$((attempts + 1))
    local err_file="$worker_dir/iter-${attempts}.stderr"
    local iter_start_ms
    iter_start_ms="$(date +%s%3N)"
    if EP_PLAYBOOK_RUN_ID="${RUN_ID}-w${worker_id}-i${attempts}" \
      EP_PLAYBOOK_SCENARIO_FILE="${SCENARIO_FILE}" \
      EP_PLAYBOOK_SKIP_CLEANUP="${PLAYBOOK_SKIP_CLEANUP}" \
      EP_BASE_URL="$BASE_URL" \
      EP_TOKEN="${worker_token}" \
      "$ROOT_DIR/scripts/mcp-validation-playbook.sh" \
      >"$worker_dir/iter-${attempts}.log" 2>"$err_file"; then
      successes=$((successes + 1))
      consecutive_429s=0
    else
      failures=$((failures + 1))
      if rg -q " 429|error: 429|URL returned error: 429" "$err_file" 2>/dev/null; then
        consecutive_429s=$((consecutive_429s + 1))
        local retry_after
        retry_after="$(rg -oI 'retry-after:\s*([0-9]+)' -i "$err_file" --replace '$1' 2>/dev/null | head -1 || true)"
        local backoff_ms
        if [[ -n "$retry_after" ]]; then
          backoff_ms=$(( retry_after * 1000 ))
          [[ $backoff_ms -gt 30000 ]] && backoff_ms=30000
        else
          local exp=$(( consecutive_429s > 5 ? 5 : consecutive_429s - 1 ))
          local exp_ms=$(( ON_429_SLEEP_MS * (1 << exp) ))
          [[ $exp_ms -gt 30000 ]] && exp_ms=30000
          backoff_ms=$(( exp_ms + (RANDOM % 1000) ))
        fi
        sleep "$(awk "BEGIN{printf \"%.3f\", ${backoff_ms}/1000}")"
      else
        consecutive_429s=0
      fi
    fi
    echo $(( $(date +%s%3N) - iter_start_ms )) >> "$worker_dir/latencies.txt"
    sleep "$(awk "BEGIN{printf \"%.3f\", ${INTERVAL_MS}/1000}")"
  done

  local end_ts
  end_ts="$(date +%s)"
  jq -n \
    --arg worker_id "$worker_id" \
    --arg mode "$MODE" \
    --argjson attempts "$attempts" \
    --argjson successes "$successes" \
    --argjson failures "$failures" \
    --argjson start_ts "$start_ts" \
    --argjson end_ts "$end_ts" \
    '{worker_id:$worker_id,mode:$mode,attempts:$attempts,successes:$successes,failures:$failures,start_ts:$start_ts,end_ts:$end_ts}' \
    > "$status_file"
}

IFS=',' read -r -a token_list <<< "$TOKENS_CSV"
token_count="${#token_list[@]}"
for i in $(seq 1 "$WORKERS"); do
  worker_token="${EP_TOKEN:-}"
  if [[ "$token_count" -gt 0 && -n "${token_list[0]}" ]]; then
    idx=$(( (i - 1) % token_count ))
    worker_token="$(echo "${token_list[$idx]}" | xargs)"
  fi
  if [[ -z "$worker_token" ]]; then
    echo "worker $i has empty token; set EP_TOKEN or MC_PRESSURE_TOKENS" >&2
    exit 2
  fi
  if [[ "$MODE" == "agent" ]]; then
    run_agent_worker "$i" "$worker_token" &
  else
    run_playbook_worker "$i" "$worker_token" &
  fi
  WORKER_PIDS+=("$!")
done
wait

status_files=("$RUN_DIR"/workers/*/status.json)
if [[ ! -e "${status_files[0]}" ]]; then
  jq -n \
    --arg report_version "$REPORT_VERSION" \
    --arg run_id "$RUN_ID" \
    --arg mode "$MODE" \
    --arg model "$MODEL" \
    --arg base_url "$BASE_URL" \
    --arg shim_host "$SHIM_HOST" \
    --argjson shim_port "$SHIM_PORT" \
    '{
      report_version: $report_version,
      run_id: $run_id,
      mode: $mode,
      model: $model,
      workers: 0,
      duration_sec: 0,
      attempts: 0,
      successes: 0,
      failures: 0,
      fatal_worker_failures: 0,
      success_rate: 0,
      startup_timeout_hits: 0,
      latency: {count: 0, p50_ms: 0, p95_ms: 0, p99_ms: 0},
      metrics: {startup_timeout_hits: 0, rate_limit_hits: 0, latency_p50_ms: 0, latency_p95_ms: 0, latency_p99_ms: 0},
      failures_by_category: {auth_config: 0, rate_limit: 0, ownership_acl: 0, shim_transport: 0, api_5xx: 0, scenario_assertion: 0},
      diagnostics: {sample_limit: 0, failure_drilldowns: {}, note: "No worker status files were produced; workers likely exited before reporting."},
      endpoint: {base_url: $base_url, shim_host: $shim_host, shim_port: $shim_port},
      scenario: {file: "", playbook_results: []},
      end_state: {playbook_results_count: 0, all_cleanup_succeeded: false, task_counts_match: false},
      pass: false,
      failure_reason: "missing_worker_status"
    }' > "$RUN_DIR/summary.json"
  echo "== summary =="
  cat "$RUN_DIR/summary.json"
  echo "result=fail strict_gate=failed"
  exit 1
fi

all_latencies_json="$(cat "$RUN_DIR"/workers/*/latencies.txt 2>/dev/null \
  | jq -Rs 'split("\n") | map(select(length > 0) | tonumber) | sort' || echo "[]")"
latency_count=$(echo "$all_latencies_json" | jq 'length')
latency_p50=$(echo "$all_latencies_json" | jq 'if length > 0 then .[(((length * 0.50)|tostring|split(".")[0])|tonumber)] else 0 end')
latency_p95=$(echo "$all_latencies_json" | jq 'if length > 0 then .[(((length * 0.95)|tostring|split(".")[0])|tonumber)] else 0 end')
latency_p99=$(echo "$all_latencies_json" | jq 'if length > 0 then .[(((length * 0.99)|tostring|split(".")[0])|tonumber)] else 0 end')

startup_timeout_hits="$({ rg -n -e "MCP startup incomplete" -e "timed out after" "$RUN_DIR" -g '*.stderr' -g '*.txt' 2>/dev/null || true; } | wc -l | tr -d ' ')"
auth_config_hits="$({ rg -n -e "EP_TOKEN is required" -e "Forbidden" -e "Unauthorized" "$RUN_DIR/workers" -g '*.stderr' -g '*.log' 2>/dev/null || true; } | wc -l | tr -d ' ')"
rate_limit_hits="$({ rg -n -e " 429" -e "error: 429" -e "URL returned error: 429" "$RUN_DIR/workers" -g '*.stderr' -g '*.log' 2>/dev/null || true; rg -n -e '"http_code":"429"' "$RUN_DIR/workers" -g '*.jsonl' 2>/dev/null || true; } | wc -l | tr -d ' ')"
ownership_acl_hits="$({ rg -n -e "owner required" -e "contributor or owner required" "$RUN_DIR/workers" -g '*.stderr' -g '*.log' 2>/dev/null || true; } | wc -l | tr -d ' ')"
shim_transport_hits="$({ rg -n -e "MCP startup incomplete" -e "timed out handshaking" -e "timed out after" -e "unexpected status code" -e "Connection refused" "$RUN_DIR/workers" -g '*.stderr' -g '*.log' -g '*.txt' 2>/dev/null || true; } | wc -l | tr -d ' ')"
api_5xx_hits="$({ rg -n -e "HTTP 5[0-9][0-9]" "$RUN_DIR/workers" -g '*.stderr' -g '*.log' 2>/dev/null || true; } | wc -l | tr -d ' ')"
scenario_assertion_hits="$({ rg -n -e "scenario_assertion" "$RUN_DIR/workers" -g '*.stderr' -g '*.log' 2>/dev/null || true; } | wc -l | tr -d ' ')"
playbook_results_file="$RUN_DIR/playbook-results.jsonl"
{ rg --no-heading -n "PLAYBOOK_RESULT_JSON=" "$RUN_DIR/workers" -g '*.log' 2>/dev/null || true; } \
  | sed 's/^.*PLAYBOOK_RESULT_JSON=//' > "$playbook_results_file"

diag_startup_samples="$(collect_log_samples 'MCP startup incomplete|timed out handshaking|timed out after|unexpected status code|Connection refused')"
diag_rate_limit_samples="$(collect_log_samples ' 429|error: 429|URL returned error: 429|too many requests|http_code.*429')"
diag_auth_samples="$(collect_log_samples 'EP_TOKEN is required|Forbidden|Unauthorized')"
diag_ownership_samples="$(collect_log_samples 'owner required|contributor or owner required')"
diag_shim_samples="$(collect_log_samples 'MCP startup incomplete|timed out handshaking|timed out after|unexpected status code|Connection refused')"
diag_api_samples="$(collect_log_samples 'HTTP 5[0-9][0-9]')"
diag_scenario_samples="$(collect_log_samples 'scenario_assertion')"

jq -s \
  --arg report_version "$REPORT_VERSION" \
  --arg run_id "$RUN_ID" \
  --arg mode "$MODE" \
  --arg model "$MODEL" \
  --arg base_url "$BASE_URL" \
  --arg shim_host "$SHIM_HOST" \
  --argjson shim_port "$SHIM_PORT" \
  --arg scenario_file "$SCENARIO_FILE" \
  --argjson startup_timeout_hits "$startup_timeout_hits" \
  --argjson auth_config_hits "$auth_config_hits" \
  --argjson rate_limit_hits "$rate_limit_hits" \
  --argjson ownership_acl_hits "$ownership_acl_hits" \
  --argjson shim_transport_hits "$shim_transport_hits" \
  --argjson api_5xx_hits "$api_5xx_hits" \
  --argjson scenario_assertion_hits "$scenario_assertion_hits" \
  --argjson diag_sample_limit "$DIAGNOSTIC_SAMPLE_LIMIT" \
  --argjson diag_startup_samples "$diag_startup_samples" \
  --argjson diag_rate_limit_samples "$diag_rate_limit_samples" \
  --argjson diag_auth_samples "$diag_auth_samples" \
  --argjson diag_ownership_samples "$diag_ownership_samples" \
  --argjson diag_shim_samples "$diag_shim_samples" \
  --argjson diag_api_samples "$diag_api_samples" \
  --argjson diag_scenario_samples "$diag_scenario_samples" \
  --argjson latency_count "$latency_count" \
  --argjson latency_p50 "$latency_p50" \
  --argjson latency_p95 "$latency_p95" \
  --argjson latency_p99 "$latency_p99" \
  --slurpfile playbook_results "$playbook_results_file" \
  '{
    report_version: $report_version,
    run_id: $run_id,
    mode: $mode,
    model: $model,
    workers: length,
    duration_sec: (map(.end_ts - .start_ts) | max // 0),
    attempts: (map(.attempts)|add),
    successes: (map(.successes)|add),
    failures: (map(.failures)|add),
    fatal_worker_failures: (map(select(.failures > 0)) | length),
    success_rate: (if (map(.attempts)|add) == 0 then 0 else ((map(.successes)|add) / (map(.attempts)|add)) end),
    startup_timeout_hits: $startup_timeout_hits,
    latency: {
      count: $latency_count,
      p50_ms: $latency_p50,
      p95_ms: $latency_p95,
      p99_ms: $latency_p99
    },
    metrics: {
      startup_timeout_hits: $startup_timeout_hits,
      rate_limit_hits: $rate_limit_hits,
      latency_p50_ms: $latency_p50,
      latency_p95_ms: $latency_p95,
      latency_p99_ms: $latency_p99
    },
    failures_by_category: {
      auth_config: $auth_config_hits,
      rate_limit: $rate_limit_hits,
      ownership_acl: $ownership_acl_hits,
      shim_transport: $shim_transport_hits,
      api_5xx: $api_5xx_hits,
      scenario_assertion: $scenario_assertion_hits
    },
    diagnostics: {
      sample_limit: $diag_sample_limit,
      failure_drilldowns: {
        startup_timeout: {count: $startup_timeout_hits, samples: $diag_startup_samples},
        rate_limit: {count: $rate_limit_hits, samples: $diag_rate_limit_samples},
        auth_config: {count: $auth_config_hits, samples: $diag_auth_samples},
        ownership_acl: {count: $ownership_acl_hits, samples: $diag_ownership_samples},
        shim_transport: {count: $shim_transport_hits, samples: $diag_shim_samples},
        api_5xx: {count: $api_5xx_hits, samples: $diag_api_samples},
        scenario_assertion: {count: $scenario_assertion_hits, samples: $diag_scenario_samples}
      },
      note: ("Failure samples are trimmed to " + ($diag_sample_limit | tostring) + " entries per category for readability.")
    },
    endpoint: {
      base_url: $base_url,
      shim_host: $shim_host,
      shim_port: $shim_port
    },
    scenario: {
      file: $scenario_file,
      playbook_results: ($playbook_results // [])
    },
    end_state: {
      playbook_results_count: (($playbook_results // []) | length),
      all_cleanup_succeeded: (($playbook_results // []) | all(.cleanup.mission_deleted == true and .cleanup.domain_deleted == true)),
      task_counts_match: (($playbook_results // []) | all(.actual_task_count >= .expected_task_count))
    }
  }' "${status_files[@]}" > "$RUN_DIR/summary.json"

pass=true
if [[ "$(jq -r '.fatal_worker_failures' "$RUN_DIR/summary.json")" -ne 0 ]]; then
  pass=false
fi
if [[ "$(jq -r '.end_state.task_counts_match' "$RUN_DIR/summary.json")" != "true" ]]; then
  pass=false
fi
if [[ "$MODE" == "playbook" ]] && [[ "$PLAYBOOK_SKIP_CLEANUP" != "1" ]] && [[ "$(jq -r '.end_state.all_cleanup_succeeded' "$RUN_DIR/summary.json")" != "true" ]]; then
  pass=false
fi
if [[ "$MODE" == "playbook" ]] && [[ "$(jq -r '.end_state.playbook_results_count' "$RUN_DIR/summary.json")" -eq 0 ]]; then
  pass=false
fi
jq --argjson pass "$pass" '. + {pass:$pass}' "$RUN_DIR/summary.json" > "$RUN_DIR/summary.tmp.json"
mv "$RUN_DIR/summary.tmp.json" "$RUN_DIR/summary.json"

echo "== summary =="
cat "$RUN_DIR/summary.json"

if [[ "$(jq -r '.pass' "$RUN_DIR/summary.json")" == "true" ]]; then
  echo "result=pass strict_gate=ok"
  exit 0
fi
echo "result=fail strict_gate=failed"
exit 1
