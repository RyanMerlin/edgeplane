# R1d Governance Simplification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the human operator a real admin via `EP_ADMIN_EMAILS`, and delete the never-enforced policy/approvals/family governance scaffolding plus the half-wired `domainrolemembership` mechanism, across tower + CLI + TUI + web + one SQL migration.

**Architecture:** A single env-configured admin-email set flows `EP_ADMIN_EMAILS` → `AppConfig` → `AppState` → the user-session branch of the Principal resolver, where a pure `is_admin_email()` function sets `is_admin`. No authorization gate changes — the one bit flows through ~60 existing `principal.is_admin` sites. Everything else is deletion: three dead route engines, their models, the `/roles` endpoints, the role-membership SQL, and one migration that drops five tables. Code removal and the migration ship in the **same PR** (a table cannot be dropped while code still queries it).

**Tech Stack:** Rust (axum, sqlx runtime queries — *not* the `query!` macro, no `.sqlx` cache), TypeScript (TanStack Router + Vite, npm), Postgres, utoipa (OpenAPI).

**Spec:** `docs/superpowers/specs/2026-06-15-r1d-governance-design.md`. Branch: `feat/r1d-governance` (spec already committed at `a5625821`).

---

## Invariants you MUST hold (read before starting)

1. **`cargo check` cannot see the deletions' SQL.** Every governance/role table reference is a runtime `sqlx::query("…")` string literal — there is **no** `query!` macro and no `.sqlx` offline cache. The compiler will catch dangling *Rust symbol* references (models, handlers, route registrations, `openapi.rs` paths) but will **not** catch a leftover SQL string. The real guard is the textual sweep in **Task 9**, which must return zero hits before the migration is allowed to land. Do not skip it.

2. **No real-DB test harness exists.** All tower tests use `PgPool::connect_lazy("postgres://localhost/test")`, which never connects; CI has no Postgres for unit tests. Therefore the admin logic is tested as a **pure function** (`is_admin_email`), not by hitting `usersession`. The "cross-owner override" behavior cannot be an automated test in this crate — it is covered by the deploy-verification step in Task 9, stated honestly, not faked.

3. **"Approval" is overloaded — keep the ACP one.** Delete the governance `approvalrequest` queue (tower `routes/approvals.rs` + `models/approval.rs`, TUI `approval_queue.rs`, web — none here). **Never touch** `web/src/components/conversation/ApprovalPrompt.tsx` or the ACP tool-call approval flow. Different mechanism, same word.

4. **Each task ends green.** Run the task's verification command and confirm it passes before committing. The migration (Task 8) lands only after all code that queries the five tables is gone (Tasks 2–7).

5. **Single resolver.** `auth.rs::Principal::from_request_parts` is the only Principal construction point (the `require_auth` middleware caches into request extensions; the extractor reads that cache). Editing the user-session branch covers every authenticated route.

---

## File inventory

**Modify (surgical):**
- `crates/edgeplane-tower/src/auth.rs` — add `is_admin_email` + unit tests; wire user-session branch.
- `crates/edgeplane-tower/src/state.rs` — add `admin_emails` field.
- `crates/edgeplane-tower/src/server.rs` — add `AppConfig.admin_emails`; thread into `AppState`.
- `crates/edgeplane-tower/src/main.rs` — parse `EP_ADMIN_EMAILS` into `AppConfig`.
- `crates/edgeplane-tower/tests/test_proxy.rs` — fix 3 `AppConfig { … }` literals.
- `crates/edgeplane-tower/src/routes/search.rs` — drop 3 `OR EXISTS` clauses (site 1 renumbers a bind).
- `crates/edgeplane-tower/src/routes/docs.rs` — drop trailing membership query in 2 fns.
- `crates/edgeplane-tower/src/routes/artifacts.rs` — drop trailing membership query in 2 fns.
- `crates/edgeplane-tower/src/routes/explorer.rs` — drop trailing membership query in `can_read_domain`.
- `crates/edgeplane-tower/src/routes/domains.rs` — drop `/roles` routes + handlers + `row_to_role` + cleanup DELETE + role-model imports.
- `crates/edgeplane-tower/src/models/domain.rs` — drop `DomainRoleMembership`, `DomainRoleUpsert`.
- `crates/edgeplane-tower/src/models/mod.rs` — drop `pub mod approval/governance`, `pub use` re-exports.
- `crates/edgeplane-tower/src/routes/mod.rs` — drop 3 `pub mod` + 3 `.merge()`.
- `crates/edgeplane-tower/src/openapi.rs` — drop governance path stubs, schemas, tag.
- `crates/edgeplane/src/commands.rs`, `lib.rs`, `compat.rs` — drop CLI governance wiring.
- `crates/edgeplane/src/tui/{app.rs,data.rs,work.rs,screens/mod.rs}` — drop approval-queue wiring.
- `web/src/components/shell/{navModel.ts,navModel.test.ts,breadcrumbs.ts,Sidebar.tsx}`, `web/src/lib/queryKeys.ts` — drop governance nav.

**Delete (whole files):**
- `crates/edgeplane-tower/src/routes/{governance.rs,approvals.rs,family_governance.rs}`
- `crates/edgeplane-tower/src/models/{governance.rs,approval.rs}`
- `crates/edgeplane/src/governance.rs`
- `crates/edgeplane/src/tui/screens/approval_queue.rs`
- `web/src/routes/{governance.tsx,governance.test.tsx}`

**Create:**
- `crates/edgeplane-tower/migrations/0009_drop_governance.sql`

**Regenerate (commit the output):**
- `web/openapi.json`, `web/src/api/schema.gen.ts`, `web/src/routeTree.gen.ts`, `docs/reference/COMMAND-MAP.md`.

---

## Task 1: Admin via `EP_ADMIN_EMAILS` (pure-function TDD)

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs` (add fn near top after imports; user-session branch at `auth.rs:161-193`; tests appended to file)
- Modify: `crates/edgeplane-tower/src/state.rs:4-13`
- Modify: `crates/edgeplane-tower/src/server.rs:15-21,28-40`
- Modify: `crates/edgeplane-tower/src/main.rs:68-72`
- Modify: `crates/edgeplane-tower/tests/test_proxy.rs:19,33,49`

- [ ] **Step 1: Write the failing unit tests for the pure admin function**

Append to `crates/edgeplane-tower/src/auth.rs` (after the existing `#[cfg(test)] mod public_path_tests` block):

```rust
#[cfg(test)]
mod admin_email_tests {
    use super::is_admin_email;
    use std::collections::HashSet;

    fn admins() -> HashSet<String> {
        ["admin@example.com".to_string()].into_iter().collect()
    }

    #[test]
    fn listed_email_is_admin() {
        assert!(is_admin_email(Some("admin@example.com"), &admins()));
    }

    #[test]
    fn listed_email_is_case_insensitive() {
        assert!(is_admin_email(Some("Admin@Example.COM"), &admins()));
    }

    #[test]
    fn unlisted_email_is_not_admin() {
        assert!(!is_admin_email(Some("someone@example.com"), &admins()));
    }

    #[test]
    fn null_email_is_not_admin() {
        assert!(!is_admin_email(None, &admins()));
    }

    #[test]
    fn empty_admin_set_is_never_admin() {
        let empty: HashSet<String> = HashSet::new();
        assert!(!is_admin_email(Some("admin@example.com"), &empty));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail (function not defined)**

Run: `cargo test -p edgeplane-tower --lib admin_email_tests 2>&1 | tail -20`
Expected: FAIL — `cannot find function is_admin_email in this scope` (compile error).

- [ ] **Step 3: Add the pure function and its import to `auth.rs`**

Add to the imports block at the top of `crates/edgeplane-tower/src/auth.rs` (after `use std::sync::Arc;` at line 11):

```rust
use std::collections::HashSet;
```

Add the function immediately after the imports / before `pub struct Principal` (around line 14):

```rust
/// Pure admin-policy check: `true` when `email` (case-insensitive) is present
/// in the configured admin set. No DB or IO, so it is directly unit-testable.
/// Only the user-session auth branch calls this; node and service-account
/// principals are never admin, by construction.
pub(crate) fn is_admin_email(email: Option<&str>, admin_emails: &HashSet<String>) -> bool {
    email
        .map(|e| admin_emails.contains(&e.to_lowercase()))
        .unwrap_or(false)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p edgeplane-tower --lib admin_email_tests 2>&1 | tail -20`
Expected: PASS — 5 passed.

- [ ] **Step 5: Add `admin_emails` to `AppState`**

In `crates/edgeplane-tower/src/state.rs`, add the import at the top (after line 2 `use sqlx::PgPool;`):

```rust
use std::collections::HashSet;
```

Change the `AppState` struct (lines 4-13) to add the field:

```rust
pub struct AppState {
    pub db: PgPool,
    pub node: NodeInfo,
    /// Optional upstream URL — unknown routes are forwarded here (proxy mode).
    pub api_proxy: Option<String>,
    /// RS256 private key for signing node JWTs.
    pub jwt_encoding_key: EncodingKey,
    /// RS256 public key for verifying node JWTs.
    pub jwt_decoding_key: DecodingKey,
    /// Lowercased operator emails whose user-session principals resolve to
    /// `is_admin = true`. Populated from `EP_ADMIN_EMAILS` at startup.
    pub admin_emails: HashSet<String>,
}
```

- [ ] **Step 6: Add `admin_emails` to `AppConfig` and thread it into `AppState`**

In `crates/edgeplane-tower/src/server.rs`, change `AppConfig` (lines 15-21):

```rust
#[derive(Default, Clone)]
pub struct AppConfig {
    pub node_id: u64,
    pub advertise_url: Option<String>,
    /// When set, routes not matched by this app are proxied to this base URL.
    pub api_proxy: Option<String>,
    /// Lowercased admin emails, parsed from `EP_ADMIN_EMAILS` at the entrypoint.
    pub admin_emails: std::collections::HashSet<String>,
}
```

In the same file, in `build_app` where `AppState` is constructed (lines 28-40), add the field after `jwt_decoding_key`:

```rust
    let state = Arc::new(AppState {
        db,
        node: NodeInfo {
            node_id: config.node_id,
            advertise_url: config.advertise_url.clone(),
            role: "standalone",
            term: 0,
            leader_id: None,
        },
        api_proxy: config.api_proxy.clone(),
        jwt_encoding_key,
        jwt_decoding_key,
        admin_emails: config.admin_emails.clone(),
    });
```

- [ ] **Step 7: Parse `EP_ADMIN_EMAILS` at the entrypoint**

In `crates/edgeplane-tower/src/main.rs`, change the `AppConfig` construction (lines 68-72):

```rust
    let config = AppConfig {
        node_id: cli.node_id.unwrap_or(1),
        advertise_url: cli.advertise_url.clone(),
        api_proxy: cli.api_proxy.clone(),
        admin_emails: std::env::var("EP_ADMIN_EMAILS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect(),
    };
```

- [ ] **Step 8: Wire the user-session branch to set `is_admin`**

In `crates/edgeplane-tower/src/auth.rs`, the `mcs_` user-session branch (lines 161-193). Change the `SELECT` to include `email` and compute `is_admin`:

```rust
            } else if token.starts_with("mcs_") {
                // User session token — validate against usersession
                let row = sqlx::query(
                    "SELECT id, subject, email FROM usersession \
                     WHERE token_hash = $1 AND revoked = false AND expires_at > $2"
                )
                .bind(&hash)
                .bind(now)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten();

                if let Some(row) = row {
                    let subject: String = row.get("subject");
                    let email: Option<String> = row.get("email");
                    let session_id: i32 = row.get("id");
                    let db = state.db.clone();
                    let h = hash.clone();
                    tokio::spawn(async move {
                        let _ = sqlx::query(
                            "UPDATE usersession SET last_used_at = NOW() WHERE token_hash = $1"
                        )
                        .bind(&h)
                        .execute(&db)
                        .await;
                    });
                    return Ok(Principal {
                        subject,
                        is_admin: is_admin_email(email.as_deref(), &state.admin_emails),
                        session_id: Some(session_id),
                        auth_type: "session".into(),
                    });
                }
            }
```

Leave the `node` branch (`auth.rs:113`) and `service_account` branch (`auth.rs:156`) with `is_admin: false` unchanged.

- [ ] **Step 9: Fix the `AppConfig` literals in `test_proxy.rs`**

Adding a non-defaulted field breaks the 3 explicit `AppConfig { … }` literals at `crates/edgeplane-tower/tests/test_proxy.rs:19,33,49`. Add `admin_emails: Default::default(),` to each literal (or append `..Default::default()` if the literal does not already set every field). Run `cargo check -p edgeplane-tower --tests 2>&1 | grep -A3 "missing field"` first to confirm which literals error, then fix them.

- [ ] **Step 10: Verify the whole crate compiles and tests pass**

Run: `cargo check -p edgeplane-tower --all-targets && cargo test -p edgeplane-tower --lib admin_email_tests 2>&1 | tail -5`
Expected: check succeeds; 5 admin_email tests pass.

- [ ] **Step 11: Commit**

```bash
git add crates/edgeplane-tower/src/auth.rs crates/edgeplane-tower/src/state.rs \
        crates/edgeplane-tower/src/server.rs crates/edgeplane-tower/src/main.rs \
        crates/edgeplane-tower/tests/test_proxy.rs
git commit -m "feat(r1d): operator admin via EP_ADMIN_EMAILS"
```

---

## Task 2: Remove the `domainrolemembership` access paths

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/search.rs:166-182, 214-226, 253-265`
- Modify: `crates/edgeplane-tower/src/routes/docs.rs:45-75, 77-...` (two fns)
- Modify: `crates/edgeplane-tower/src/routes/artifacts.rs:66-96, 98-123` (two fns)
- Modify: `crates/edgeplane-tower/src/routes/explorer.rs:78-101`

There are no automated tests for these (no DB harness); the behavior change — pure role-holders lose search/read access; owners/contributors/admins unaffected — is intended and verified by Task 9's grep + deploy check. Each step is verified by `cargo check`.

- [ ] **Step 1: search.rs site 1 — mission search (renumber the trailing bind)**

In `crates/edgeplane-tower/src/routes/search.rs`, replace the query+binds at lines 166-182. **Note the `$4` → `$3` renumber** on `LIMIT` after the `$3` EXISTS clause is removed:

Before:
```rust
        sqlx::query(
            "SELECT k.* FROM mission k \
             LEFT JOIN domain m ON m.id = k.domain_id \
             WHERE (LOWER(k.name) LIKE $1 OR LOWER(COALESCE(k.tags,'')) LIKE $1) \
               AND (m.visibility='public' \
                    OR LOWER(m.owners) LIKE $2 \
                    OR LOWER(m.contributors) LIKE $2 \
                    OR EXISTS(SELECT 1 FROM domainrolemembership mrm WHERE mrm.domain_id=m.id AND mrm.subject=$3)) \
             ORDER BY k.updated_at DESC LIMIT $4",
        )
        .bind(&pattern)
        .bind(format!("%{}%", principal.subject.to_lowercase()))
        .bind(&principal.subject)
        .bind(limit)
```

After:
```rust
        sqlx::query(
            "SELECT k.* FROM mission k \
             LEFT JOIN domain m ON m.id = k.domain_id \
             WHERE (LOWER(k.name) LIKE $1 OR LOWER(COALESCE(k.tags,'')) LIKE $1) \
               AND (m.visibility='public' \
                    OR LOWER(m.owners) LIKE $2 \
                    OR LOWER(m.contributors) LIKE $2) \
             ORDER BY k.updated_at DESC LIMIT $3",
        )
        .bind(&pattern)
        .bind(format!("%{}%", principal.subject.to_lowercase()))
        .bind(limit)
```

- [ ] **Step 2: search.rs site 2 — `get_readable_task_ids` (no renumber)**

Replace the query+binds at lines 214-226:

Before:
```rust
    let readable_missions: Vec<String> = sqlx::query(
        "SELECT k.id FROM mission k \
         LEFT JOIN domain m ON m.id = k.domain_id \
         WHERE k.id = ANY($1) \
           AND (m.visibility='public' \
                OR LOWER(m.owners) LIKE $2 \
                OR LOWER(m.contributors) LIKE $2 \
                OR EXISTS(SELECT 1 FROM domainrolemembership mrm WHERE mrm.domain_id=m.id AND mrm.subject=$3))",
    )
    .bind(&mission_ids)
    .bind(&like_pat)
    .bind(subject)
```

After:
```rust
    let readable_missions: Vec<String> = sqlx::query(
        "SELECT k.id FROM mission k \
         LEFT JOIN domain m ON m.id = k.domain_id \
         WHERE k.id = ANY($1) \
           AND (m.visibility='public' \
                OR LOWER(m.owners) LIKE $2 \
                OR LOWER(m.contributors) LIKE $2)",
    )
    .bind(&mission_ids)
    .bind(&like_pat)
```

(`subject` is still used to build `subject_lower`/`like_pat`, so the parameter stays — only the `.bind(subject)` line is removed.)

- [ ] **Step 3: search.rs site 3 — `get_readable_doc_ids` (identical to site 2)**

Apply the exact same before→after as Step 2 to the query+binds at lines 253-265 (same SQL text and same `.bind(subject)` removal).

- [ ] **Step 4: docs.rs — drop the trailing membership query in both fns**

In `crates/edgeplane-tower/src/routes/docs.rs`, `can_read_domain` (lines 45-75): the owners/contributors branch already does an unconditional `return in_list(&owners) || in_list(&contributors);` (line 65). Delete the trailing query (lines 67-74) and end the function with `false`:

```rust
async fn can_read_domain(db: &sqlx::PgPool, principal: &Principal, domain_id: &str) -> bool {
    if principal.is_admin {
        return true;
    }
    if let Ok(Some(row)) = sqlx::query(
        "SELECT visibility, owners, contributors FROM domain WHERE id=$1",
    )
    .bind(domain_id)
    .fetch_optional(db)
    .await
    {
        let visibility: String = row.get("visibility");
        if visibility.to_lowercase() == "public" {
            return true;
        }
        let owners: String = row.get("owners");
        let contributors: String = row.get("contributors");
        let sub = principal.subject.to_lowercase();
        let in_list =
            |s: &str| s.split(',').map(|x| x.trim().to_lowercase()).any(|x| x == sub);
        return in_list(&owners) || in_list(&contributors);
    }
    false
}
```

Apply the same transformation to `can_write_domain` (lines 77 onward): delete its trailing `sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM domainrolemembership … role IN ('owner','contributor'))") … .unwrap_or(false)` block and end the function body with `false`.

- [ ] **Step 5: artifacts.rs — drop the trailing membership query in both fns**

In `crates/edgeplane-tower/src/routes/artifacts.rs`, `can_read_domain` (lines 66-96) and `can_write_domain` (lines 98-123) have the identical structure to docs.rs. For each, delete the trailing `sqlx::query_scalar::<_, bool>(…domainrolemembership…) … .unwrap_or(false)` block (read: lines 88-95 and 115-122) and end each function body with `false`.

- [ ] **Step 6: explorer.rs — drop the live membership query**

In `crates/edgeplane-tower/src/routes/explorer.rs`, `can_read_domain` (lines 78-101). Unlike docs/artifacts, the owners/contributors check here is `if in_list(...) { return true; }` (it falls through on no-match). Delete the trailing query (lines 93-100) and end the function with `false`:

```rust
        if in_list(&owners) || in_list(&contributors) {
            return true;
        }
    }
    false
}
```

(That is: keep everything through the closing `}` of the `if let Some(row)` block at line 92, then `false` as the final expression.)

- [ ] **Step 7: Verify compile (catches accidental unused bindings / params)**

Run: `cargo check -p edgeplane-tower 2>&1 | tail -10`
Expected: success, no warnings about unused `principal`/`subject`/`db`.

- [ ] **Step 8: Commit**

```bash
git add crates/edgeplane-tower/src/routes/search.rs \
        crates/edgeplane-tower/src/routes/docs.rs \
        crates/edgeplane-tower/src/routes/artifacts.rs \
        crates/edgeplane-tower/src/routes/explorer.rs
git commit -m "refactor(r1d): drop domainrolemembership access paths"
```

---

## Task 3: Remove the `/roles` endpoints and role models

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/domains.rs:16,20-28,80-89,272-273,280-335,362-386`
- Modify: `crates/edgeplane-tower/src/models/domain.rs:26-...,65-...`
- Modify: `crates/edgeplane-tower/src/models/mod.rs:15`

- [ ] **Step 1: Remove the `/roles` route registrations**

In `crates/edgeplane-tower/src/routes/domains.rs`, the `router()` fn (lines 20-28). Delete lines 26-27 (keep the `/owner` transfer route at line 25 — that is the admin-fix beneficiary, not a role route):

```rust
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/domains", get(list_domains).post(create_domain))
        .route("/domains/{domain_id}", get(get_domain).patch(update_domain).delete(delete_domain))
        .route("/domains/{domain_id}/northstar", get(get_domain_northstar_handler).put(put_domain_northstar_handler))
        .route("/domains/{domain_id}/owner", post(transfer_owner))
}
```

- [ ] **Step 2: Remove the role-model imports**

In `domains.rs`, change the import at line 16 to drop `DomainRoleMembership, DomainRoleUpsert`:

```rust
    models::domain::{Domain, DomainCreate, DomainUpdate, NorthstarUpdate},
```

- [ ] **Step 3: Remove `row_to_role`**

In `domains.rs`, delete the `row_to_role` function (lines 80-89):

```rust
fn row_to_role(row: &PgRow) -> DomainRoleMembership {
    DomainRoleMembership {
        id: row.get("id"),
        domain_id: row.get("domain_id"),
        subject: row.get("subject"),
        role: row.get("role"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
```

- [ ] **Step 4: Remove the cleanup DELETE in `delete_domain`**

In `domains.rs`, `delete_domain`, delete lines 272-273 (the membership cleanup — `let _ =` swallows its error today, so it is invisible to `cargo check`; this is exactly the kind of leftover Task 9's grep guards against):

```rust
    let _ = sqlx::query("DELETE FROM domainrolemembership WHERE domain_id = $1")
        .bind(&domain_id).execute(&state.db).await;
```

Leave the immediately following `DELETE FROM domain WHERE id = $1` intact.

- [ ] **Step 5: Remove the role handlers (`list_roles`, `upsert_role`, `delete_role`) — keep `transfer_owner`**

In `domains.rs`, delete the `// ── Role endpoints ──` section header and `list_roles` (lines 280-304) and `upsert_role` (lines 306-335). **Do not delete `transfer_owner` (lines 337-360)** — it stays. Then delete `delete_role` (lines 362-386). After this, `transfer_owner` should sit directly between `delete_domain` and the `// ── Northstar document endpoints ──` section.

- [ ] **Step 6: Remove the role structs from the model**

In `crates/edgeplane-tower/src/models/domain.rs`, delete `DomainRoleMembership` (struct at line 26, plus its `#[derive(...)]` attribute line directly above it) and `DomainRoleUpsert` (struct at line 65, plus its derive attribute).

- [ ] **Step 7: Remove the `DomainRoleMembership` re-export**

In `crates/edgeplane-tower/src/models/mod.rs`, change line 15 to drop `DomainRoleMembership` (keep `Domain`):

```rust
pub use domain::Domain;
```

- [ ] **Step 8: Verify compile**

Run: `cargo check -p edgeplane-tower 2>&1 | tail -10`
Expected: success. (If `PgRow` or `Utc` is now unused in domains.rs, remove the unused import the compiler names.)

- [ ] **Step 9: Commit**

```bash
git add crates/edgeplane-tower/src/routes/domains.rs \
        crates/edgeplane-tower/src/models/domain.rs \
        crates/edgeplane-tower/src/models/mod.rs
git commit -m "refactor(r1d): remove domain /roles endpoints and role models"
```

---

## Task 4: Delete the policy/approvals/family engine (tower)

**Files:**
- Delete: `crates/edgeplane-tower/src/routes/{governance.rs,approvals.rs,family_governance.rs}`
- Delete: `crates/edgeplane-tower/src/models/{governance.rs,approval.rs}`
- Modify: `crates/edgeplane-tower/src/routes/mod.rs:2,10,13,53,55,78`
- Modify: `crates/edgeplane-tower/src/models/mod.rs:2,6,14`
- Modify: `crates/edgeplane-tower/src/openapi.rs:55-101,268-270,282-290,325`
- Regenerate: `web/openapi.json`

`DEFAULT_POLICY` and its stale `skills.bundle.*` keys live inside `governance.rs`, so they vanish when the file is deleted — no separate step.

- [ ] **Step 1: Delete the route files**

```bash
git rm crates/edgeplane-tower/src/routes/governance.rs \
       crates/edgeplane-tower/src/routes/approvals.rs \
       crates/edgeplane-tower/src/routes/family_governance.rs
```

- [ ] **Step 2: Delete the model files**

```bash
git rm crates/edgeplane-tower/src/models/governance.rs \
       crates/edgeplane-tower/src/models/approval.rs
```

- [ ] **Step 3: Remove the route module declarations and merges**

In `crates/edgeplane-tower/src/routes/mod.rs`, delete the three `pub mod` lines: `pub mod approvals;` (line 2), `pub mod family_governance;` (line 10), `pub mod governance;` (line 13). Then in `build_router()` delete the three merge lines: `.merge(approvals::router())`, `.merge(governance::router())`, and `.merge(family_governance::router())`.

- [ ] **Step 4: Remove the model module declarations and re-exports**

In `crates/edgeplane-tower/src/models/mod.rs`, delete `pub mod approval;` (line 2), `pub mod governance;` (line 6), and `pub use approval::ApprovalRequest;` (line 14).

- [ ] **Step 5: Remove the governance OpenAPI surface**

In `crates/edgeplane-tower/src/openapi.rs`:
- Delete the three stub fns and their `#[utoipa::path(...)]` attributes: `governance_active_stub` (lines 55-68), `governance_reload_stub` (lines 70-83), `governance_events_stub` (lines 85-101).
- In the `paths(...)` list, delete `governance_active_stub,`, `governance_reload_stub,`, `governance_events_stub,` (lines 268-270).
- In the `components(schemas(...))` list, delete the `// governance` comment and the eight `crate::models::governance::*` schema lines (lines 282-290): `GovernancePolicyResponse, GovernanceReloadResponse, PolicyEvent, PolicyDoc, PolicyGlobal, PolicyTerminal, PolicyMcp, PolicyActionRule`.
- Delete the tag `(name = "governance", description = "Policy lifecycle management"),` (line 325).

- [ ] **Step 6: Verify compile (this is where the compiler earns its keep)**

Run: `cargo check -p edgeplane-tower 2>&1 | tail -20`
Expected: success. Any remaining `crate::models::governance::*` or `approval::*` reference is a hard compile error here — fix whatever the compiler names (it will point at the exact site).

- [ ] **Step 7: Regenerate `web/openapi.json`**

Run: `cargo run -p edgeplane-tower --bin gen-openapi > web/openapi.json`
Then confirm the governance paths are gone: `grep -c "governance" web/openapi.json` → expected `0`.

- [ ] **Step 8: Commit**

```bash
git add crates/edgeplane-tower/src/routes/mod.rs \
        crates/edgeplane-tower/src/models/mod.rs \
        crates/edgeplane-tower/src/openapi.rs web/openapi.json
git commit -m "refactor(r1d): delete policy/approvals/family engine (tower)"
```

---

## Task 5: Remove the CLI governance surface

**Files:**
- Delete: `crates/edgeplane/src/governance.rs`
- Modify: `crates/edgeplane/src/lib.rs:24`
- Modify: `crates/edgeplane/src/commands.rs:10,1062,1067,3609-3630`
- Modify: `crates/edgeplane/src/compat.rs:189`
- Regenerate: `docs/reference/COMMAND-MAP.md`

- [ ] **Step 1: Delete the CLI governance module**

```bash
git rm crates/edgeplane/src/governance.rs
```

- [ ] **Step 2: Remove the module declaration and command wiring**

- `crates/edgeplane/src/lib.rs`: delete `pub mod governance;` (line 24).
- `crates/edgeplane/src/commands.rs`: in the `use … {…}` module list (line 10), remove `governance,`. Delete the `Governance(governance::GovernanceCommand)` enum variant (line 1062) and its doc comment (line 1067 region). Delete the `AdminCommand::Governance(cmd) => governance::run(cmd, &client).await?,` handler arm (line 3630) and the inline governance-policy handler arms that call `/governance/policy/active`, `/governance/policy/versions`, `/governance/policy/events` (lines 3609-3626).

- [ ] **Step 3: Remove the compat placeholder reference**

In `crates/edgeplane/src/compat.rs:189`, the string literal `"full-mode check placeholder for governance/approval contract"` references the removed concept. Replace it with a still-meaningful placeholder for whatever check that arm represents, or remove the arm if it solely described the governance contract. Read the surrounding `compat` check list first and keep the change minimal; if unsure, change only the string text to drop "governance/approval".

- [ ] **Step 4: Verify compile**

Run: `cargo check -p edgeplane 2>&1 | tail -20`
Expected: success. Fix any compiler-named dangling references (e.g., a now-unused `use`).

- [ ] **Step 5: Regenerate the command map**

Run: `make docs`
This runs `cargo run -p edgeplane -- system internal gen-cli-doc > docs/reference/COMMAND-MAP.md`. Confirm governance commands are gone: `grep -c -i "governance" docs/reference/COMMAND-MAP.md` → expected `0`.

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplane/src/lib.rs crates/edgeplane/src/commands.rs \
        crates/edgeplane/src/compat.rs docs/reference/COMMAND-MAP.md
git commit -m "refactor(r1d): remove CLI governance commands"
```

---

## Task 6: Remove the TUI approval queue

**Files:**
- Delete: `crates/edgeplane/src/tui/screens/approval_queue.rs`
- Modify: `crates/edgeplane/src/tui/screens/mod.rs`, `crates/edgeplane/src/tui/app.rs`, `crates/edgeplane/src/tui/data.rs`, `crates/edgeplane/src/tui/work.rs`

This is the governance `approvalrequest` queue (`data.rs` calls `/approvals?…&status=pending`), **not** ACP. Removing it is correct.

- [ ] **Step 1: Delete the screen file**

```bash
git rm crates/edgeplane/src/tui/screens/approval_queue.rs
```

- [ ] **Step 2: Remove the screen module declaration and any screen-enum/dispatch entry**

In `crates/edgeplane/src/tui/screens/mod.rs`, remove the `pub mod approval_queue;` declaration and any `ApprovalQueue` variant in the screen enum / render dispatch / navigation table. Then in `crates/edgeplane/src/tui/app.rs`, remove the approval-queue screen wiring (its match arms, key bindings, and any `Screen::ApprovalQueue` references).

- [ ] **Step 3: Remove the approval data/work plumbing**

- `crates/edgeplane/src/tui/data.rs`: remove the `ApprovalSummary` type and the `list_approvals` / `respond_approval` trait methods + impls (the `// ─── approvals ───` block around lines 67-156 and the concrete impls around 188-288).
- `crates/edgeplane/src/tui/work.rs`: remove the `WorkRequest::{ListApprovals?, RespondApproval}` variants, the `WorkResult::{ApprovalsListed, ApprovalResponded}` variants, and their handler arms (lines ~43-46, 99-106, 267-286).

- [ ] **Step 4: Verify compile**

Run: `cargo check -p edgeplane 2>&1 | tail -20`
Expected: success. The compiler names every dangling `ApprovalSummary` / `list_approvals` / `ApprovalQueue` reference — remove each until green.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplane/src/tui/
git commit -m "refactor(r1d): remove TUI governance approval queue"
```

---

## Task 7: Remove the web governance route and nav

**Files:**
- Delete: `web/src/routes/governance.tsx`, `web/src/routes/governance.test.tsx`
- Modify: `web/src/components/shell/navModel.ts:19`, `web/src/components/shell/navModel.test.ts:5-7`, `web/src/components/shell/breadcrumbs.ts:11`, `web/src/components/shell/Sidebar.tsx:40`, `web/src/lib/queryKeys.ts:22-26`
- Regenerate: `web/src/api/schema.gen.ts`, `web/src/routeTree.gen.ts`

**Do not touch** `web/src/components/conversation/ApprovalPrompt.tsx` (ACP).

- [ ] **Step 1: Delete the route and its test**

```bash
git rm web/src/routes/governance.tsx web/src/routes/governance.test.tsx
```

- [ ] **Step 2: Remove the nav entries**

- `web/src/components/shell/navModel.ts`: delete `{ to: '/governance', label: 'Governance' },` (line 19).
- `web/src/components/shell/breadcrumbs.ts`: delete `'/governance': 'Governance',` (line 11).
- `web/src/components/shell/Sidebar.tsx`: delete `'/governance': '⚖',` (line 40).
- `web/src/lib/queryKeys.ts`: delete the `governance: { … }` block (lines 22-26).

- [ ] **Step 3: Update the nav test to match**

In `web/src/components/shell/navModel.test.ts` (lines 5-7), remove `Governance` from the description string and `'/governance'` from the expected `toEqual([...])` array:

```ts
  it('exposes Dashboard, Agents, Nodes, Domains, Feed, Admin', () => {
    const tos = ...;
    expect(tos).toEqual(['/', '/agents', '/nodes', '/domains', '/feed', '/admin']);
  });
```

(Confirm the exact remaining order against the edited `navModel.ts` — match the array to what the model now produces.)

- [ ] **Step 4: Regenerate the API schema from the new openapi.json**

Run: `cd web && npm run gen:api`
This runs `openapi-typescript ./openapi.json -o src/api/schema.gen.ts`. Confirm: `grep -c -i "governance" web/src/api/schema.gen.ts` → expected `0`.

- [ ] **Step 5: Build (regenerates `routeTree.gen.ts`) and run tests**

Run: `cd web && npm run build && npm run test`
Expected: build succeeds (the `tsc -b` step regenerates and type-checks `routeTree.gen.ts` with the governance route gone); vitest passes including the updated `navModel.test.ts`.

Note (known gotcha): `tsc -b` runs before vite regenerates the route tree, so if the build complains about a dangling `/governance` typed-route reference, ensure every importer of the governance route (nav, breadcrumbs, queryKeys, the test) is already cleaned (Steps 2-3) — then rebuild.

- [ ] **Step 6: Commit**

```bash
git add web/src/components/shell/navModel.ts web/src/components/shell/navModel.test.ts \
        web/src/components/shell/breadcrumbs.ts web/src/components/shell/Sidebar.tsx \
        web/src/lib/queryKeys.ts web/src/api/schema.gen.ts web/src/routeTree.gen.ts
git commit -m "refactor(r1d): remove web governance route and nav"
```

---

## Task 8: Drop the five tables (migration)

**Files:**
- Create: `crates/edgeplane-tower/migrations/0009_drop_governance.sql`

This lands only now — after Tasks 2-7 removed every code path that queries these tables.

- [ ] **Step 1: Create the migration**

Create `crates/edgeplane-tower/migrations/0009_drop_governance.sql`:

```sql
-- R1d governance simplification: drop the never-enforced policy/approvals/family
-- scaffolding and the half-wired domain role-membership table. All five are dead,
-- dormant, or half-broken; no code references them after this migration's PR.
-- Destructive and effectively irreversible (a down-migration would recreate empty
-- tables, not restore rows). DROP TABLE IF EXISTS is idempotent and safe on both
-- fresh and existing databases.
DROP TABLE IF EXISTS governancepolicy;
DROP TABLE IF EXISTS governancepolicyevent;
DROP TABLE IF EXISTS approvalrequest;
DROP TABLE IF EXISTS familymember;
DROP TABLE IF EXISTS domainrolemembership;
```

- [ ] **Step 2: Confirm exactly one migration head / clean ordering**

Run: `ls crates/edgeplane-tower/migrations/ | sort | tail -3`
Expected: `0007_…`, `0008_…`, `0009_drop_governance.sql` — `0009` sorts last (sqlx applies in filename order).

- [ ] **Step 3: Confirm the migrations still embed (compile-time macro)**

Run: `cargo check -p edgeplane-tower 2>&1 | tail -5`
Expected: success (`sqlx::migrate!("./migrations")` re-embeds the directory; a new `.sql` file does not break compilation).

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane-tower/migrations/0009_drop_governance.sql
git commit -m "feat(r1d): migration 0009 drops governance + role tables"
```

---

## Task 9: Final verification sweep (the real guard)

No new code. This is the gate that the runtime-SQL deletions (invisible to `cargo check`) are actually complete.

- [ ] **Step 1: Zero textual references to the dropped tables/identifiers in source**

Run:
```bash
cd /home/merlin/code/edgeplane
rg -n "domainrolemembership|governancepolicy|governancepolicyevent|approvalrequest|familymember" \
   crates/*/src web/src
```
Expected: **no output.** (The only allowed remaining mentions are `crates/edgeplane-tower/migrations/0001_initial_schema.sql` — historical — and the new `0009_drop_governance.sql`. Both are migrations, not in `src/`, so the command above excludes them.) Any hit here is a leftover the compiler could not see — fix it and re-run.

- [ ] **Step 2: Confirm the ACP approval flow is untouched**

Run: `git status --porcelain web/src/components/conversation/ApprovalPrompt.tsx`
Expected: **no output** (file unmodified).

- [ ] **Step 3: Full Rust check + clippy + tests**

Run:
```bash
cargo check -p edgeplane-tower -p edgeplane --all-targets
cargo clippy -p edgeplane-tower -p edgeplane --all-targets -- -D warnings
cargo nextest run -p edgeplane-tower -p edgeplane --no-fail-fast
```
Expected: all green, including the 5 `admin_email_tests` and the existing `mcp_parity` tests.

- [ ] **Step 4: Confirm the MCP catalogue is still exactly 23**

Run: `cargo nextest run -p edgeplane-tower http_catalogue_has_exactly_23_tools`
Expected: PASS (no MCP tools were removed; the count is unchanged).

- [ ] **Step 5: Web build + tests**

Run: `cd web && npm run build && npm run test`
Expected: green.

- [ ] **Step 6: Generated-artifact drift is clean**

Run: `git status --porcelain web/openapi.json web/src/api/schema.gen.ts web/src/routeTree.gen.ts docs/reference/COMMAND-MAP.md`
Expected: no *uncommitted* changes (all regenerated outputs were committed in their tasks). If any show as dirty, regenerate and commit — CI drift gates will otherwise block the PR.

- [ ] **Step 7: Deploy-verification note (manual; cannot be automated in this crate)**

Record in the PR description: the cross-owner admin behavior has no automated test (the tower test harness has no database). Verify post-deploy by setting `EP_ADMIN_EMAILS` to the exact `email` claim Authentik emits for the operator (confirm the live value — do not assume), logging in fresh, and hitting a pure-admin endpoint (e.g. `GET /api/ops/backups`) plus a cross-owner resource. With `EP_ADMIN_EMAILS` unset, behavior is identical to today (no admin) — fail-safe.

- [ ] **Step 8: Open the PR**

```bash
git push -u origin feat/r1d-governance
```
Then open the PR (do **not** merge — await Merlin's review per the engineer workflow). Body must cover: the admin model (`EP_ADMIN_EMAILS`, request-time, email-keyed), the destructive migration (5 tables, irreversible, acceptable), the bounded behavior change (only pure role-holders lose access), and the deploy-gating note from Step 7.

---

## Self-review (run against the spec)

**Spec coverage:**
- Goal 1 (operator admin) → Task 1. ✓
- Goal 2 (delete policy/approvals/family) → Tasks 4 (tower), 5 (CLI), 6 (TUI), 7 (web). ✓
- Goal 3 (remove `domainrolemembership`) → Tasks 2 (access paths), 3 (`/roles` + models), 8 (migration). ✓
- Spec Change 1 (config emails, request-time) → Task 1 (threaded via `AppConfig`, parsed in `main.rs` — refinement over the spec's "parse in `server.rs`", chosen for testability; behavior identical). ✓
- Spec Change 2 (strip engine, all surfaces) → Tasks 4-7. Includes two surfaces the spec under-specified and this plan pins: `models/approval.rs` (Task 4) and `openapi.rs` (Task 4). ✓
- Spec Change 3 (drop `domainrolemembership`) → Tasks 2-3; adds `domains.rs:272` cleanup DELETE (Task 3 Step 4) and the `rg` sweep (Task 9 Step 1), both absent from the spec's enumeration. ✓
- Migration → Task 8. ✓
- Testing & verification → Tasks 1 + 9; the spec's DB-dependent cross-owner test is honestly downgraded to deploy-verification (Task 9 Step 7) because no DB harness exists. ✓
- Deferred seam `(c)+(ii)` → not built (correct; no task). ✓

**Type/name consistency:** `is_admin_email(Option<&str>, &HashSet<String>) -> bool` is defined once (Task 1 Step 3) and called once (Task 1 Step 8) with `email.as_deref()` and `&state.admin_emails` — types line up. `AppState.admin_emails` and `AppConfig.admin_emails` are both `HashSet<String>`, threaded `config.admin_emails.clone()` → state. ✓

**Placeholder scan:** No "TBD"/"add error handling"/"similar to" — every code-changing step shows the code or names the exact symbol/line to delete. The two judgment spots (`compat.rs:189` string, `navModel.test.ts` array order) instruct reading the surrounding code first because their exact final text depends on sibling content not worth quoting in full. ✓

**Deviations from spec (intentional, flagged above):** admin emails threaded via `AppConfig` rather than parsed inside `server.rs`; `models/approval.rs` + `openapi.rs` added to the deletion set; `domains.rs:272` added; `rg` sweep added as the real completeness guard since runtime SQL strings are invisible to `cargo check`.
