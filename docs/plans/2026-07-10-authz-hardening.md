# Authorization Hardening — Plan

**Status:** design approved; implementation not started.
**Branch:** `fix/claims-integrity-and-onboarding` (may split to `fix/authz-hardening`).
**Owner:** Merlin + Aria (engineer).
**Origin:** claims-integrity audit (2026-07-09/10). A tower authorization-surface audit
turned up (1) pre-existing broken-access-control gaps affecting *all* principals and
(2) the node-credential full-trust model. Merlin's call: **fix the access-control gaps
first**, then scope node credentials by domain.

This is a **security-seam** effort. Per repo policy every phase ends with cross-boundary
red-team probes against a live tower and a `rust-reviewer` (Opus) pass before merge.

---

## Evidence base — the five authorization mechanisms in the tower

All routes are served under `/api` (`server.rs` `.nest("/api", authed)`), behind `require_auth`.
Domain authorization is **not** uniform — five distinct mechanisms coexist:

1. **Shared default-deny domain check** — `authorized_for()` (`auth.rs:364`) via
   `authorized_for_domain` / `authz_domain` (`routes/authz.rs:21`) / `mcp_authz_domain`
   (`routes/mcp.rs:292`). Grants if: admin **OR** `auth_type=="node"` (blanket, `auth.rs:373`)
   **OR** domain ∈ `principal.domain_scope` **OR** subject ∈ owners/contributors.
   Honors agent `domain_scope`. Governs ~39 endpoints (4 `domains.rs`, 18 `work.rs`, 17 MCP tools).
2. **Local reimplementations** — `tasks.rs::domain_access`, `missions.rs::domain_readable/…`,
   `docs.rs`/`artifacts.rs`/`explorer.rs::can_read_domain/can_write_domain`,
   `persistence.rs::is_domain_owner`. Check admin + owners/contributors (± public) but
   **ignore `domain_scope` and `auth_type`**. A scoped agent that is not an owner/contributor
   is **denied** here even in its own domain — an inconsistency with mechanism (1).
3. **Node-self identity gate** — `runtime.rs::is_authorized_for_node` (`3084`): admin, or
   `auth_type=="node" && subject=="node:{id}"`, or owner. Node self-scoping, no domain concept.
4. **Owner-subject-only gate** — most of `runtime.rs`'s node-fleet surface
   (`heartbeat_node`, `get_node_config`, jobs, leases, execution-sessions) filters
   `WHERE owner_subject = principal.subject`. A node's own JWT (`node:{id}`) does not match.
5. **No gate at all** — `require_auth` only; `principal` extracted and ignored. Any
   authenticated principal (any `auth_type`, any domain) has full access.

---

## Open items to resolve before/within the work

- **[O1] Node-fleet credential (blocks node-scoping, not the IDOR work).** `edgeplaned`
  calls `/runtime/nodes/{id}/heartbeat` and `/attach-secret` itself (daemon.rs), but those
  are owner-gated (mechanism 4) and reject node-self JWTs. Either the daemon presents a
  non-node credential for them, or node self-heartbeat is already broken. Resolve by tracing
  the daemon's live token per call (`edgeplaned-bin/src/daemon.rs` token setup) and/or a live probe.
- **[O2] Two-mechanism inconsistency (blocks IDOR fixes).** Mechanism (2) ignores
  `domain_scope`; mechanism (1) honors it. Any new domain check MUST use the shared
  `authorized_for_domain` (mechanism 1) so scoped agents/nodes still pass. Decide whether to
  also migrate the mechanism-(2) files onto the shared helper (recommended, but a behavior
  change for agents that rely on `domain_scope`).
- **[O3] `is_full_trust` second axis.** `session|node` bypass at `authz_task_owner`,
  claim-on-behalf, cross-agent heartbeat/status/profile, mesh impersonation, inbox reads.
  Node-scoping bounds these to scoped domains (good) — keep the axis (nodes legitimately
  manage hosted agents), do not remove it.

---

## Workstream 1 — Broken-access-control remediation (PRIORITIZED)

Fix mechanism-(5) "no gate" and mechanism-(2) fail-open holes. Use the **shared**
`authorized_for_domain` so scoped agents/owners/admins (and, until Workstream 2, nodes) pass;
cross-domain non-members are denied. **Every fix requires a caller-safety check** (web UI, CLI,
daemon, agents) before landing.

### Group A — no-gate cross-domain reads (highest ROI, lowest breakage risk)
Add `authz_domain` (resolve domain from the path object first):
`work.rs` `get_task`, `task_graph`, `get_task_progress`, `list_gates`, `list_tasks` (by mission),
`list_domain_agents`, `list_domain_messages`, `domain_roster`, `get_agent`, `get_agent_messages`;
`tasks.rs::list_tasks_by_mission`; `missions.rs` `get_mission_brief_flat`.
Caller-safety: daemon/agent read within their scope → pass; the closed hole is cross-domain reads.

### Group B — `agents.rs` (no authz on any handler) + `attach_domain`
`attach_domain` (moves any agent into any domain) → owner/admin of the target domain.
Agent CRUD/messaging → domain read/write as appropriate. **Highest breakage risk** — verify the
web dashboard and daemon call these as owner/session (likely) before gating. `put_mission_brief_flat`
(overwrite any brief by id) → domain-write.

### Group C — NULL-domain fail-open (`artifacts.rs`, `docs.rs`)
`create/update/publish_{artifact,doc}` skip the check when the parent mission `domain_id` is NULL.
Change NULL-domain to deny (or admin-only), never allow.

### Group D — `remotectl::create_launch`
No gate + mints a session token with caller-supplied unclamped `ttl_hours`/`capability_scope`.
Add authz; clamp TTL and capability scope to server maxima. **Privilege-escalation surface — careful.**

### Group E (lower) — unvalidated caller-supplied `domain_id`
`runtime::create_job`, `budgets::record_usage_batch`: validate the caller may act on the supplied domain.

### Group F (lower) — `search.rs` `LIKE`-based readability filter → exact-match membership.

---

## Workstream 2 — Node domain-scoping (design approved: DYNAMIC)

Confine a `node` principal to the domains it actually operates, replacing the `auth.rs:373`
blanket `return true`.

- **Scope source (dynamic):** domains where the node hosts assigned agents —
  `SELECT DISTINCT domain_id FROM meshagent WHERE runtime_node_id = ?` — resolved at authz
  time with a short per-node cache. Chosen over static-in-`NodeClaims` because node tokens
  rotate every 12 h and a freshly-assigned domain must be reachable immediately (for
  `mint_agent_token`).
- **Bootstrap is clean:** `assign_node_agent` is owner-gated and creates the `meshagent` row,
  so *owner assigns → row exists → node scope includes the domain → node mints the agent token*.
  The node never bootstraps its own access.
- **Mechanism:** populate `Principal.domain_scope` for nodes in the auth extractor (reuse the
  agent path), then change `auth.rs:373` from `return true` to the scope check. Infra/self
  paths (register, rotate-token, node-self roster/notify) use other mechanisms — untouched.
- **entities.md § Domain change (lands FIRST, per the entity HARD RULE):** replace
  *"is a `node` (full-trust infra)"* with: a node's `domain_scope` is the set of domains in
  which it currently hosts assigned agents (from `meshagent`) — trusted only within the
  domains it operates, not globally.

Affects exactly the ~39 mechanism-(1) endpoints; a stolen node token is confined to that node's
operating domains instead of the whole fleet.

---

## Validation strategy (both workstreams)

- **Unit:** flip `auth.rs::node_is_full_trust_authorized_anywhere` → `…_only_in_scope`; test the
  node-scope resolver; test each new domain gate (member allow, non-member deny, public-read where applicable).
- **Integration:** full node/agent lifecycle still works — register → owner-assign → node-mint →
  agent claim/complete → publish. Daemon flows (`task_worker`, `task_loop`, `bootstrap`) must not regress.
- **Red-team (mandatory, live tower):** with real tokens, a domain-A principal/node must 403 on
  every domain-B endpoint across dispatch/ledger/stream/workspace and the ~39 scoped endpoints.
- **Review:** `rust-reviewer` (Opus) on each diff before merge. `cargo clippy --all-targets -D warnings`
  + `nextest` locally (CI parity).

---

## Sequencing

0. **[O1]/[O2] traces** — daemon per-call credential; confirm the shared-helper migration is safe.
1. **Workstream 1** — Group A → C → B → D (→ E/F), each with caller-safety + tests + red-team + review.
2. **Workstream 2** — entities.md § Domain → resolver → `auth.rs:373` → tests → red-team → review.
3. Optional consolidation: migrate mechanism-(2) files onto the shared `authorized_for_domain`.

Do not batch across groups — land each with its own tests and review so a regression is bisectable.
