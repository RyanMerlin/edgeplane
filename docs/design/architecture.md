# Architecture

## Core Runtime
- **Postgres (+ pgvector)**: authoritative coordination state (missions, klusters, tasks, approvals, profiles, ledger).
- **S3-compatible object storage**: active artifact/document bytes and workspace file persistence.
- **Git publication layer**: explicit memory-of-record projection for routable entities.
- **MCP/API control plane**: policy-gated tool execution, publish planning, and audited mutations.

## Persistence Model
- **Coordination truth stays in MissionControl (Postgres)**.
- **Git is a projection sink**, never the authority for mission ownership, approvals, or governance.
- **Mission-scoped routing** controls where publication events land:
  - `repo_connections`
  - `repo_bindings`
  - `mission_persistence_policies`
  - `mission_persistence_routes`
  - `publication_records`

## Publish Flow
1. Mutation enters ledger (`pending`) in Postgres.
2. Approval/policy checks run in MissionControl.
3. Route resolver picks binding/repo/branch/path from mission policy.
4. Provider adapter acquires server-side credential.
5. Publisher writes canonical file(s) to Git and records commit provenance.
6. Ledger/publication records are marked and queryable via API/MCP.
