# Security Hardening — Seam 1 + Seam 2 (v0.15.0, 2026-06-19)

## Release

- **Version:** v0.15.0
- **Deployed:** 2026-06-19
- **PRs:** #53 (Seam 1), #56 (Seam 2), #57 (fix), #58 (fix), #59 (release), #62 (hardening)

## Security model shipped

### Seam 1 — Domain authorization (PR #53)

Default-deny `authorized_for_domain` guard applied to every privileged dispatch, ledger, and stream handler in edgeplane-tower (REST + MCP). Also: per-task lease ownership enforcement (`authz_task_owner`) on lifecycle mutations; admin-gated `global_sse`; closed an artifact-exfil gap in `get_artifact_download_url`.

**Authorization predicate:**
```
authorized_for_domain(principal, domain) :=
    principal.is_admin
    OR principal.auth_type == "node"          // full-trust infra
    OR domain.id ∈ principal.domain_scope     // per-agent JWT home domain(s)
    OR principal.subject ∈ domain.owners
    OR principal.subject ∈ domain.contributors
```

**Principal trust tiers:**

| Principal | `auth_type` | Tier |
|-----------|-------------|------|
| Human session (OIDC) | `session` | Full-trust |
| Node / daemon (node JWT) | `node` | Full-trust |
| Service account | `service_account` | Scoped |
| Agent (per-agent JWT) | `agent` | Scoped |

Node principals are full-trust (first-party infra, single-operator posture). Node→managed-domains scoping is deferred to multi-tenant.

### Seam 2 — Per-agent identity (PR #56)

Each enrolled agent receives its own short-lived (12 h), domain-scoped per-agent JWT (`AgentClaims`) instead of the shared `EP_AGENT_TOKEN`. Tokens are RS256-signed, revocable via the `agenttoken` table (migration `0010`), and fail-closed (revocation check returns denied on DB error). `claim` and `progress` are attributed to the authenticated agent in both REST and MCP surfaces. The mint endpoint (`POST /work/agents/{id}/token`) is full-trust/admin-gated — agents cannot mint peer tokens.

The daemon injects each agent's own token as its `EP_AGENT_TOKEN`, with a graceful fail-closed fallback to the shared daemon token if minting is unavailable (PR #57/#58: minting is restricted to token-consuming runtimes — `claude_code`).

**Live-validated:** full lifecycle tested post-deploy: enroll → `auth_type:agent` → domain-scope 403 on cross-domain → revoke → 401.

---

## Red-team (2026-06-19)

Three adversarial lenses reviewed the v0.15.0 model: cross-domain mutation bypass, token forgery/escalation, and intra-domain/read-side gaps.

### Verified sound

| Threat | Verdict |
|--------|---------|
| Cross-domain mutation bypass | No bypass — `authorized_for_domain` on every write; node↔agent tokens non-decodable (`deny_unknown_fields`, RS256-pinned) |
| Token forgery / escalation | No path — RS256-pinned; `is_admin` hardcoded `false` for agent/node; fail-closed revocation incl. DB-error |
| Lease bypass | No bypass — `authz_task_owner` enforced on lifecycle mutations |
| SQLi / panic-DoS in auth/authz paths | No vectors found |
| Daemon fallback silently open | No — `cmd.env_remove` on mint failure; fails closed |

### Findings (all closed in PR #62)

| Severity | Finding | Resolution |
|----------|---------|------------|
| HIGH | `list_mesh_messages` unauthenticated — system-wide message body broadcast readable by any valid token | Domain-authz guard added |
| MEDIUM | 5 read-side MCP arms (`get_domain_northstar`, `resolve_publish_plan`, `get_overlap_suggestions`, `list_mesh_tasks`, `get_mesh_task`) lacked domain authorization | Domain-authz guard added to each |
| MEDIUM | `progress_mesh_task`, `append_progress`, `unblock_task`, `create_gate`, `agent_heartbeat`/`set_agent_status`/`update_agent_profile` lacked `authz_task_owner`/self-identity checks | Owner/self checks added |
| MEDIUM | `send_mesh_message` sender-spoof — no sender-identity verification | Sender identity enforced |
| LOW | Agent-delete did not revoke the agent's JWT | Revoke-on-agent-delete added |

---

## Hardening (PR #62)

Merged 2026-06-19. `harden(tower): close red-team read-side + intra-domain authz gaps`

- Domain-authz'd the 6 read-side MCP arms including the HIGH `list_mesh_messages` leak.
- Added `authz_task_owner`/self-identity checks to all intra-domain mutation paths.
- Closed `send_mesh_message` sender-spoof.
- Daemon token fallback explicitly fail-closed (`cmd.env_remove`).
- Revoke-on-agent-delete.

---

## Open issues and deferrals

| # | Description |
|---|-------------|
| #55 | Mid-life token refresh — supervised agent alive >12 h without respawn will 401 at token expiry; fail-safe (heals on respawn) |
| #60 | Non-idempotent home-domain backfill produces a WARN on first run |
| #61 | Test coverage for the #62 hardening |
| #54 | Progress owner-gate — **RESOLVED** by #62 |

**Deferred (no live bug, no second tenant):**
- Seam 3 — nftables egress + cgroup enforcement (sandbox built, unwired)
- §5 trust-tier dispatch-template split (no untrusted dispatch-token consumer yet)
- `expires_at` `timestamp without time zone` → `timestamptz` nit (latent, no live bug)
- Node → managed-domains scoping (multi-tenant refinement)
