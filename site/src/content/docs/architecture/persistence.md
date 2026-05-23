---
title: Persistence Model
description: How MissionControl stores state across Postgres, S3-compatible object storage, and Git.
---

MissionControl uses three complementary persistence layers. Each serves a specific role in the information lifecycle. Understanding the boundaries prevents architectural confusion.

## The Three Layers

### PostgreSQL + pgvector — Structured State

The operational database. All structured entities live here:

- Domains, missions, tasks, meshtasks
- Artifacts (metadata — bytes are in S3)
- Agents, mesh agents, agent runs
- Roles and domain memberships
- Governance policies and approval records
- Ledger events
- Publication records

Postgres is the coordination substrate — fast, queryable, role-scoped, vector-indexed for semantic search. **Coordination truth stays in Postgres, not Git.**

pgvector enables hybrid search across all entities — tasks, documents, and missions are indexed for similarity queries alongside standard relational lookups.

### S3-Compatible Object Storage — Working File Store

Artifact bytes, document content, workspace files, and skill bundles are stored in S3-compatible object storage, not inline in the database.

Storage path layout:

```
domains/{domain_id}/missions/{mission_id}/{entity}/{filename}
```

Agents can read, write, and iterate on file content without polluting the structured state database. Storage scales independently. Any S3-compatible backend works — AWS S3, MinIO, or self-hosted alternatives — with no code changes.

**S3 is not optional infrastructure. It is where active work lives.**

The Docker Compose development stack ships with an S3-compatible backend bundled. No external infrastructure is required to run locally with full file persistence.

### Git — Long-Term Memory of Record

When a mutation is approved and published, it is committed to Git. Artifact provenance metadata (repo, branch, path, commit hash) is written back to Postgres, creating a permanent link between the operational record and the historical record.

**Git is a projection sink, never the authority** for domain ownership, approvals, or governance. Those live in Postgres.

## The Publish Flow

```
1. Mutation enters ledger (status: pending) in Postgres
2. Approval / policy checks run in MissionControl
3. Route resolver picks binding / repo / branch / path from domain policy
4. Provider adapter acquires server-side credential
5. Publisher writes canonical file(s) to Git
6. Commit provenance recorded — repo, branch, path, commit hash
7. Ledger / publication records marked and queryable via API / MCP
```

The data model supporting this:

| Table | Purpose |
|-------|---------|
| `repo_connections` | Git provider credentials |
| `repo_bindings` | Domain → repo mappings |
| `domain_persistence_policies` | Governance rules for publication |
| `domain_persistence_routes` | Entity-type → path routing |
| `publication_records` | Completed publication audit trail |

## The Full Flow

```
Agent produces artifact
     │
     ▼
S3 (working store)
domains/{domain_id}/missions/{mission_id}/...
     │
     │ Mutation recorded
     ▼
Postgres (structured state)
missions, tasks, artifacts, roles, ledger
     │
     │ Approval granted
     ▼
Git (memory of record)
commit → provenance written back → full chain of custody
```

## Why This Separation Exists

**Postgres for coordination:** structured entities need transactions, role-based access, vector indexing, and fast point queries. File storage in Postgres doesn't scale.

**S3 for working files:** artifact bytes are large, frequently mutated during active work, and need to be scoped per domain/mission. Postgres blobs don't scale; S3 does.

**Git for memory of record:** Git provides immutable, auditable, reproducible history outside the control plane. If the Postgres instance were lost, Git contains the full audit trail of every approved, published mutation. This creates an organizational knowledge base that survives infrastructure changes.

## Querying the State

### Via API / MCP

```bash
mc missions list --json
mc tasks list --mission-id <id> --json
# or via MCP tools: list_pending_ledger_events, get_entity_history
```

### Semantic Search

All entities are indexed in pgvector. Use `search_tasks`, `search_missions`, or the `--search` flag on CLI commands for hybrid full-text + vector search.

### Publication Status

```bash
mc data sync status --mission-id <id>
# or via MCP: get_publication_status, resolve_publish_plan
```

## See Also

- [Architecture: System Overview](/missioncontrol/architecture/overview/) — how the components fit together
- [Concepts: Domains, Missions & Tasks](/missioncontrol/concepts/domains-missions-tasks/) — the organizational model
- [Guides: Deployment](/missioncontrol/guides/deployment/) — running Postgres and S3 in production
