# Authorization Hardening — Plan

**Status (2026-07-14):** **Workstream 1 and Workstream 2 are both COMPLETE.** W1: all groups (A, B, C, D, E, F, G) shipped, deployed, reviewed (#102/#103/#104, tower `sha-e2666eb`). W2: node domain-scoping shipped, deployed, and prod red-teamed (#105 docs, #106 code, tower `sha-34386f6a`), plus a follow-up fix for a gap an adversarial review surfaced in the shipped W2 diff (#107, `assign_node_agent` missing domain authz, tower `sha-ffdda81`). **Remaining: the delegated-session design item only** (see below) — owner decision pending; a scoping review recommends a single contained design-and-patch, not a full architecture-review process.
**Owner:** Merlin + Aria (engineer).
**Origin:** claims-integrity audit (2026-07-09/10). A tower authorization-surface audit
turned up (1) pre-existing broken-access-control gaps affecting *all* principals and
(2) the node-credential full-trust model. Merlin's call: **fix the access-control gaps
first**, then scope node credentials by domain.

This is a **security-seam** effort. Per repo policy every phase ends with cross-boundary
red-team probes against a live tower and a `rust-reviewer` (Opus) pass before merge.

## Shipped log (all squash-merged to main + auto-deployed; tower `sha-7b9d5a8`)

| PR | Scope | Red-team |
|----|-------|----------|
| #96 | mission-brief flat GET/PUT read+write IDOR | ✅ 20/20 run |
| #97 | **Group A** — 13 cross-domain read IDORs | ✅ 20/20 run |
| #98 | **Group E (feedback)** + **Group G (ingestion)** reads | ✅ 20/20 run |
| #99 | **Group B** — delete 7 dead `agents.rs` handlers + gate 4 write mutations | integration-tested |
| #100 | **Group B Tier-3** — gate `list_messages`/`send_message`/`attach_domain` + `is_self_control_plane_agent` bridge | ✅ 5/5 run |
| #102 | **Group D** — gate `create_launch` (reject node/agent) + clamp `ttl_hours`/`capability_scope` | ✅ live 7/7 (agent→403, admin→201) |
| #103 | **Group C** (NULL-domain fail-open artifact/doc writes) + **Group E-remainder** (`create_job` + `record_usage_batch` domain gate) | ✅ live 7/7 (E1/E2 cross-domain→403, own-domain→201/200) |
| #104 | **Group F** — search readability substring-LIKE → exact membership (`authorized_for`) | integration-tested (7/7, incl. reverted-code regression proof) |
| #105 | docs — `entities.md § Domain` update, lands before W2 code per the entity HARD RULE | n/a (docs only) |
| #106 | **Workstream 2** — dynamic per-node domain scope, removes `auth.rs` blanket node full-trust; cache invalidated on assign/revoke/detach/enroll/delete | ✅ live 6/6 (own-domain→200, foreign-domain→403, admin control→200) |
| #107 | **W2 follow-up** — `assign_node_agent` never authorized the caller against the target domain before seeding a `meshagent` row, reopening W2's own fix; gated via `authz_domain`, mirroring `enroll_agent` | ✅ live 6/6 (deny-unauthorized→403, deny-read-after-deny→403, allow-owned→201, admin-bypass→201) |

**Live red-team: 25/25 (reads) + 7/7 (D+E write-path, 2026-07-14, tower `sha-caec5a4`) + 6/6 (W2 node-scoping, 2026-07-14, tower `sha-34386f6a`) + 6/6 (W2 follow-up, 2026-07-14, tower `sha-ffdda81`)** — real domain-scoped **agent**, **node**, and throwaway **service-account** tokens vs the prod tower; all cross-domain/priv-esc → 403, all legitimate controls → 200/201; throwaway domains/agents/nodes/launches/service-accounts cleaned up. Groups C + F verified via real-JWT integration tests (seeding NULL-domain missions / substring-owner scenarios on prod is impractical) + deployed-sha == tested-code. W2: registered a throwaway node, assigned it an agent in domain D1 (creating the `meshagent` row the resolver reads), confirmed the node reaches D1 (`agents`/`roster` → 200) but is denied in a domain D2 it doesn't operate (`agents`/`messages`/`roster` → 403) — the exact regression the blanket-trust removal targets. W2 follow-up: a throwaway SA registered its own node, was denied assigning it into a domain it doesn't own (403) and denied reading that domain afterward (403, confirming no scope was granted), succeeded assigning into a domain it owns (201), and admin retained the ability to assign the same node into the other domain (201) — proving the fix targets exactly the caller-authorization gap, not legitimate assignment.

### Deferred residuals (low-risk for single-tenant; NOT bugs to chase now)
- **`send_message` intra-domain impersonation** — the cross-domain case is gated; closing intra-domain fights the `signal.rs` shared-sender-peer design (would 403 agent-type `edgeplane agent signal`). Needs a signal-auth redesign, not a quick fix.
- **`get_agent`/`list_agents`/`create_agent`** — intentionally left as operational reads / self-register (single-tenant); revisit if multi-tenant.
- ~~Daemon `/agents/{id}/heartbeat|status|notify` 404 bug~~ — **NOT A BUG** (verified 2026-07-14): `solo_supervisor.rs` already uses correct `/work/agents/…` paths.

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

- **[O1] Node-fleet credential — RESOLVED (2026-07-10).** The daemon has two modes
  (`daemon.rs:550` `is_node_credential_mode = active_profile.is_none() && node_id.is_some()`):
  - **Profile mode** (`edgeplane auth login`): `cfg.token` = owner **OIDC session token**
    (subject = owner). `BackendClient` (`edgeplaned-core/src/client.rs`) carries it; the
    per-agent `task_loop` reads/claims as a full-trust **session**. `heartbeat_node`/
    `attach-secret`/`rotate-token` pass their `owner_subject = principal.subject` filter.
  - **Node-credential mode** (headless system-service, `node.json`): `cfg.token` = **node JWT**
    (`sign_node_jwt` sets `sub = "node:{id}"`, `jwt.rs:32`). `task_loop` reads/claims pass via the
    `auth.rs:373` blanket node-trust. **BUT** `heartbeat_node` (`runtime.rs:1055`),
    `get_node_attach_secret`, and `rotate_node_token` gate on `owner_subject` (= the human owner,
    copied from the join-token row at registration, `runtime.rs:818`), which `node:{id}` can never
    match → **node self-heartbeat / attach-secret self-heal / token-rotation are a pre-existing
    latent bug in node-credential mode** (heartbeat 404 logged non-fatally at `daemon.rs:523`;
    rotation failure would expire the node token after its 24 h TTL). Out of scope for this
    workstream (separate correctness bug), but it **intersects Workstream 2** — track separately.
  - **Group-A consequence:** in *both* modes the task-loop reads pass `authz_domain`
    (owner → full-trust/member; node → blanket-trust). Adding `authz_domain` does **not** break
    the daemon. IDOR work is unblocked.
- **[O2] Two-mechanism inconsistency — RESOLVED (2026-07-10).** Confirmed by reading every
  mechanism-(2) reimplementation (`tasks.rs::domain_access`, `missions.rs::domain_readable/
  writable/ownable`, `docs.rs`/`artifacts.rs`/`explorer.rs::can_read/write_domain`,
  `persistence.rs::is_domain_owner`): **all check only `is_admin` + owners/contributors (± public
  visibility); none reference `domain_scope` or `auth_type`.** Therefore:
  - **New Group-A/B/C/D gates MUST use the shared `authz_domain` (mechanism 1)** — a local
    mechanism-(2)-style check would deny the node-credentialed daemon (node → no blanket trust in
    mechanism 2) and scoped agents. **Locked: use `authz_domain`.**
  - **Migrating existing mechanism-(2) files onto the shared helper is NOT a clean swap and is
    deferred (sequencing step 3, optional).** Two behavior changes: (a) it *widens* — scoped
    agents/nodes would gain access they're currently denied (arguably more correct); (b) it
    *narrows* — `authorized_for` has **no public-visibility branch**, so migrating READ paths
    would remove public-domain read access. A migration must add a public-read-aware variant, or
    it regresses public domains. Do not migrate as part of the IDOR fix.
  - **Group-A public-visibility note:** Group-A endpoints are `work.rs` operational reads
    (task graph, roster, get_agent) that were never gated and never consulted visibility.
    `authz_domain` is default-deny (no public branch) — correct for operational endpoints
    (cross-domain task reads should not be public). Flagged as a reversible decision; add a
    public-read branch only if a public domain must expose these operational reads.
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
`list_domain_agents`, `list_domain_messages`, `domain_roster`, `get_agent`, `get_agent_messages`,
`list_mission_messages`;
`tasks.rs::list_tasks_by_mission`; `missions.rs` `get_mission_brief_flat`.
Caller-safety: daemon/agent read within their scope → pass; the closed hole is cross-domain reads.
**`list_mission_messages` (`GET /work/missions/{id}/messages`) was missed by this enumeration** —
surfaced by the Codex (gpt-5.5, xhigh) adversarial review on 2026-07-10 after the Opus rust-reviewer
passed it. Same class; its route-family siblings (`mission_stream` GET, `send_mission_message` POST)
were already gated, so it was the lone hole. Now gated via `authz_by_mission` + deny/allow tests.

### Group B — `agents.rs` (no authz on any handler) + `attach_domain`
`attach_domain` (moves any agent into any domain) → owner/admin of the target domain.
Agent CRUD/messaging → domain read/write as appropriate. **Highest breakage risk** — verify the
web dashboard and daemon call these as owner/session (likely) before gating.
~~`put_mission_brief_flat` (overwrite any brief by id) → domain-write.~~ **DONE in PR #96**
— the flat-brief `archived_at` correctness fix would have unmasked this write IDOR (and the
`get_mission_brief_flat` read IDOR), so #96 gates both flat endpoints on the mission's domain
via `authz_domain`. Removed from Group B scope.

### Group C — NULL-domain fail-open (`artifacts.rs`, `docs.rs`) — ✅ SHIPPED (#103)
`create/update/publish_{artifact,doc}` skipped the check when the parent mission `domain_id` was
NULL. Fixed: replaced `if let Some(mid) && !can_write` with `match { Some => can_write_domain,
None => require is_admin }` (mirrors delete/read handlers). NULL-domain writes are now admin-only.

### Group D — `remotectl::create_launch` — ✅ SHIPPED (#102; prod red-team 7/7)
Was: no gate + minted a full-trust `usersession` token with caller-supplied unclamped
`ttl_hours`/`capability_scope` (raw `ttl_hours` into `chrono::Duration::hours` → `i64::MAX`
overflow-panic; negatives → past-dated tokens). Fix: reject `node`/`agent` auth_types (403,
mirrors `create_join_token`); clamp `ttl_hours` to `[1, 87_600]`; structurally clamp
`capability_scope` (≤64 entries, ≤4096 bytes, whole-entry trim — `capability_scope` is not
yet enforced at auth time, so this is defense-in-depth, not a semantic allowlist). Tests:
real agent/node JWTs → 403, session → 201, TTL + both scope-clamp paths verified. Opus
review: SHIP.

### Delegated-session design item — ⬜ NEW (surfaced investigating Group D; NOT a quick gate)
`create_launch` is fixed, but two sibling paths let a **non-full-trust credential mint a
full-trust `session` token** for its own subject — the same escalation class:
- **`create_session` (`POST /auth/sessions`)** — ungated; ANY authenticated principal mints
  a session. But this is *load-bearing*: the CLI `edgeplane auth login --non-interactive`
  (`edgeplane/src/auth.rs:286`) deliberately exchanges `EP_AGENT_TOKEN` → a session, a **live
  fleet bootstrap flow** (verified: this host's `state/session.json` was minted for an opaque
  non-human subject). A blunt node/agent reject-gate here would **break bootstrap** — do NOT
  copy the Group D gate.
- **`create_launch` allowing `service_account`** — a `service_account` (itself not full-trust)
  can still mint a full-trust session. Consistent with `create_join_token`; flagged for
  conscious sign-off, same root cause.

Root cause: a minted `usersession` always re-authenticates as `auth_type="session"` →
`is_full_trust=true`, regardless of the presenting credential's privilege. Proper fix is a
**delegated/scoped session** that inherits (never exceeds) the presenting credential's scope,
not a per-endpoint type gate. Design alongside Workstream 2 (node domain-scoping) — both hinge
on `Principal` carrying real scope. Owner decision pending (Merlin: track separately).

**Scoping review (2026-07-14, codex gpt-5.5 high, independently verified before treating as
fact):** recommends a **single focused design-and-patch, not a full multi-workstream
architecture-review program** — the privilege reinterpretation is centralized (only one
construction site for `auth_type: "session"`, `auth.rs:315`), so the runtime surface is limited
even though it needs a short explicit design first. Findings that sharpen the scope:
- **Three** production paths mint a `usersession`, not one: `/auth/sessions` (broad, load-bearing
  for bootstrap), `/remotectl/launches` (blocks node/agent, still allows `service_account`), and
  OIDC web login (`routes/oidc_web.rs`, the intended interactive-human path — correctly full-trust
  and out of scope for the fix).
- **Broader blast radius than `is_full_trust` alone:** `create_join_token` and `rotate_join_token`
  (`routes/runtime.rs`) reject `node|agent` by raw `auth_type`. A node/agent that first exchanges
  through `/auth/sessions` presents as `"session"` and bypasses these gates too, not just the
  per-task lease bypass.
- **Do not** resolve a delegated session back to `auth_type: "node"`/`"agent"` — that collides
  with node-self (`runtime.rs` reconcile/notify) and agent-self (`authz.rs`) special-case checks
  elsewhere. The model needs a separate "bearer type" (stays `"session"` for `ep_*` token
  mechanics) vs. "effective delegated authority" (what the session should actually be trusted for).
- **Fold in the #107 adjacent finding:** `usersession.capability_scope` is written by
  `issue_session_token` on launch but never read back at session-resolution time — it's dead as
  an authorization mechanism today. The delegated-session fix is the natural place to either make
  it load-bearing or replace it; don't design around it as if it already worked.
- Minimum acceptance bar proposed: agent/node/service-account credentials can still use
  `/auth/sessions` where bootstrap depends on it; the resulting session never gains more
  `is_full_trust`/domain-scope/join-token/launch authority than the presenting principal had;
  OIDC/human sessions stay full-trust; tests cover direct and exchanged credentials for agent,
  node, service-account, and OIDC/session cases across `/auth/sessions`, `create_join_token`,
  `create_launch`, and at least one `is_full_trust` task/MCP path. No `entities.md`/Domain rework
  needed unless delegated sessions must track live node domain reassignment or inherit
  source-token expiry — those are callable-out-but-deferrable, not blockers.

### Group E (lower) — unvalidated caller-supplied `domain_id` — ✅ SHIPPED (#103; feedback in #98)
`runtime::create_job` + `budgets::record_usage_batch` now validate the caller may act on the
supplied domain via `authz_domain` (create_job: empty domain → admin-only, mirroring Group C;
usage batch: authorize each distinct non-empty domain once, None/empty allowed as self-owned
telemetry). Prod red-team 7/7 (cross-domain → 403, own-domain → 201/200).
**`feedback.rs::list_feedback` + `feedback_summary`** (`GET /feedback[/summary]?domain_id=…`) —
`_principal` ignored; the query binds the caller-supplied `q.domain_id` with **zero** authorization
(confused deputy: name any domain → read its full feedback). Gate on `q.domain_id` via `authz_domain`
and reject empty/missing `domain_id` (422). *(Not in the original enumeration; surfaced by the
2026-07-10 dual-review.)*

### Group F (lower) — `search.rs` `LIKE`-based readability filter → exact-match membership. — ✅ SHIPPED (#104)
Substring `LOWER(owners) LIKE '%subject%'` (leaked `alice` → owner `alicexyz@…`) replaced with
exact CSV membership via `crate::auth::authorized_for` (also honors node + `domain_scope`) OR
domain `visibility='public'`, filtered in Rust. Regression-proven: the substring-denied tests
fail against the old LIKE code, pass after.

### Group G — `ingestion.rs` no-gate reads *(surfaced by the 2026-07-10 dual-review; not in the original enumeration)*
`list_jobs` (`GET /ingest/jobs?mission_id=…`) ignores `_principal`; **with no `mission_id` it dumps
all tenants' jobs** (LIMIT 200), and with one it reads any mission's jobs. `get_job`
(`GET /ingest/jobs/{id}`) returns any job by id, no check. Job `config` can name source
systems/connectors. Require `mission_id`, gate via `authz_by_mission`; drop or admin-restrict the
no-filter branch; resolve `get_job`'s mission → `authz_by_mission` (or admin).

---

## Workstream 2 — Node domain-scoping — ✅ SHIPPED (#106; prod red-team 6/6)

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

**As-built (2026-07-14):** 15s-TTL cache, fail-closed on any DB error/lock-poison. `rust-reviewer`
(Opus) static pass found no lock/async/panic/fail-open defects in the resolver itself, but flagged
two invalidation gaps beyond the three the design named (`assign_node_agent`/`revoke_node_agent`/
`delete_node`): `enroll_agent` and `delete_agent` also mutate a node's `meshagent` footprint and
were not invalidating. Both closed in #106, keyed off **both** `node_id` and `runtime_node_id`
(a meshagent row's node association can live in either column). Live red-team (6/6) confirms the
shipped behavior: node reaches its operating domain, denied everywhere else.

**Follow-up fix (2026-07-14, #107): `assign_node_agent` missing domain authz.** An adversarial
codex review of the shipped #106 diff (independently verified against the code before treating it
as fact) found that `assign_node_agent` checked node ownership and that the target domain exists,
but never checked the *caller* was authorized for that domain before inserting the `meshagent` row
— since #106 made `meshagent` the source of truth for a node's domain scope, this let any node
owner seed their node into an arbitrary domain and grant it cross-domain read access, reopening
exactly what Workstream 2 closed. Root cause: no integration test exercised the node-scoping
behavior end-to-end (only pure-logic unit tests against a hand-built `Principal`), which is likely
how this slipped past both the original review and the red-team. Fixed via `authz_domain`,
mirroring the sibling `enroll_agent` handler; two new integration tests added
(`assign_node_agent_denied_for_domain_outsider`, `assign_node_agent_allowed_for_domain_owner`).
`rust-reviewer` (Opus): ship, no blocking findings. Live red-team 6/6 against `sha-ffdda81`
(deny-unauthorized, deny-read-after-deny, allow-owned, admin-bypass-intact, plus 2 self-cleaning
checks). Same adversarial pass also flagged (adjacent, not fixed here — folded into the
delegated-session item below) that `usersession.capability_scope` is write-only: `create_launch`
stores it but session resolution never reads it back, so a "scoped" launch token is actually a
full non-admin session for the subject, with a max TTL of 87,600 hours.

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

0. **[O1]/[O2] traces** — DONE (resolved 2026-07-10).
1. **Workstream 1** — ✅ COMPLETE. A (#97), B (#99/#100), E-feedback + G (#98), mission-brief (#96), D (#102), C + E-remainder (#103), F (#104) — all shipped + red-teamed.
2. **Workstream 2** — ✅ COMPLETE (#105 docs, #106 code). Node domain-scoping shipped, deployed (`sha-34386f6a`), prod red-teamed 6/6.
3. **Remaining:** the delegated-session design item (owner decision pending — see above). Optional consolidation (migrate mechanism-(2) files onto the shared `authorized_for_domain`) remains unscheduled.

Do not batch across groups — land each with its own tests and review so a regression is bisectable.
New authz helpers now available for the remaining groups: `authz_domain`, `authz_by_task/mission/agent`, `authz_by_control_plane_agent`, `domain_id_for_control_plane_agent`, `is_self_control_plane_agent` (all in `routes/authz.rs`). Test harness: `common::setup()` + `seed_*` helpers in `crates/edgeplane-tower/tests/common/mod.rs`.
