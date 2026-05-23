# MCP Validation Playbook

This playbook runs a live lifecycle validation against Edgeplane using MCP + API.

## What it covers

- Create domain (MCP)
- Create mission (MCP)
- Create/list/update/delete task (MCP)
- Create doc (MCP) + update doc (API)
- Create artifact (MCP) + update artifact (API)
- Load/commit/release mission workspace (MCP)
- Cleanup attempt (delete mission/domain)

## Pressure Harness (multi-agent)

For concurrent pressure tests against the Rust `edgeplane daemon` shim path, use:

`scripts/edgeplane-pressure-test.sh`

Defaults:

- mode: `agent`
- workers: `5`
- duration: `600` seconds
- model: `gpt-5.1-codex-mini`
- stack profile: `full` (`EP_STACK_PROFILE=full`)

Required env:

- `EP_BASE_URL`
- `EP_TOKEN`
- local shim must be reachable at `EP_DAEMON_HOST:EP_DAEMON_PORT` (defaults `127.0.0.1:8765`)
- full Docker stack should be running (`bash scripts/dev-up.sh`)

Example:

```bash
export EP_BASE_URL=http://localhost:8008
export EP_TOKEN="<token>"
EP_PRESSURE_MODE=agent EP_PRESSURE_WORKERS=5 EP_PRESSURE_DURATION_SEC=600 \
scripts/edgeplane-pressure-test.sh
```

Deterministic baseline mode (no Codex workers):

```bash
EP_PRESSURE_MODE=playbook EP_PRESSURE_WORKERS=5 EP_PRESSURE_DURATION_SEC=600 \
scripts/edgeplane-pressure-test.sh
```

Quickstart remains available for local debugging only:

```bash
EP_STACK_PROFILE=quickstart EP_PRESSURE_MODE=playbook EP_PRESSURE_WORKERS=1 EP_PRESSURE_DURATION_SEC=15 \
scripts/edgeplane-pressure-test.sh
```

For real multi-session Codex collaboration pressure (non-nested), use:

- `docs/CODEX-SWARM-WORKFLOW.md`

The pressure harness now emits a versioned summary report at:

- `artifacts/pressure/<run_id>/summary.json`

Report includes strict gate fields:

- `pass` (true/false)
- `fatal_worker_failures`
- `failures_by_category` (`auth_config`, `rate_limit`, `ownership_acl`, `shim_transport`, `api_5xx`, `scenario_assertion`)
- `end_state` assertions and extracted playbook results

## Script

`scripts/mcp-validation-playbook.sh`

## Prerequisites

- Running API (default `http://localhost:8008`)
- Auth token exported as `EP_TOKEN`
- `jq` and `curl` installed

## Run

```bash
export EP_BASE_URL=http://localhost:8008
export EP_TOKEN="<token>"
scripts/mcp-validation-playbook.sh
```

Optional variables:

- `EP_PLAYBOOK_ACTOR` (default: `token-client`)
- `EP_PLAYBOOK_RUN_ID` (default: timestamp)
- `EP_PLAYBOOK_SCENARIO_FILE` (default: `scripts/pressure-scenarios/reliability-trio.json`)

## Notes

- The canonical scenario is `reliability-trio` (3 deterministic tasks).
- Playbook emits a machine-readable line `PLAYBOOK_RESULT_JSON=...` used by the pressure harness for strict end-state checks.
