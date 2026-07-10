---
title: Persistence Model
description: How EdgePlane stores state across Postgres and Git today, with S3-compatible object storage on the roadmap.
---

EdgePlane's target design is three complementary persistence layers. Two are live today — Postgres and Git. The third, S3-compatible object storage, is planned but not implemented; artifact content lives inline in Postgres in the meantime. Understanding the boundaries (current and intended) prevents architectural confusion.

## The Three Layers

### PostgreSQL — Structured State

The operational database. All structured entities live here:

- Domains, missions, tasks, meshtasks
- Artifacts, **including content** — bytes are stored inline (`content_b64`), not in an external object store
- Agents, mesh agents, agent runs
- Domain ownership (`owners`/`contributors` columns on the `domain` row)
- Ledger events
- Publication records

Postgres is the coordination substrate — fast, queryable, and scoped by domain membership. **Coordination truth stays in Postgres, not Git.**

`pgvector` is not currently used — there is no embedding generation or vector search anywhere in the stack today. Hybrid/semantic search across entities is on the roadmap, not implemented.

### S3-Compatible Object Storage — Working File Store (planned)

The intended design stores artifact bytes, document content, and workspace files in S3-compatible object storage, keyed by:

```
domains/{domain_id}/missions/{mission_id}/{entity}/{filename}
```

**This is not implemented yet.** There is no S3/object-storage client in `edgeplane-tower` or `edgeplaned` today — artifact content lives inline in the `artifact.content_b64` Postgres column (see above). The `artifact` table does carry a `storage_backend` column and the MCP tool `get_artifact_download_url` is named for a signed-URL flow, but the write path to an external object store has not been built. Treat this section as the target architecture, not current behavior.

### Git — Long-Term Memory of Record

When a mutation is authorized and published, it is committed to Git. Artifact provenance metadata (repo, branch, path, commit hash) is written back to Postgres, creating a permanent link between the operational record and the historical record.

**Git is a projection sink, never the authority** for domain ownership or coordination state. Those live in Postgres.

## The Publish Flow

```
1. Mutation enters ledger (status: pending) in Postgres
2. Authorization check — caller must be a domain owner/contributor or an admin (there is no separate approval-token workflow; see [Domain Access Control](/guides/governance-and-approvals/))
3. Route resolver picks binding / repo / branch / path from the domain's persistence policy (MCP `resolve_publish_plan`)
4. Provider adapter acquires server-side credential
5. Publisher writes canonical file(s) to Git
6. Commit provenance recorded — repo, branch, path, commit hash
7. Ledger / publication records marked and queryable via API / MCP

The `provision_domain_persistence` MCP tool creates/updates the connection, binding, and policy routes for a domain in one call.

The data model supporting this (actual Postgres table names):

| Table | Purpose |
|-------|---------|
| `repoconnection` | Git provider credentials |
| `repobinding` | Domain → repo mappings |
| `domainpersistencepolicy` | Publication routing policy (default binding per domain) |
| `domainpersistenceroute` | Entity-type → path routing |
| `publicationrecord` | Completed publication audit trail |
| `ledgerevent` | Pending/published mutation ledger |

## The Full Flow

```
Agent produces artifact
     │
     │ Mutation recorded (content stored inline)
     ▼
Postgres (structured state + working store today)
missions, tasks, artifacts (content_b64), domain ownership, ledger
     │
     │ Authorized + routed via domain persistence policy
     ▼
Git (memory of record)
commit → provenance written back → full chain of custody
```

S3-compatible storage as a separate working-file tier (`domains/{domain_id}/missions/{mission_id}/...`) is the target design but is not built yet — see the planned-storage note above.

## Why This Separation Exists

**Postgres for coordination:** structured entities need transactions, membership-scoped access, and fast point queries. It's also where artifact content lives today, inline.

**S3 for working files (planned):** artifact bytes are large and frequently mutated during active work; scoping them per domain/mission in an object store rather than Postgres blobs is the intended design once the write path is built.

**Git for memory of record:** Git provides immutable, auditable, reproducible history outside the control plane. If the Postgres instance were lost, Git contains the full audit trail of every published mutation. This creates an organizational knowledge base that survives infrastructure changes.

## Querying the State

### Via API / MCP

```bash
edgeplane mission list --json
edgeplane task list --mission-id <id> --json
# or via MCP tools: list_mesh_tasks, get_mesh_task
```

### Semantic Search (planned)

Hybrid full-text + vector search across tasks, docs, and missions is on the roadmap. There is no `pgvector` usage, embedding generation, or `--search` flag today — don't rely on this.

### Publication Status

```bash
edgeplane data tools call --tool get_publication_status --payload '{"domain_id":"<id>"}'
# or via MCP: get_publication_status, resolve_publish_plan
```

## See Also

- [Architecture: System Overview](/architecture/overview/) — how the components fit together
- [Concepts: Domains, Missions & Tasks](/concepts/domains-missions-tasks/) — the organizational model
- [Guides: Deployment](/guides/deployment/) — running Postgres and Git in production
