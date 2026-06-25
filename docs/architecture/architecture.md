# Architecture

## Core Runtime
- **Postgres (+ pgvector)**: authoritative coordination state (domains, missions, tasks, approvals, profiles, ledger).
- **S3-compatible object storage**: active artifact/document bytes and workspace file persistence.
- **Git publication layer**: explicit memory-of-record projection for routable entities.
- **MCP/API control plane**: policy-gated tool execution, publish planning, and audited mutations.

## Persistence Model
- **Coordination truth stays in Edgeplane (Postgres)**.
- **Git is a projection sink**, never the authority for domain ownership, approvals, or governance.
- **Domain-scoped routing** controls where publication events land:
  - `repo_connections`
  - `repo_bindings`
  - `domainpersistencepolicy`
  - `domainpersistenceroute`
  - `publication_records`

## Publish Flow
1. Mutation enters ledger (`pending`) in Postgres.
2. Approval/policy checks run in Edgeplane.
3. Route resolver picks binding/repo/branch/path from domain policy.
4. Provider adapter acquires server-side credential.
5. Publisher writes canonical file(s) to Git and records commit provenance.
6. Ledger/publication records are marked and queryable via API/MCP.

## Security Model (shipped v0.15.0)

### Principal types

Four principal types, two trust tiers:

| Principal | `auth_type` | Trust tier |
|-----------|-------------|------------|
| Human session (OIDC) | `session` | Full-trust |
| Node / daemon (node JWT) | `node` | Full-trust — first-party infrastructure |
| Service account | `service_account` | Scoped |
| Agent (per-agent JWT) | `agent` | Scoped — domain-bound |

Full-trust principals (`session`, `node`) pass all domain authorization checks unconditionally. Scoped principals must satisfy `authorized_for_domain`.

### Domain authorization (Seam 1)

`Domain` is the authorization boundary. Every privileged dispatch, ledger, and stream handler enforces a default-deny `authorized_for_domain` predicate before acting:

```
authorized_for_domain(principal, domain) :=
    principal.is_admin
    OR principal.auth_type == "node"
    OR domain.id ∈ principal.domain_scope      // per-agent JWT home domain
    OR principal.subject ∈ domain.owners
    OR principal.subject ∈ domain.contributors
```

In addition, lifecycle mutations on a `MeshTask` require the caller to hold the task's `claim_lease_id` / be its `claimed_by_agent_id` (`authz_task_owner`), unless full-trust or admin. A compromised agent is therefore bounded to its own tasks within its domain.

### Per-agent JWT identity (Seam 2)

Each enrolled agent receives its own short-lived (12 h), domain-scoped JWT (`AgentClaims`). Tokens are RS256-signed, carried in the `agenttoken` revocation table (migration `0010`), and injected by the daemon as each agent's `EP_AGENT_TOKEN`. Agents cannot mint peer tokens — the mint endpoint (`POST /work/agents/{id}/token`) is full-trust/admin-gated. `claim` and `progress` actions are attributed to the authenticated agent identity in both the REST and MCP surfaces.

Token minting is restricted to runtimes that consume them (`claude_code`); other runtime kinds use the shared daemon token.

---

## Ephemeral Task Subagents (edgeplaned task_worker)
- **`edgeplaned::task_worker`** runs the task claim loop per node. See `docs/design/ephemeral-task-subagents.md` for the full identity model.
- **Bootstrap** (edgeplaned startup): ensures a default `home` domain (overridable via `EP_HOME_DOMAIN_NAME`) and an `intake` mission under it. Idempotent, soft-fail. The `home` domain is a regular domain — no special `kind` — that serves as the default container for operational scaffolding (the intake mission, optionally agents' `home_domain_id`).
- **Claim loop**: polls for `MeshTask`s with `status='ready'` whose `claim_policy.target_profile` matches a profile supervised on this node. For each match: enrolls an ephemeral `MeshAgent` under the parent `Agent` identity, claims the task, opens an `AgentRun`, spawns `claude -p` in a per-task worktree (`~/.ep/worktrees/<task_id>/`) with `--allowed-tools` derived from `required_capabilities`, completes on subprocess exit, DELETEs the `MeshAgent` (FK `ON DELETE SET NULL` on `agentrun.mesh_agent_id` preserves audit).
- **Capability enforcement**: a coarse vocabulary (`shell:read/write`, `fs:read/write`, `vault:read/write`, `edgeplane:read/write`, `web:fetch`, `gh:read/write`) maps to `claude -p --allowed-tools` patterns. Dispatchers declare blast radius via `MeshTask.required_capabilities` (JSON array of capability names).
- **Audit invariant**: `Agent` identity is reused per parent profile (never created per task); `MeshAgent` rows are ephemeral; `AgentRun` rows are permanent and carry the durable trace (`total_cost_cents`, `idempotency_key`, `parent_run_id`, `runtime_session_id`). Multiple concurrent subagents share one `Agent` and have N concurrent `MeshAgent` projections — exactly the pattern `entities.md` line 96 anticipates.
