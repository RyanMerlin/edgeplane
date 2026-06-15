# R1d — Governance simplification: single-operator admin + strip the dead policy engine

- **Status:** design approved, pending implementation plan
- **Date:** 2026-06-15
- **Epic:** R1 deletion epic (final item). R1a (AI chat), R1b (evolve), R1c (skill-bundle/domain-pack cluster) already merged.
- **Scope:** `edgeplane-tower` (Rust), `edgeplane` CLI/TUI (Rust), `web` (TypeScript), one SQL migration.

## Goal

1. **Make the human operator a real admin.** Today `is_admin` is hardwired `false` in every auth path, so the operator cannot administer resources owned by his own agents/service-accounts, nor reach the pure-admin endpoints (ops backups, secrets bootstrap/rotate, service-account management).
2. **Delete the never-enforced policy/approvals/family governance scaffolding** — fork-inherited, dead, ~1,500 lines across surfaces.
3. **Remove the half-wired `domainrolemembership` mechanism** — a latent access-leak bug, not a usable RBAC seam.

## Non-goals

- Multi-tenant RBAC. The system is single-operator now and that is explicitly designed for. Multi-tenant is **low confidence** ("maybe someday"). It is preserved as a documented, additive upgrade path (see "Deferred seam"), **not built**.

## Verified ground truth

Every claim here was read from the code on branch `main` @ `f00d6344`, not inferred.

### Identity model

- A caller's identity is `Principal { subject, is_admin, session_id, auth_type }` (`edgeplane-tower/src/auth.rs:25`). `auth_type` ∈ `{session, service_account, node}`.
- `subject` for a human session is the **Authentik OIDC `sub` claim** — an opaque IdP identifier (`routes/oidc_web.rs:1037`). Login *also* persists `email` and `display_name`/preferred_username onto the `usersession` row (`routes/oidc_web.rs:220`; columns added in migrations `0003_usersession_email.sql`, `0004_usersession_display_name.sql`).
- The user-session token is **opaque** (`make_token`, random, hash-validated) — it carries no claims. Only *node* tokens are JWTs (the `matches('.').count() == 2` path). So per-request the resolver can read `usersession` columns but can never re-see OIDC claims.
- **Single resolver:** `require_auth` middleware (`auth.rs:266`) delegates to `Principal::from_request_parts` (`auth.rs:277`) and caches the result in request extensions. The per-handler `Principal` extractor reads that cache first. So there is exactly **one** place that constructs a Principal — modifying it covers both paths.
- The three Principal-construction branches all hardwire `is_admin: false`: node (`auth.rs:113`), service_account (`auth.rs:156`), user-session (`auth.rs:189`).

### Authorization model

- `is_admin` is the **global superuser override**, used at ~60 sites across ~17 route files. Dominant pattern: `owner == principal.subject || principal.is_admin`.
- **Owner-scoping already works without admin:** `create_domain` defaults a domain's `owners` to the creator's subject (`routes/domains.rs:136`). So the operator can already do everything *he owns*. The admin bit matters for (a) cross-owner administration and (b) pure-admin endpoints.
- **What is genuinely broken today** (pure-admin gates, no owner fallback):
  - `routes/ops.rs`: `/ops/backups` (trigger + list), `/ops/secrets/{status,bootstrap,rotate}` — all via `require_admin` (`ops.rs:153`).
  - `routes/auth.rs`: service-account create/list/revoke (`auth.rs:184/238/281`).
  - `routes/domains.rs`: `transfer_owner` (`domains.rs:343`).
  - All `routes/governance.rs` + `routes/family_governance.rs` gates (dead engines anyway).
- **Cross-owner gap:** agents/service-accounts create resources under their own subject (`sa:*`, `node:*`). With `is_admin=false` the human operator is not the owner and cannot administer them. This is the real fleet-oversight motivation.
- **Node enrollment is NOT broken** — it is join-token based; `rotate_node_token` allows node-self or owner (`runtime.rs:932`). Admin is only an override there.

### The dead policy/approvals/family engine

- `DEFAULT_POLICY` (`routes/governance.rs:21`) sets `require_approval_for_mutations:false` and `allow_*:true` — a pure no-op. No mutation path consults policy. It still carries now-stale `skills.bundle.publish`/`skills.bundle.deprecate` action keys left over from R1c.
- `governancepolicy` / `governancepolicyevent` referenced only in `routes/governance.rs`. `approvalrequest` referenced only in `routes/approvals.rs` (confirmed governance shape: `domain_id, action, channel, reason, target_entity_*, executed_action` — **not** ACP). `familymember` referenced only in `routes/family_governance.rs`. All self-contained within the tower crate.
- **No governance/approval/family MCP tools** exist — the single `policy` hit in `routes/mcp.rs` is `provision_domain_persistence` (unrelated persistence routing). The MCP catalogue stays at **23**; `tests/mcp_parity.rs` is unaffected.

### `domainrolemembership` is half-wired (a bug, not a seam)

- **Live** in `routes/search.rs` (3 sites: mission search + `get_readable_task_ids` + `get_readable_doc_ids`) — an `OR EXISTS(SELECT 1 FROM domainrolemembership …)` directly in the listing WHERE (`search.rs:173/221/260`); and in `routes/explorer.rs::can_read_domain` (`explorer.rs:93`), which matches `owners`/`contributors` with `if … { return true; }` and **falls through** to the membership query on no-match — so the role check is reachable for a real domain.
- **Dead fallback** in `routes/docs.rs` and `routes/artifacts.rs`: `can_read_domain`/`can_write_domain` `return in_list(owners) || in_list(contributors)` *unconditionally*, so the trailing `domainrolemembership` query is reached only when the domain row is *absent* — unreachable for any real domain (`docs.rs:45-95`, `artifacts.rs:66-123`).
- **Never consulted** by `domains.rs`'s own `can_read`/`can_write`/`can_own` (`domains.rs:39-56`) nor by `tasks.rs`.
- Net effect: granting a role via `/domains/{id}/roles` makes a domain's missions/docs **appear in the grantee's search but stay unopenable**. Inconsistent access — a latent leak.

## Design

### Change 1 — Admin fix `(a)+(i)`: config emails, evaluated at request time

- Add env var **`EP_ADMIN_EMAILS`** (comma-separated, case-insensitive). Parse once at startup (same pattern as `EP_WEB_DIR`/`EP_JWT_SIGNING_KEY` in `server.rs`) into a new `AppState.admin_emails: std::collections::HashSet<String>` (lowercased) — `state.rs:4`.
- In the **user-session branch only** of `Principal::from_request_parts` (`auth.rs` ~161-193): extend the query to `SELECT id, subject, email FROM usersession …` and set
  `is_admin = email.map(|e| state.admin_emails.contains(&e.to_lowercase())).unwrap_or(false)`.
  The `node` and `service_account` branches keep `is_admin: false` unchanged.
- No gate code changes — the single bit flows through all ~60 `principal.is_admin` sites and unblocks every broken pure-admin endpoint.
- **Rationale for email over `sub`:** `subject` is an opaque Authentik identifier; an admin list keyed on it is brittle and unreadable. `email` is already persisted on the row and human-meaningful.
- **Rationale for request-time over persisted column:** admin-list changes take effect on the next request (no re-login staleness), and it needs no migration. Group-claim-driven admin would *require* a persisted column (see Deferred seam) — but that is the deferred case.

### Change 2 — Strip the policy/approvals/family engine

Delete across all surfaces:

| Surface | Remove |
|---|---|
| tower routes | `routes/governance.rs`, `routes/approvals.rs`, `routes/family_governance.rs` |
| tower models | `models/governance.rs`; drop its re-exports in `models/mod.rs` |
| tower wiring | router registrations in `routes/mod.rs` / `server.rs`; the `DEFAULT_POLICY` const (stale `skills.bundle.*` keys vanish with it) |
| CLI | `crates/edgeplane/src/governance.rs` (policy/events/roles subcommands) + its command wiring (`commands.rs`, `lib.rs`) |
| TUI | `tui/screens/approval_queue.rs` (renders the governance `approvalrequest` queue) + wiring in `tui/app.rs`, `tui/data.rs`, `tui/work.rs`, `tui/screens/mod.rs` |
| web | `web/src/routes/governance.tsx` (+ `governance.test.tsx`); nav/breadcrumb/queryKey entries; regenerate `routeTree.gen.ts` and `openapi.json` |

**Explicitly KEEP — do not touch:** `web/src/components/conversation/ApprovalPrompt.tsx` and the conversation approval flow. That is **ACP tool-call approval** (live agent tool-use confirmation), a different mechanism that shares only the overloaded word "approval".

### Change 3 — Drop `domainrolemembership`

- Remove `/domains/{domain_id}/roles` GET/POST + `/domains/{domain_id}/roles/{subject}` DELETE handlers, `row_to_role`, and the `DomainRoleMembership` / `DomainRoleUpsert` models (`models/domain.rs`, re-export in `models/mod.rs`). (The CLI `run_roles` is removed with `governance.rs` above.)
- Remove the **live** `domainrolemembership` access paths — they would error once the table is dropped, and removing them is the intended behavior change (role-based access goes away with the mechanism): the 3 `OR EXISTS` clauses in `routes/search.rs`, and the trailing membership query in `routes/explorer.rs::can_read_domain` (delete the query so the function ends returning `false` after the owners/contributors check).
- Remove the **dead-fallback** `domainrolemembership` queries in `routes/docs.rs` and `routes/artifacts.rs` — truly unreachable for real domains; delete and have the function end returning `false`.
- **Behavior change is bounded:** only a *pure role-holder* (a subject granted via `/roles` but absent from `owners`/`contributors`) loses access. Owners, contributors, and admins are unaffected. Dropping `/roles` means no such grants can be created going forward, and the migration discards any existing ones.
- `owners` / `contributors` string columns on `domain` remain the single, consistent domain-authz mechanism.

### Migration

- New `crates/edgeplane-tower/migrations/0009_drop_governance.sql` (sqlx, filename-ordered after R1c's `0008`).
- `DROP TABLE IF EXISTS` (idempotent, fresh-vs-existing safe) for: `governancepolicy`, `governancepolicyevent`, `approvalrequest`, `familymember`, `domainrolemembership` (5 tables).
- **Destructive and effectively irreversible** — a down-migration would recreate empty tables, not restore rows. Acceptable: all five are dead, dormant, or half-broken.
- Code removal and migration ship in the **same PR** — a table cannot be dropped while code still queries it.

### Deferred seam `(c)+(ii)` — documented, not built

When/if multi-tenant + IdP-driven admin becomes real, the upgrade is purely additive and rewrites **zero** gates (because every gate just reads `principal.is_admin`):

1. Forward migration adds `is_admin BOOLEAN NOT NULL DEFAULT false` to `usersession`.
2. `oidc_web.rs` populates it at login from the Authentik `groups` claim (e.g. membership in `edgeplane-admin`).
3. The Principal resolver reads the column instead of (or in addition to) the `EP_ADMIN_EMAILS` config check, which remains as a break-glass bootstrap.

This is required (not optional) for group-claim admin because the opaque session token carries no claims and the resolver runs per-request — group membership is visible only at login and must be persisted. `capability_scope` is **not** available for reuse: `remotectl.rs` owns it as the remote-exec capability grant.

## Testing & verification

- `cargo check` + `cargo clippy -D warnings` across `edgeplane-tower` and `edgeplane` (catches dangling references from the deletions).
- `cargo nextest run -p edgeplane-tower -p edgeplane --no-fail-fast`.
- Confirm `tests/mcp_parity.rs` stays green (catalogue still **23** — no MCP tools removed).
- New auth tests:
  - user-session whose `email` ∈ `EP_ADMIN_EMAILS` → `is_admin == true`;
  - user-session with non-listed email and with NULL email → `false`;
  - service_account and node tokens → `false` regardless of `EP_ADMIN_EMAILS`;
  - one cross-owner override test: an admin can mutate a resource whose owner subject differs.
- Web: build green after `routeTree.gen.ts` regen; no dangling imports to the removed governance route.

## Risks & deploy notes

- **Overloaded "approval"** — mitigated: governance `approvalrequest` (tower `approvals.rs`, TUI `approval_queue.rs`) is removed; ACP `ApprovalPrompt.tsx` is kept. Both verified by struct shape and module location.
- **`routeTree.gen.ts` ordering** — removing a route requires regen; `tsc -b` runs first, so transient build breakage until the route file is gone and the tree is regenerated (known gotcha).
- **Deploy gating** — admin is off until `EP_ADMIN_EMAILS` is set. The deploy/runbook must set it to the exact email Authentik emits for the operator (verify the live `email` claim value; do not assume). If unset, behavior is identical to today (no admin) — fail-safe, not fail-open.
- **Data loss on migrate** — dropping the 5 tables discards any rows. Confirmed acceptable given they are dead/dormant/half-broken.

## Out of scope

- Any change to the ACP tool-approval flow.
- Building group-claim admin, a `usersession.is_admin` column, or any multi-tenant role model (deferred seam only).
- Changes to `owner_subject`-based ownership on runtime/budgets/persistence/work (already functional; admin override now simply works for them).
