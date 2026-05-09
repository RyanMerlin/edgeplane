# Playbook vs Agent Gate Runbook

This runbook captures the operator steps for supporting the two gated pressure modes that safeguard MissionControl: the deterministic `playbook` gate and the more open-ended `agent` gate. Start each pressure cycle with the playbook gate, only carrying forward to the agent gate after the deterministic pass is recorded. Track the signal path by collecting artifacts, timestamps, and decision rationale so the gate can be formally declared satisfied before promoting to wider agent-level pressure.

## Preconditions
- MissionControl full stack is healthy (postgres, api, mcpd/shim, mosquitto, rustfs) and reachable from the pressure harness.
- `MC_TOKEN` is valid, `MC_BASE_URL` points to the primary API, and the MCP shim at `127.0.0.1:8765` is up.
- Pressure assets already exist (docs/press scripts, artifacts path) and the `docs/CODEX-SWARM-WORKFLOW.md` sequence has been reviewed.
- Metrics pipes are configured (`mosquitto`/`mc daemon` log forwarding, `artifacts/collab/` writable) so every run deposits `summary.json` and `pressure-results.jsonl` where the gate owner can pull them.

Before invoking the harness, verify:

1. `scripts/mc-pressure-test.sh` resolves to the intended Python harness (confirm via `which mc-pressure-test.sh`).
2. The mission/kluster IDs match the reference mission and `MC_PRESSURE_MODE` will run against `mission_id=b1d0cc58dd3e` and `kluster_id=818f645ab54f` when Stage C is running. Document these values in the task notes.
3. Alerting channels (e.g., Slack binding or incident console) are tagged as watchers for `playbook` and `agent` runs so regressions are surfaced during the gating window.

## Playbook gate (deterministic baseline)
1. Use `scripts/mc-pressure-test.sh` with `MC_PRESSURE_MODE=playbook`, `MC_PRESSURE_WORKERS=1..5`, and an MC_PRESSURE_DURATION_SEC window derived from `docs/MCP-VALIDATION-PLAYBOOK.md`. Pin `MC_METADATA=runbook-playbook-gate` so the output is easy to search.
2. Watch the console log and ensure the helper `scripts/mcp-validation-playbook.sh` executes. Confirm each driver run emits `summary.json` under `artifacts/collab/<timestamp>/summary.json` with:
   - `observed_changes > 0`
   - `all_cleanup_succeeded == true`
   - `playbook_results_count > 0`
3. Validate `pressure-results.jsonl` for the run. Look for missing cleanup steps, lingering resources, or `acquire_doc` failures. Flag any regression to the playbook owner before continuing.
4. Capture the deterministic failure mode (HTTP 429, handshake 503, missing docs) by bundling the run-level `summary.json`, `pressure-results.jsonl`, and relevant `mc daemon`/`mcpd` logs, then rerun once the blocker resolves.
5. Declare the playbook gate satisfied by posting the run summary (`artifacts/collab/<run_id>/summary.json`) to the task and noting the timestamp of the final green output.

## Agent gate (real-world concurrency)
1. Only start the agent-pressure harness once the playbook gate run has been confirmed green. Launch `scripts/mc-pressure-test.sh` with `MC_PRESSURE_MODE=agent` against the same mission/kluster IDs that the swarm uses, and set `MC_PRESSURE_METADATA=agent-gate-checkpoint`.
2. Execute the Codex swarm workflow (`docs/CODEX-SWARM-WORKFLOW.md`), clearly assigning roles per session: Session A tracks handshake health, Session B observes instrumentation counters, Session C records runbook and docs outcomes. Document who is responsible for posting artifacts to the task.
3. Observe the MCP handshake logs, `artifacts/collab/<run_id>/summary.json`, `pressure-results.jsonl`, and `mcpd`/shim output. Use the `MC_COLLAPSED` instrumentation toggles and `mosquitto` metrics to detect stalls, task execution islands, or timeouts.
   - Session B should also open `artifacts/pressure/<run_id>/summary.json` and read the new `diagnostics.failure_drilldowns` block. It now surfaces per-category counts (startup timeout, rate limit, auth/ownership ACLs, shim transport, API 5xx, scenario assertions) together with representative log snippets, and `MC_PRESSURE_DIAGNOSTIC_SAMPLE_LIMIT` lets you tune how many lines are kept per bucket for faster triage.
4. If the agent gate regresses (run fails, tasks stay blocked, handshake loops), stop the harness, archive `mcpd`, `mc daemon`, `mosquitto`, and harness logs, and roll back to the last validated playbook summary before re-running. Mention the regression in the task description and attach the relevant artifacts.

## Escalation and turn-over
- Keep the `docs/PLAYBOOK-VS-AGENT-GATEBOOK.md` file linked in the `Runbook and docs accuracy validation` task so that future operators can trace the steps.
- Document why the gate was blocked/done in the task description along with links to the latest playbook summary artifacts.
- Revisit the multi-session driver and codex run if either gate exhibits non-deterministic behavior; note the timestamped `summary.json` and `playbook-results.jsonl` files as part of post-mortems.
- Before handing off to the next session or team, call out which gate mode succeeded, link to the key artifact, and explicitly mention whether the next pressure stage is safe to trigger per the documented acceptance criteria above.
