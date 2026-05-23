# Evolve — Edgeplane Self-Improvement Loop

`edgeplane agent evolve` seeds and tracks a self-improvement backlog in Edgeplane.
Today it is a thin domain/run tracker: it stores the evolve spec, records run launches,
and returns run status metadata.

## Quick Start

```bash
# 1. Seed a domain with a task backlog
edgeplane agent evolve seed --spec docs/examples/evolve-seed-spec.json
# → outputs domain_id: evolve-abc12345

# 2. Record a run launch for that domain
edgeplane agent evolve run --domain evolve-abc12345 --agent claude

# 3. Inspect domain/run status metadata
edgeplane agent evolve status --domain evolve-abc12345
```

## Spec File Format

The spec file is JSON:

```json
{
  "name": "MC self-improvement sprint 1",
  "description": "Improve pressure test latency and add evolve loop",
  "agent_system_prompt": "You are an expert Rust and Python engineer improving Edgeplane. Work from the task backlog. Each task should result in a passing test. Commit your changes.",
  "scoring_criteria": {
    "tests_passing": true,
    "builds_clean": true,
    "diff_reviewable": true
  },
  "tasks": [
    {
      "id": "t001",
      "title": "Add latency percentiles to pressure test",
      "description": "Add P50/P95/P99 latency tracking to edgeplane-pressure-test.sh",
      "acceptance": "summary.json contains latency_p95_ms field"
    },
    {
      "id": "t002",
      "title": "Add edgeplane agent evolve CLI",
      "description": "Implement edgeplane agent evolve seed/run/status subcommands",
      "acceptance": "edgeplane agent evolve --help shows seed, run, status"
    }
  ]
}
```

## Subcommands

| Command | Description |
|---------|-------------|
| `edgeplane agent evolve seed --spec <file>` | Seed an evolve domain from a JSON spec. Outputs `domain_id`. |
| `edgeplane agent evolve run --domain <id> [--agent <name>]` | Record a launched run entry (default: claude) for a seeded domain. |
| `edgeplane agent evolve status --domain <id>` | Show domain status, task count from the seeded spec, and recorded runs. |

## Current Behavior

1. **seed** — stores a new evolve domain record in backend memory and returns a generated `domain_id`.
2. **run** — appends a run record (`run_id`, `agent`, `started_at`, `status=launched`) to the domain.
3. **status** — returns domain metadata (`status`, `task_count`, `run_count`, and run records).

## Current Limitations

- Storage is in-memory only (process-local); data resets on restart.
- Domain records are not yet persisted to the DB.
- Task completion scoring is not computed automatically.
- Run launch does not yet orchestrate/stream a real agent execution loop by itself.

## Agent Roles

| Agent | Best For |
|-------|----------|
| claude | Complex refactors, architecture changes, test authoring |
| codex | Fast iteration on well-defined code tasks |
| gemini | Documentation, multi-file review |
| openclaw | Parallel sub-task execution |

## Guardrails

- Existing Edgeplane auth middleware still applies to evolve endpoints.
- Domain records are scoped to the authenticated principal (`subject`).
- The evolve spec is stored as provided and echoed via status metadata.

## Backend API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/evolve/domains` | POST | Seed a new evolve domain |
| `/evolve/domains/{id}/run` | POST | Launch agent run |
| `/evolve/domains/{id}/status` | GET | Get domain progress |

## Planned Next Steps

- [ ] Add retention/cleanup policies for older evolve domains and runs
- [ ] Score runs automatically (parse test results, build status from artifacts)
- [ ] `edgeplane agent evolve watch` — stream agent output in real time
- [ ] Leaderboard: which agent completed the most tasks with the highest scores
