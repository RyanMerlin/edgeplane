# EdgePlane — Layered Tenancy Decision & Seam-Hardening Design

**Date:** 2026-06-17
**Status:** Decisions locked 2026-06-18 (see §9); P0 implementation plan to follow
**Project:** **edgeplane ONLY** (tower crate + edgeplaned crates + `edgeplane-homeassistant` plugin). **Zero aria dependency.**
**Supersedes framing of:** `Aria/Engineer/2026-06-17-edgeplane-oss-enterprise-hardening-engineer-handoff` (the 7-axis handoff)
**Reconciles:** `2026-06-13-edgeplane-platform-roadmap.md`, `2026-06-14-tower-dispatch-authz-hardening-design.md`, `2026-06-10-edgeplane-home-layout-and-wal-design.md`

---

## Separation invariant (non-negotiable)

Every change here lives in the **edgeplane** project. Nothing references, imports, calls, or requires aria-rs.
> **Test:** if aria-rs did not exist, does edgeplane work 100% as designed? **Answer: yes.**

---

## 1. The decision

The "OSS/enterprise hardening" effort was framed as a fork (platform roadmap R1d):

- **(a)** single-operator simplification — drop the multi-tenant ambitions; or
- **(b)** real multi-tenant collaboration — build governance, isolation, RBAC now.

**We choose neither. We choose (c): build the authorization/identity/isolation _seam_, default-off; defer the multi-tenant _machinery_ until a second real tenant exists.**

This is the Kubernetes model. A fresh cluster ships flat — you are cluster-admin, no RBAC ceremony, it just runs. But every object is namespaced and every action flows through a pluggable authorizer; you *bind* policy when a second principal appears. The seam is always present; enforcement is opt-in. EdgePlane should ship the same way: **zero ceremony for one operator, org-scale as a config change, not a rewrite.**

### Why this is the right call (grounded)

A four-thread industry review (2026-06-17, see Appendix B) converged on:

- **Multi-tenancy is not the differentiator.** The genuine white space is the *heterogeneous-node fleet* (register laptops/VMs/clusters/edge, dispatch agents to user-owned compute) — no OSS product ships that. Multi-tenancy itself is table-stakes for SaaS, an enterprise upsell for self-host, or a non-goal for the frameworks. Building it does not make EdgePlane stand out.
- **The mature lesson** (k8s namespaces/vCluster, Temporal namespaces, Nomad, GitHub runners): build the identity/permission *seam*; defer the isolation *machinery* (per-tenant storage, quotas, crypto cross-tenant guarantees) until tenants are *adversarial or compliance-distinct*. For an edge/industrial agent, the right model is a **scoped service identity**, not a tenant.
- **The near-term forcing function is agent identity + governance + distributed-fleet ops**, not multi-tenancy. Per-agent identity, scoped creds, and per-domain audit pay off *single-operator* and *are* the tenancy precondition.
- **Positioning:** multi-tenancy *without real tenants* reads as premature enterprise complexity. Depth (one hard problem solved well — real isolation, real agent identity) outsignals an RBAC/SSO checklist. The "individual→org pattern" narrative is open white space, won by a sharp architectural argument with a working proof.

---

## 2. Why now — the verified current posture (the live problem)

The four gaps below are **code-verified as of 2026-06-17** and **compose into a single problem**: any token → dispatches arbitrary work → to an agent holding full operator authority → running unsandboxed → leaving no audit trail. EdgePlane is single-operator *by accident* (missing seams), not zero-ceremony *by design*.

| # | Gap | Evidence (file:line) | Severity |
|---|-----|----------------------|----------|
| 1 | **Dispatch/ledger has no authorization.** No `authorized_for_domain` exists. `create_task`, `submit_mesh_task` insert claimable tasks against any domain for any authenticated principal; `domain_stream`/`mission_stream` don't even take a `Principal`. | `routes/work.rs:529,2331`; `routes/mcp.rs:209` | **Critical (live RCE-class)** |
| 2 | **No agent identity.** `EP_AGENT_TOKEN` injects the operator's own token into every agent; tower can't distinguish agent vs operator. `append_progress` hardcodes `agent_id=""`. | `agent_harness.rs:1064`; `routes/work.rs:1033` | High |
| 3 | **Agents run unsandboxed.** `edgeplaned-sandbox` fully built but wired into nothing; every runtime spawns the agent as the bare operator. | `runtimes/goose.rs:268`, `claude_code.rs:293`; `capability_dispatcher.rs:322 (TODO)` | High |
| 4 | **No real audit trail.** `ledgerevent` only records `workspace_commit`; task/agent/domain lifecycle emit nothing. Streams are unauthenticated. | `routes/mcp.rs:1149`; `routes/work.rs:2331` | Medium |

**Already resolved (do not redo):**
- `is_admin` is fully live via `EP_ADMIN_EMAILS` (~60 call sites; only OIDC sessions can be admin; SA tokens + node JWTs are always non-admin). `auth.rs:20,199`, `main.rs:72`.
- The dead-governance deletion (roadmap R1) is **done**: migration `0009_drop_governance.sql` dropped `governancepolicy`, `governancepolicyevent`, `approvalrequest`, `familymember`, `domainrolemembership`. Only residue remains (see §4 cleanup).

---

## 3. The principle: Domain is the namespace

Per `entities.md § Domain`, a Domain is "a policy surface; a permission boundary" carrying `owners`, `contributors`, `visibility`. **It is already the right seam.** Every seam below keys off Domain. Single-operator: you own all your domains → every check passes trivially → zero ceremony. Multi-tenant: bind other principals to domains → the same checks isolate them. No new boundary concept is introduced.

> **Correction to the 2026-06-14 authz spec:** its membership predicate referenced `domainrolemembership`, which migration 0009 dropped. The corrected predicate is:
> ```
> authorized_for_domain(principal, domain) :=
>     principal.is_admin
>     OR principal.subject ∈ domain.owners
>     OR principal.subject ∈ domain.contributors
> ```
> Default deny. One shared helper in `auth.rs`, reused at every privileged site.

---

## 4. The four seams

Each seam states: **what**, **current state**, **single-operator behavior** (must stay zero-ceremony), **multi-tenant behavior** (what switches on later), and **design notes**.

### Seam 1 — Authorization (P0; closes the live RCE hole)
- **What:** the `authorized_for_domain` helper above, applied before every privileged dispatch/ledger action; plus a **principal trust-tier split**: full-trust principals (human/admin sessions) may create free-form tasks in their domains; **service-account / dispatch tokens may only instantiate server-registered templates** with constrained params. Infra-grade templates create in a non-claimable `pending_approval` state.
- **Current state:** NOT-STARTED (designed in 2026-06-14 spec, corrected here for 0009).
- **Single-operator:** you are owner/admin of your domains → authorized for everything; you create tasks freely. No ceremony.
- **Multi-tenant:** other principals are scoped to their domains; the template allowlist means a compromised edge/dispatch token can only fire `{run-ceph-doctor}`, never `{reboot the cluster}`.
- **Design notes:** this is the 2026-06-14 spec, minus the dropped table. It is the highest-priority item because the hole is live. Service-account identity (Seam 2) makes the trust-tier split meaningful, so 1 and 2 ship together.

### Seam 2 — Agent identity (P0/P1; coupled to Seam 1)
- **What:** an agent acts under its **own** scoped identity, not the operator's token. On enrollment, mint a per-agent credential (per-agent service account or per-enrollment JWT) scoped to the agent's home domain(s). `claim_task`/`append_progress` record the *authenticated* agent_id.
- **Current state:** NOT-STARTED. Today `EP_AGENT_TOKEN` = operator token; agent==operator at the auth boundary.
- **Single-operator:** your agents are scoped to your domains — still "just works," but actions become attributable and constrainable (and Seam 1's SA-tier applies to them).
- **Multi-tenant:** an agent enrolled by tenant A can never act in tenant B's domains; agent compromise is bounded to its domain.
- **Design notes:** the SA path exists (`mcs_sa_*`) but is admin-provisioned and globally scoped — extend it to domain-scoped, or mint per-agent JWTs (node-JWT pattern already exists in `jwt.rs`). Pick one in the implementation plan.

### Seam 3 — Execution isolation (P1; highest positioning signal)
- **What:** wire `edgeplaned-sandbox` into the agent + capability spawn paths so agents run jailed, not as the bare operator.
- **Current state:** BUILT, UNWIRED. Jail implements userns/mount-ns/pivot_root/Landlock/seccomp/cap-drop/NO_NEW_PRIVS. **Incomplete:** cgroup limits and nftables egress are struct/env-only (no enforcement code).
- **Single-operator:** your agents run in a jail instead of as you — strictly safer, no behavior change for well-behaved agents.
- **Multi-tenant:** per-tenant resource limits + egress become enforceable (after the cgroup/nftables gaps are closed).
- **Design notes:** real subtlety — the jail expects the child to call `enter_jail()` on startup, but raw CLI agents (`claude`, `goose`, `kubectl`) never will. The plan must choose a wiring model: a thin **launcher/pre-exec wrapper** that enters the jail then `exec`s the agent, vs. a fork-in-parent approach. Integration points: `capability_dispatcher.rs:322` (explicit TODO) and the four runtime spawn sites. This is the "deeply-solved hard problem" that anchors the FDE/thought-leadership artifact — scope it deliberately.

### Seam 4 — Audit (P2)
- **What:** emit `ledgerevent` rows on task lifecycle (create/claim/complete/fail), agent enroll/delete, and domain mutations — not just workspace commits; authorize + domain-scope the `domain_stream`/`mission_stream` subscriptions (depends on Seam 1).
- **Current state:** NOT-STARTED for lifecycle events; ledger schema is domain/mission-keyed and ready.
- **Single-operator:** you get a real activity log of your fleet.
- **Multi-tenant:** it *is* the per-tenant audit trail; stream auth (Seam 1) keeps tenants from reading each other's activity.

---

## 5. Explicitly deferred (YAGNI until a second real tenant)

Do **not** build these now. Each is justified-deferred by the research (Appendix B):

- Per-tenant storage isolation / sharding.
- Resource quotas / noisy-neighbor controls (beyond finishing Seam 3's cgroup hooks for single-operator safety).
- SSO / SCIM / IdP-group→role mapping.
- Tenant admin hierarchies / org management UI.
- Cryptographic cross-tenant guarantees / per-tenant Infisical identities (today: one global machine identity; per-dispatch `SessionStore` scoping at the daemon already exists and is correct).
- Any rebuild of the GovernancePolicy/approvals engine (correctly deleted in 0009 — do not resurrect).

The architecture *allows* all of these later because Domain is the boundary and the seams exist. We add them when a second operator with adversarial or compliance-distinct needs actually arrives.

---

## 6. Reconciliation — handoff axes ↔ roadmap ↔ this plan

| Handoff axis | Roadmap item | Verified state | Disposition |
|---|---|---|---|
| 1. XDG / no hardcoded `$HOME` | (home-layout design) | Product source clean (hits are test fixtures); `edgeplaned-paths` honors `$EP_HOME`, not split-XDG | **Mostly done.** Optional small delta; "full XDG split" is debatable, not a goal |
| 2. System-install + service account | — | `crates/edgeplane/install.sh` already does `/etc/edgeplane` + systemd + env file | **Partial.** Remaining: `DynamicUser`/`ProtectSystem`/`/var/lib`. Low priority |
| 3. Decouple multiplexer | (retire-tmux-via-ACP plan) | ACP-first chosen; plan exists | Out of scope here; existing plan owns it |
| 4. Multi-tenancy + close authz gap | R1d, R5 | **Seams 1–4** | **This document.** Resolved as option (c) |
| 5. Pluggable secrets | — | Provider abstraction partial; per-dispatch scoping exists; one global Infisical identity | Deferred (§5) |
| 6. Config precedence | (home-layout) | Partial via paths crate + install.sh | Low priority; document the chain |
| 7. Packaging | R3 | `distribution/` + `infra/helm` exist (chart still named `missioncontrol`) | **Partial.** R3 init/doctor owns the gap; rename chart |
| — | R1 (delete dead scaffolding) | 0009 dropped 5 governance tables | **Done.** Residue cleanup only (below) |

**Residue cleanup (tiny):** drop the `approvalnonceuse` orphan table (a follow-up migration) and the always-NULL `reviewgate.approval_request_id` / `reviewgate.policy_rule_id` columns; remove the Slack `approval.request` stub string.

---

## 7. Sequencing (by risk)

1. **P0 — Seam 1 + Seam 2** (authorization + agent identity). The authz hole is live; the trust-tier split needs real agent identity. Ship together. *This is the security release.*
2. **P1 — Seam 3** (wire the sandbox). Highest positioning signal; moderate effort (resolve the `enter_jail` impedance first).
3. **P2 — Seam 4** (lifecycle audit + stream scoping).
4. **Cleanup** — residue migration (§6).

Each is its own PR with `cargo nextest` + `clippy -D warnings` green, generated artifacts regenerated, and the migration discipline from `profiles/engineer/CLAUDE.md`.

---

## 8. Positioning thesis (the narrative this earns)

> Agent orchestration should ship like `kubectl` on a fresh cluster — zero ceremony for one operator, with the authorization, identity, and isolation seams already in the architecture so org-scale is a config, not a fork.

This is true and *demonstrable* once Seams 1–3 land: a single binary that runs flat for one person, and the same binary, with policy bound, that isolates many. That is the ownable "individual→org" argument — backed by a working proof (real agent identity + a real jail), not a feature checklist.

---

## 9. Resolved decisions (2026-06-18, Merlin)

1. **Agent identity mechanism** (Seam 2): **per-agent JWT**, reusing the node-JWT pattern in `jwt.rs`, minted per-enrollment and scoped to the agent's home domain(s). (Rejected: extending the global `mcs_sa_*` service-account path.)
2. **Sandbox wiring model** (Seam 3): **launcher/pre-exec wrapper** — a thin wrapper enters the jail then `exec`s the agent, so raw CLI agents that never call `enter_jail()` are still jailed. **Finish cgroup limits now** (single-operator safety); **defer nftables egress** to multi-tenant. (Rejected: fork-in-parent.)
3. **Template registry** (Seam 1): **config file (TOML)** — declarative, version-controlled allowlist, no migration; restart-to-change is acceptable for one operator. (Rejected: DB table — revisit only if runtime mutation / per-domain scoping is needed.)
4. **Sequencing:** **land P0 (Seams 1 + 2) first** as a dedicated security release; freeze other feature work until it ships, because the dispatch/ledger authz hole is live.

---

## Appendix A — Verification provenance

Current-state claims verified by four parallel code-investigation passes on 2026-06-17 against `/home/merlin/code/edgeplane` @ `main`. File:line citations in §2/§4 are from those passes.

## Appendix B — Industry research provenance

Four research threads (landscape, control-plane economics, future trajectory, FDE/positioning), 2026-06-17, web-grounded. Key sources: vCluster multi-tenancy 2025/2026; Temporal namespace multi-tenancy docs; AWS Bedrock AgentCore (multi-tenant agent runtime); Kagent (CNCF Sandbox, K8s-only); Bain "agentic enterprise control plane" (Google Cloud Next 2026); CSA non-human-identity/agentic governance; Palantir/Anthropic/OpenAI FDE role analyses (MarkTechPost, The New Stack, Perspective AI). Full URL set retained in the session record.
