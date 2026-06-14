# EdgePlane Tower — Dispatch & Ledger Authorization Hardening

**Date:** 2026-06-14
**Status:** Draft — pending review
**Project:** **edgeplane ONLY** (tower crate + `edgeplane-homeassistant` plugin). **Zero aria dependency.**

---

## Separation invariant (non-negotiable)

Every change in this spec lives in the **edgeplane** project — the `edgeplane-tower` crate and the
`edgeplane-homeassistant` plugin repo. **Nothing here references, imports, calls, or requires aria-rs.**

> **The test:** if aria-rs did not exist, does edgeplane work 100% as designed?
> **Answer: yes.** No aria binary, CLI, endpoint, config, or data is touched. Postgres, Prometheus,
> and Home Assistant are *shared infrastructure*, not aria. See the Separation Audit at the end.

This spec is the spun-out **security-hardening track** (option #2) extracted from the parked
`aria/docs/superpowers/specs/2026-06-14-ha-edgeplane-ops-view-design.md`. It stands alone: it is worth
doing regardless of whether any HA ops view is ever built, because the vulnerability is **live today**.

---

## Why — the live vulnerability (red-team, 2026-06-14, code-verified)

Adversarial review found a real, current authorization hole in the tower — independent of any HA work:

- **Task creation has no authorization.** `create_task` (`routes/work.rs:529`) and `submit_mesh_task`
  (`routes/mcp.rs:211`) insert a `status='ready'` (immediately claimable) task against **any**
  mission/domain for **any** authenticated principal. `created_by_subject` is recorded but never
  *authorized*. No domain-membership check, no capability constraint, no template constraint.
- **Ledger reads are not domain-scoped.** `domain_stream` / `mission_stream` (`routes/work.rs:2334`)
  authenticate the upgrade but never check that the principal belongs to the domain — and the handler
  doesn't even take a `Principal`. Any valid token streams **any** domain's full activity ledger.
- **Plugin capability gate is bypassable.** `coordinator.py:253` — a task with **empty**
  `required_capabilities` skips the capability filter and executes.
- **Approval gate is at the wrong layer.** The only human gate (`models.py:38`) fires solely for
  `notify`-with-actions payloads; an infra service-call task executes with no confirmation.

**Impact:** any holder of any valid tower token (a service-account token, a node JWT, a future
dispatch surface) can dispatch arbitrary work to capability-matched agents — including infra agents
that run k8s/Ceph/deploy actions. That is RCE-equivalent against the cluster. Fixing this is the
prerequisite for ever exposing *any* dispatch surface.

**Note — the existing pattern already exists, just not here.** The tower already enforces
`if owner != principal.subject && !principal.is_admin { 403 }` for reviewgates (`work.rs:1514`) and
runtime nodes (`work.rs:1696`). This spec applies that same discipline to dispatch and ledger reads.

---

## Scope / non-goals

**In:** domain-scoped authorization on task creation + ledger streams; a server-side dispatchable-template
allowlist; an infra-grade confirm-at-creation gate; the plugin capability fail-closed fix; least-privilege
posture for service-account tokens.

**Out (parked, not this spec):** the HA ops view, LedgerConsumer, EntityMaterializer, dashboards,
tower health domain, HGA bridge. **Any aria change** — none is required or permitted here.

---

## Membership model (grounded in schema)

Domain authorization uses what already exists:
- `domain.owners` (text, `ck_domain_owners_nonempty`) — owning subject(s).
- `domainrolemembership` table — explicit `(domain_id, subject, role)` membership.
- `Principal { subject, is_admin, … }` (`auth.rs:25`).

**Predicate:**
```
authorized_for_domain(principal, domain_id) :=
    principal.is_admin
    OR principal.subject ∈ domain.owners
    OR EXISTS(domainrolemembership WHERE domain_id = ? AND subject = principal.subject)
```
Default deny. This is a new shared helper in the tower (`auth.rs`), reused by every site below.

---

## Design

### 1. Domain-scoped authorization (tower)
Apply `authorized_for_domain` before the privileged action at:
- **`create_task`** (`work.rs`) — resolve `domain_id` via the mission; 403 if unauthorized.
- **`submit_mesh_task`** (`mcp.rs`) — same; also verify the mission exists (today it doesn't check).
- **`domain_stream` / `mission_stream`** (`work.rs`) — thread `Principal` into the handler; authorize
  **before** the WS upgrade; 403 otherwise. (For missions, resolve the owning domain.)

### 2. Dispatchable-template allowlist (tower)
Free-form task creation (`title` / `description` / `required_capabilities` chosen by the caller) is
restricted by **principal trust tier**:
- **Full-trust principals** (human operator sessions, admins) may create free-form tasks in their domains.
- **Service-account / dispatch tokens** (e.g. the HA plugin token, any future dispatch surface) may
  **only** instantiate a **named, server-registered template** with typed/constrained params — never
  free-form. Each template declares fixed `required_capabilities`, an allowed-params schema, and a
  trust tag (`auto` | `infra-grade`).
- Registry lives tower-side (config + table). Examples: `run-ceph-doctor` (no params),
  `argocd-sync` (`app` ∈ allowlist).

This is the gate that makes *any* dispatch surface safe: a compromised plugin token can only fire
constrained, pre-approved templates — not `{reboot the cluster}`.

### 3. Infra-grade confirm-at-creation (tower)
Templates tagged `infra-grade` create the task in **`pending_approval`**, not `ready` — it is not
claimable until an explicit approval action flips it. The gate is enforced **tower-side**, not in the
plugin's notify heuristic. (A mobile/console approver UX can come later; the *enforcement* is here.)

### 4. Plugin capability fail-closed (`edgeplane-homeassistant`)
`coordinator.py` — reject a task with empty/unmatched `required_capabilities` (currently it executes).
Capability match becomes mandatory. (Plugin repo = edgeplane org; still zero aria.)

### 5. Least-privilege service-account posture (tower + plugin)
The HA plugin SA identity is scoped to its own domain + the template allowlist — not a general principal.
Document the token's authz profile; it should be unable to free-form-create or read other domains' ledgers.

---

## Trust tiers

| Principal | Free-form `create_task` | Template dispatch | Ledger read |
|-----------|------------------------|-------------------|-------------|
| admin / human operator session | yes (own domains) | n/a | own domains |
| service-account / dispatch token (HA plugin) | **no** | allowlisted templates only; `infra-grade` → `pending_approval` | **own domain only** |
| node JWT | per node role, own domain | per role | own domain |

---

## Verification (cargo nextest + caller-phase)

- **Unit:** `authorized_for_domain` allow/deny matrix (owner / member / admin / stranger × domain);
  template allowlist enforcement (free-form by SA token → reject); `infra-grade` → `pending_approval`.
- **Route:** ledger stream 403 for non-member; `create_task`/`submit_mesh_task` 403 cross-domain.
- **Plugin:** capability fail-closed (empty `required_capabilities` → rejected, with a unit test).
- **Caller-phase exercise:** with a real SA token — free-form `create_task` → 403; off-allowlist
  template → 403; cross-domain `domain_stream` → 403; an `infra-grade` template lands `pending_approval`.

---

## Phasing

- **P1 — stop the bleeding:** `authorized_for_domain` helper + enforce on `create_task`,
  `submit_mesh_task`, `domain_stream`, `mission_stream`. (This alone closes the RCE path.)
- **P2 — constrain dispatch:** template registry + allowlist for SA/dispatch tokens + `infra-grade`
  confirm-at-creation.
- **P3 — harden the edges:** plugin capability fail-closed + SA token least-privilege scoping.

P1 is independently shippable and is the highest-priority fix.

---

## Separation Audit (the test Merlin demanded)

| Change | Repo | aria touched? |
|--------|------|---------------|
| `authorized_for_domain` + enforcement | `edgeplane` (tower) | no |
| Template allowlist + registry | `edgeplane` (tower) | no |
| `infra-grade` confirm-at-creation | `edgeplane` (tower) | no |
| Capability fail-closed | `edgeplane-homeassistant` (plugin) | no |
| SA token least-privilege | `edgeplane` (tower) | no |

**Verdict: 100% separation.** Delete aria-rs from existence and every line of this spec still compiles,
deploys, and behaves as designed. Shared infra (Postgres / Prometheus / HA) is not aria. **Confidence: high.**
