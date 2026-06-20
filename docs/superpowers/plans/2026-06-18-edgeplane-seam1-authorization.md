# EdgePlane Seam 1 — Domain Authorization Implementation Plan

> **STATUS: SHIPPED in v0.15.0 (2026-06-19)** — PR #53 (Seam 1) / #56 (Seam 2), released #59. Live-validated. Read-side authz hardening followed in #62.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Revised 2026-06-18 after adversarial review.** Changes from v1: node principals are full-trust (was a fleet-outage bug); the trust-tier template allowlist is removed from P0 (deferred to spec §5); per-task lease enforcement added; the guarded-handler list is now complete (v1 missed `dispatch_task`, gates, `agent_notify_ws`, the unauthenticated `global_sse`, and several MCP arms); `authz_domain` uses the house `query + row` pattern (not `query_as`); a real test harness (Task 0) is added because the one v1 referenced (`crate::common::setup()`) never existed.

**Goal:** Add default-deny domain authorization to every privileged dispatch/ledger/stream action in `edgeplane-tower`, plus per-task ownership (lease) enforcement on lifecycle mutations — closing the live RCE-class hole where any authenticated token can dispatch/mutate work in any domain.

**Architecture:** A shared predicate `authorized_for_domain` in `auth.rs` (default deny: admin, OR `auth_type=="node"` first-party infra, OR `domain.id ∈ principal.domain_scope`, OR subject ∈ owners/contributors). A thin async guard `authz_domain(db, principal, domain_id) -> Result<(), Response>` loads the domain's `owners`/`contributors` and enforces it; every unguarded handler calls it after resolving its target domain (directly, via mission, via task, or via agent). On lifecycle mutations a second check requires the caller to hold the task's claim lease (unless full-trust/admin). `domain_scope` is added to `Principal` now (empty for all existing kinds) and is populated by the per-agent JWT in Seam 2.

**Tech Stack:** Rust, axum 0.8, sqlx (Postgres), `axum-test` for integration tests, `serde`.

## Global Constraints

- **edgeplane-only — zero aria dependency.** If aria-rs did not exist, every change here must still work.
- **All tower HTTP paths use the `/api/` prefix** (e.g. `/api/work/...`, `/api/mcp/...`).
- **Per-task green gate:** `cargo nextest run -p edgeplane-tower --no-fail-fast` and `cargo clippy -p edgeplane-tower -- -D warnings` both pass before commit. DB-backed tests run only when `TEST_DATABASE_URL` is set (Task 0); when unset they are skipped via the harness guard, and that is recorded — never silently treated as passing.
- **Rust toolchain pinned to 1.96.0.**
- **No new migration** — `meshtask.status` is free-text, so `pending_approval` is not introduced (the template tier is deferred); the lease check reads existing columns (`claim_lease_id`, `claimed_by_agent_id`).
- **Default deny:** the guard returns `403`/`404`/`422` on any failure path; no handler proceeds past an `Err` from the guard.
- **Datetimes UTC-aware** (`chrono::Utc::now().naive_utc()`).
- **No drift-gated artifact regen needed** — this plan adds no routes or DTOs (it only adds auth to existing handlers). If that changes, run `make docs` + regenerate `web/openapi.json` in the same commit.
- **Commits:** conventional-commit style, end every message with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

### Task 0: Integration-test harness (`tests/common`)

**Why first:** v1's tests called `crate::common::setup()` and a `ctx` fixture that do not exist; `build_app` does NOT run migrations (only `main.rs` does), and the only pool helper is a non-connecting lazy pool. Every DB-backed authz test depends on this harness. It is env-gated so CI without a Postgres still compiles and passes (tests skip).

**Files:**
- Create: `crates/edgeplane-tower/tests/common/mod.rs`

**Interfaces:**
- Produces:
  - `pub async fn setup() -> Option<(PgPool, Ctx)>` — `None` when `TEST_DATABASE_URL` is unset (caller returns early = skip). Otherwise connects, runs migrations, seeds, returns a migrated pool + `Ctx`.
  - `pub struct Ctx { pub domain_id: String, pub other_domain_id: String, pub mission_id: String, pub owner_session_token: String, pub outsider_sa_token: String, pub member_sa_token: String }`
  - `pub async fn mint_session(db, subject, email) -> String` and `pub async fn mint_sa(db, name) -> String` — insert real rows so the auth extractor accepts the returned raw token.

- [ ] **Step 1: Write the harness**

Create `crates/edgeplane-tower/tests/common/mod.rs`:

```rust
//! Shared integration-test harness: migrated Postgres + seeded domain/mission +
//! token minting. Env-gated on TEST_DATABASE_URL so DB-less CI just skips.
#![allow(dead_code)]
use edgeplane_tower::auth::hash_token; // pub fn hash_token(&str)->String (auth.rs)
use sqlx::PgPool;
use uuid::Uuid;

pub struct Ctx {
    pub domain_id: String,
    pub other_domain_id: String,
    pub mission_id: String,
    pub owner_session_token: String,
    pub outsider_sa_token: String,
    pub member_sa_token: String,
}

/// Insert a usersession row and return the raw `mcs_` token the extractor accepts.
pub async fn mint_session(db: &PgPool, subject: &str, email: &str) -> String {
    let token = format!("mcs_{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO usersession (subject, email, token_hash, revoked, expires_at, created_at, updated_at) \
         VALUES ($1,$2,$3,false, now() + interval '1 hour', now(), now())",
    )
    .bind(subject)
    .bind(email)
    .bind(hash_token(&token))
    .execute(db)
    .await
    .expect("insert usersession");
    token
}

/// Insert a serviceaccount + token, return the raw `mcs_sa_` token.
pub async fn mint_sa(db: &PgPool, name: &str) -> String {
    let token = format!("mcs_sa_{}", Uuid::new_v4().simple());
    let sa_id: i32 = sqlx::query_scalar(
        "INSERT INTO serviceaccount (name, revoked, created_at) VALUES ($1,false,now()) RETURNING id",
    )
    .bind(name)
    .fetch_one(db)
    .await
    .expect("insert serviceaccount");
    sqlx::query(
        "INSERT INTO serviceaccounttoken (service_account_id, token_hash, revoked, created_at) \
         VALUES ($1,$2,false,now())",
    )
    .bind(sa_id)
    .bind(hash_token(&token))
    .execute(db)
    .await
    .expect("insert serviceaccounttoken");
    token
}

pub async fn setup() -> Option<(PgPool, Ctx)> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let db = PgPool::connect(&url).await.expect("connect TEST_DATABASE_URL");
    sqlx::migrate!("./migrations").run(&db).await.expect("migrate");

    let domain_id = format!("dom-{}", Uuid::new_v4().simple());
    let other_domain_id = format!("dom-{}", Uuid::new_v4().simple());
    let owner_email = format!("owner-{}@example.com", Uuid::new_v4().simple());

    // Domain owned by owner_email; member SA is a contributor; outsider SA is neither.
    let member_token = mint_sa(&db, &format!("member-{}", Uuid::new_v4().simple())).await;
    let outsider_token = mint_sa(&db, &format!("outsider-{}", Uuid::new_v4().simple())).await;
    let owner_token = mint_session(&db, &owner_email, &owner_email).await;
    // The SA subject the extractor produces is `sa:{name}` — record it as a contributor.
    let member_sa_name = sqlx::query_scalar::<_, String>(
        "SELECT sa.name FROM serviceaccount sa JOIN serviceaccounttoken t ON t.service_account_id=sa.id WHERE t.token_hash=$1",
    )
    .bind(hash_token(&member_token))
    .fetch_one(&db)
    .await
    .unwrap();

    for did in [&domain_id, &other_domain_id] {
        sqlx::query(
            "INSERT INTO domain (id,name,description,owners,contributors,tags,visibility,status,\
             northstar_md,northstar_version,northstar_created_by,northstar_modified_by,created_at,updated_at) \
             VALUES ($1,$1,'',$2,$3,'','private','active','',0,'','',now(),now())",
        )
        .bind(did)
        .bind(&owner_email)
        .bind(format!("sa:{member_sa_name}"))
        .execute(&db)
        .await
        .expect("insert domain");
    }

    let mission_id = format!("mis-{}", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO mission (id,domain_id,title,status,created_at,updated_at) VALUES ($1,$2,'m','active',now(),now())")
        .bind(&mission_id)
        .bind(&domain_id)
        .execute(&db)
        .await
        .expect("insert mission");

    Some((db, Ctx {
        domain_id, other_domain_id, mission_id,
        owner_session_token: owner_token,
        outsider_sa_token: outsider_token,
        member_sa_token: member_token,
    }))
}
```

- [ ] **Step 2: Confirm the seed columns against the real schema**

Run: `rg -n 'CREATE TABLE public.(domain|mission|usersession|serviceaccount|serviceaccounttoken) ' crates/edgeplane-tower/migrations/0001_initial_schema.sql` and adjust the INSERT column lists to the actual NOT-NULL columns (the snippet covers the known ones; add any other NOT-NULL columns with sane defaults). Confirm `edgeplane_tower::auth::hash_token` is `pub` (it is — `auth.rs`); if not, make it `pub`.

- [ ] **Step 3: Compile the test crate**

Run: `cargo nextest run -p edgeplane-tower --no-run`
Expected: the test binary builds (no tests run yet).

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane-tower/tests/common/mod.rs
git commit -m "test(tower): env-gated integration harness (migrate + seed + token mint)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 1: Add `domain_scope` to `Principal`

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs` (struct ~36–42; the three extractor return sites ~122/165/199; the struct-level doc ~26–34)

**Interfaces:**
- Produces: `Principal { subject, is_admin, session_id, auth_type, domain_scope: Vec<String> }`. `domain_scope` is the domain ids the principal is intrinsically authorized for (empty for session/service_account/node today; the agent JWT fills it in Seam 2).

- [ ] **Step 1: Enumerate construction sites**

Run: `rg -n 'Principal\s*\{' crates/edgeplane-tower/src crates/edgeplane-tower/tests`
Expected (verified 2026-06-18): exactly 4 — the struct def + 3 extractor branches. `require_auth` does NOT construct one (it calls the extractor); `Principal` derives neither `Deserialize` nor `Default`. If the count differs, every literal needs the new field.

- [ ] **Step 2: Add the field + update both doc comments**

```rust
/// Caller identity extracted from request headers.
/// `auth_type` is one of "session", "service_account", "node", "agent".
#[derive(Clone)]
pub struct Principal {
    pub subject: String,
    pub is_admin: bool,
    pub session_id: Option<i32>,
    /// One of: "session", "service_account", "node", "agent".
    pub auth_type: String,
    /// Domain ids this principal is intrinsically authorized for (per-agent JWT).
    /// Empty for session/service_account/node.
    pub domain_scope: Vec<String>,
}
```

- [ ] **Step 3: Set `domain_scope: Vec::new()` at all three extractor return sites.**

- [ ] **Step 4: Compile** — `cargo check -p edgeplane-tower` → PASS (a missing-field error means a site was missed).

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplane-tower/src/auth.rs
git commit -m "feat(tower): add domain_scope to Principal

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: `authorized_for_domain` predicate (with node full-trust) + `is_full_trust`

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs`
- Reference: `crates/edgeplane-tower/src/models/domain.rs` (`Domain { id, owners, contributors, ... }`)

**Interfaces:**
- Produces:
  - `pub fn split_csv(s: &str) -> Vec<String>`
  - `pub fn authorized_for(domain_id: &str, owners: &str, contributors: &str, p: &Principal) -> bool` — pure core (so `authz_domain` need not build a full `Domain`).
  - `pub fn authorized_for_domain(domain: &Domain, p: &Principal) -> bool` — wrapper for `domains.rs`.
  - `pub fn is_full_trust(p: &Principal) -> bool` — `session` or `node` (humans + first-party infra). Used for lease-bypass (Task 8) and claim-as-other (Seam 2).

- [ ] **Step 1: Write the failing unit tests**

```rust
#[cfg(test)]
mod authz_tests {
    use super::*;

    fn principal(subject: &str, is_admin: bool, auth_type: &str, scope: &[&str]) -> Principal {
        Principal {
            subject: subject.into(),
            is_admin,
            session_id: None,
            auth_type: auth_type.into(),
            domain_scope: scope.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn admin_authorized_anywhere() {
        assert!(authorized_for("d1", "", "", &principal("x@x.com", true, "session", &[])));
    }
    #[test]
    fn node_is_full_trust_authorized_anywhere() {
        let p = principal("node:excalibur", false, "node", &[]);
        assert!(authorized_for("d1", "", "", &p));
        assert!(is_full_trust(&p));
    }
    #[test]
    fn owner_authorized_case_insensitive() {
        let p = principal("Alice@Example.COM", false, "session", &[]);
        assert!(authorized_for("d1", "alice@example.com", "", &p));
    }
    #[test]
    fn contributor_authorized() {
        let p = principal("sa:worker", false, "service_account", &[]);
        assert!(authorized_for("d1", "alice@x.com", "sa:worker", &p));
    }
    #[test]
    fn domain_scope_match_authorized() {
        let p = principal("agent:w7", false, "agent", &["d1", "d2"]);
        assert!(authorized_for("d2", "", "", &p));
    }
    #[test]
    fn outsider_denied() {
        let p = principal("sa:mallory", false, "service_account", &["d9"]);
        assert!(!authorized_for("d1", "alice@x.com", "sa:bob", &p));
    }
    #[test]
    fn only_sessions_and_nodes_are_full_trust() {
        assert!(!is_full_trust(&principal("sa:x", false, "service_account", &[])));
        assert!(!is_full_trust(&principal("agent:x", false, "agent", &[])));
        assert!(is_full_trust(&principal("u@x.com", false, "session", &[])));
        assert!(is_full_trust(&principal("node:n", false, "node", &[])));
    }
}
```

- [ ] **Step 2: Run → FAIL** (`cargo nextest run -p edgeplane-tower authz_tests`).

- [ ] **Step 3: Implement**

```rust
use crate::models::domain::Domain;

pub fn split_csv(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect()
}

/// Pure core of the domain authorization predicate. Default deny.
pub fn authorized_for(domain_id: &str, owners: &str, contributors: &str, p: &Principal) -> bool {
    if p.is_admin { return true; }
    if p.auth_type == "node" { return true; } // first-party infra, full-trust (P0; §5 scopes later)
    if p.domain_scope.iter().any(|d| d == domain_id) { return true; }
    let id = p.subject.to_lowercase();
    split_csv(owners).contains(&id) || split_csv(contributors).contains(&id)
}

pub fn authorized_for_domain(domain: &Domain, p: &Principal) -> bool {
    authorized_for(&domain.id, &domain.owners, &domain.contributors, p)
}

/// Full-trust principals: interactive human/admin sessions and first-party nodes.
/// They bypass the per-task lease check and may act on behalf of other agents.
pub fn is_full_trust(p: &Principal) -> bool {
    p.auth_type == "session" || p.auth_type == "node"
}
```

- [ ] **Step 4: Run → PASS** (7 tests).

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/auth.rs
git commit -m "feat(tower): authorized_for_domain predicate + node full-trust + is_full_trust

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Refactor `domains.rs` to use the shared predicate (DRY)

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/domains.rs:33–54`

- [ ] **Step 1: Delegate** — remove the local `split_csv`; rewrite the predicates:

```rust
use crate::auth::{authorized_for_domain, split_csv};

fn can_read(domain: &Domain, p: &Principal) -> bool {
    if domain.visibility.to_lowercase() == "public" { return true; }
    authorized_for_domain(domain, p)
}
fn can_write(domain: &Domain, p: &Principal) -> bool { authorized_for_domain(domain, p) }
fn can_own(domain: &Domain, p: &Principal) -> bool {
    if p.is_admin { return true; }
    split_csv(&domain.owners).contains(&p.subject.to_lowercase())
}
```

- [ ] **Step 2: `cargo nextest run -p edgeplane-tower --no-fail-fast`** → existing domain tests still PASS (predicate is identical, just relocated + node now full-trust, which only widens domain CRUD for nodes — acceptable).

- [ ] **Step 3: Clippy + commit**

```bash
cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/domains.rs
git commit -m "refactor(tower): domains.rs uses shared authorized_for_domain

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: `authz_domain` guard + resolvers (house query pattern)

**Files:**
- Create: `crates/edgeplane-tower/src/routes/authz.rs`
- Modify: `crates/edgeplane-tower/src/routes/mod.rs` (`pub(crate) mod authz;`)

**Interfaces:**
- Consumes: `crate::auth::{authorized_for, Principal}`; `sqlx::PgPool`.
- Produces:
  - `pub async fn authz_domain(db, p, domain_id) -> Result<(), Response>` — `422` on empty/unresolved domain, `404` if absent, `403` if unauthorized, `500` on DB error.
  - `pub async fn domain_id_for_mission(db, mission_id) -> Result<String, Response>`
  - `pub async fn domain_id_for_task(db, task_id) -> Result<String, Response>`
  - `pub async fn domain_id_for_agent(db, agent_id) -> Result<String, Response>`
  - `pub async fn domain_id_for_gate(db, gate_id) -> Result<String, Response>` (gate → task → domain)

Note: loads only `owners`/`contributors` via `sqlx::query(...).get(...)` — the house pattern (no `query_as::<_, Domain>`, which the codebase never uses and which couples to full-row migration state).

- [ ] **Step 1: Write the module**

```rust
//! Shared domain-authorization guard for privileged dispatch/ledger/stream handlers.
use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::auth::{authorized_for, Principal};

fn deny(status: StatusCode, detail: &str) -> Response {
    (status, Json(json!({ "detail": detail }))).into_response()
}

/// Load `domain_id`'s owners/contributors and authorize `p`. Default deny.
pub async fn authz_domain(db: &PgPool, p: &Principal, domain_id: &str) -> Result<(), Response> {
    if domain_id.is_empty() {
        return Err(deny(StatusCode::UNPROCESSABLE_ENTITY, "target has no domain"));
    }
    let row = sqlx::query("SELECT owners, contributors FROM domain WHERE id = $1")
        .bind(domain_id)
        .fetch_optional(db)
        .await
        .map_err(|e| { tracing::error!("authz_domain load {domain_id}: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() })?;
    let Some(row) = row else { return Err(deny(StatusCode::NOT_FOUND, "Domain not found")); };
    let owners: String = row.get("owners");
    let contributors: String = row.get("contributors");
    if authorized_for(domain_id, &owners, &contributors, p) {
        Ok(())
    } else {
        Err(deny(StatusCode::FORBIDDEN, "not authorized for domain"))
    }
}

async fn resolve(db: &PgPool, sql: &str, id: &str, missing: &str) -> Result<String, Response> {
    let v: Option<Option<String>> = sqlx::query_scalar(sql)
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(|e| { tracing::error!("resolver ({sql}) {id}: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() })?;
    v.flatten().ok_or_else(|| deny(StatusCode::NOT_FOUND, missing))
}

pub async fn domain_id_for_mission(db: &PgPool, mission_id: &str) -> Result<String, Response> {
    resolve(db, "SELECT domain_id FROM mission WHERE id=$1", mission_id, "Mission not found").await
}
pub async fn domain_id_for_task(db: &PgPool, task_id: &str) -> Result<String, Response> {
    resolve(db, "SELECT domain_id FROM meshtask WHERE id=$1", task_id, "Task not found").await
}
pub async fn domain_id_for_agent(db: &PgPool, agent_id: &str) -> Result<String, Response> {
    resolve(db, "SELECT domain_id FROM meshagent WHERE id=$1", agent_id, "Agent not found").await
}
pub async fn domain_id_for_gate(db: &PgPool, gate_id: &str) -> Result<String, Response> {
    // reviewgate → task → domain
    resolve(db,
        "SELECT t.domain_id FROM reviewgate g JOIN meshtask t ON t.id=g.task_id WHERE g.id=$1",
        gate_id, "Gate not found").await
}
```

(The `Option<Option<String>>` from `query_scalar` + `fetch_optional` is: outer `None` = no row, inner `None` = NULL `domain_id`; `.flatten()` collapses both to the not-found path. Confirm the gate→task join column names against `reviewgate` in `0001_initial_schema.sql`.)

- [ ] **Step 2: Register** — add `pub(crate) mod authz;` to `routes/mod.rs`.

- [ ] **Step 3: Compile** — `cargo check -p edgeplane-tower` → PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane-tower/src/routes/authz.rs crates/edgeplane-tower/src/routes/mod.rs
git commit -m "feat(tower): authz_domain guard + domain resolvers

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Guard the REST dispatch/agent/message handlers

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs`
- Test: `crates/edgeplane-tower/tests/test_authz.rs` (new)

**Interfaces:** consumes Task 4 resolvers + guard. Each handler already has `principal: Principal` + `State<Arc<AppState>>`. Insert the guard after the target id/domain is known, before the first write. On `Err(resp)` return `resp`.

**Complete list (verified 2026-06-18) by resolution group:**

| Group | Resolver | Handlers (file:line approx) |
|---|---|---|
| A — direct `domain_id` | none | `enroll_agent` (1666), `send_domain_message` (2121) |
| B — via `mission_id` | `domain_id_for_mission` | `create_task` (529, reuse its existing resolved `domain_id`), `send_mission_message` (2260) |
| C — via `task_id` | `domain_id_for_task` | `claim_task` (796), `complete_task` (1078), `fail_task` (1175), `cancel_task` (712), `retry_task` (754), `block_task` (1231), `unblock_task` (1327), `heartbeat_task` (947), `append_progress` (1007), **`dispatch_task` (1273)**, **`create_gate` (1412)** |
| C′ — via `gate_id` | `domain_id_for_gate` | **`resolve_gate` (1483)** |
| D — via `agent_id` | `domain_id_for_agent` | `agent_heartbeat` (1817), `set_agent_status` (1908), `update_agent_profile` (1942), `delete_agent` (1856, keep its existing `enrolled_by_subject` check too) |

Canonical insertion (Group C example):

```rust
let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
    Ok(d) => d,
    Err(resp) => return resp,
};
if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
    return resp;
}
```

Group A uses `&domain_id` directly; Group B resolves via mission (create_task already computes `domain_id` ~541 — insert the guard right after); C′ uses `domain_id_for_gate(&state.db, &gate_id)`; D uses `domain_id_for_agent`.

- [ ] **Step 1: Group A** — guard `enroll_agent`, `send_domain_message`.
- [ ] **Step 2: Group B** — guard `create_task` (reuse resolved `domain_id`), `send_mission_message`.
- [ ] **Step 3: Group C** — guard all 11 task-id handlers. For `append_progress` and any handler currently taking `_principal`, rename to `principal`. `dispatch_task` keeps its existing `created_by_subject` check AND gains the domain guard. Leave `claim_task`'s body `agent_id` as-is (Seam 2 hardens it).
- [ ] **Step 4: Group C′ + D** — guard `resolve_gate` (via gate), and the four agent-id handlers.
- [ ] **Step 5: Compile** — `cargo check -p edgeplane-tower` → PASS.

- [ ] **Step 6: Write integration tests** (`tests/test_authz.rs`):

```rust
mod common;
use axum_test::TestServer;
use common::setup;
use edgeplane_tower::{build_app, AppConfig};

fn server(pool: sqlx::PgPool) -> TestServer {
    TestServer::new(build_app(pool, AppConfig::default())).unwrap()
}

#[tokio::test]
async fn create_task_denied_for_outsider_sa() {
    let Some((pool, ctx)) = setup().await else { return; }; // skip without TEST_DATABASE_URL
    let s = server(pool);
    let res = s.post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header("authorization", format!("Bearer {}", ctx.outsider_sa_token))
        .json(&serde_json::json!({ "title": "pwn" })).await;
    assert_eq!(res.status_code(), 403);
}

#[tokio::test]
async fn create_task_allowed_for_owner_session() {
    let Some((pool, ctx)) = setup().await else { return; };
    let s = server(pool);
    let res = s.post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header("authorization", format!("Bearer {}", ctx.owner_session_token))
        .json(&serde_json::json!({ "title": "legit" })).await;
    assert_eq!(res.status_code(), 201);
}

#[tokio::test]
async fn create_task_allowed_for_member_sa_contributor() {
    let Some((pool, ctx)) = setup().await else { return; };
    let s = server(pool);
    let res = s.post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header("authorization", format!("Bearer {}", ctx.member_sa_token))
        .json(&serde_json::json!({ "title": "ok" })).await;
    assert_eq!(res.status_code(), 201); // SA is a contributor; no template gate at P0
}
```

- [ ] **Step 7: Run + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "feat(tower): enforce domain authz on all REST dispatch/agent/gate/message handlers

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Guard the MCP mesh handlers

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs` (the `dispatch()` arms; `dispatch` has `state: &AppState`, `principal: &Principal`)

**Complete list (verified 2026-06-18):** `submit_mesh_task` (209), `claim_mesh_task` (289), `heartbeat_mesh_task` (310), `progress_mesh_task` (328), `complete_mesh_task`/`fail_mesh_task`/`block_mesh_task` (349), `send_mesh_message` (374), `load_mission_workspace` (899), **`provision_domain_persistence` (492)**, **`publish_pending_ledger_events` (643)**.

Add the MCP helper (returns an MCP error `Value`):

```rust
async fn mcp_authz_domain(state: &AppState, p: &Principal, domain_id: &str) -> Result<(), Value> {
    match crate::routes::authz::authz_domain(&state.db, p, domain_id).await {
        Ok(()) => Ok(()),
        // Distinguish server errors from authz denials for debuggability.
        Err(resp) if resp.status() == axum::http::StatusCode::INTERNAL_SERVER_ERROR =>
            Err(json!({ "ok": false, "error": "database_error" })),
        Err(_) => Err(json!({ "ok": false, "error": "forbidden", "detail": "not authorized for domain" })),
    }
}
```

- [ ] **Step 1: `submit_mesh_task`** — drop `domain_id` from the required args; resolve it from the mission and use the resolved value in the INSERT (closes the client-supplied-domain mismatch):

```rust
let domain_id = match crate::routes::authz::domain_id_for_mission(&state.db, &mission_id).await {
    Ok(d) => d, Err(_) => return err_result("mission not found"),
};
if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await { return e; }
// ... INSERT binding the RESOLVED domain_id (not str_arg(args,"domain_id")) ...
```

- [ ] **Step 2: task-id arms** (`claim_mesh_task`, `heartbeat_mesh_task`, `progress_mesh_task`, `complete_mesh_task`, `fail_mesh_task`, `block_mesh_task`):

```rust
let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
    Ok(d) => d, Err(_) => return err_result("task not found"),
};
if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await { return e; }
```

- [ ] **Step 3: direct/mission arms** — `send_mesh_message` + `provision_domain_persistence` + `publish_pending_ledger_events` (direct `domain_id` arg); `load_mission_workspace` (via mission).
- [ ] **Step 4: parity test** — `cargo nextest run -p edgeplane-tower mcp_parity` → PASS (no arms added/removed).
- [ ] **Step 5: MCP authz test** in `tests/test_authz.rs`:

```rust
#[tokio::test]
async fn mcp_submit_mesh_task_denied_for_outsider() {
    let Some((pool, ctx)) = common::setup().await else { return; };
    let s = server(pool);
    let res = s.post("/api/mcp/call")
        .add_header("authorization", format!("Bearer {}", ctx.outsider_sa_token))
        .json(&serde_json::json!({ "tool": "submit_mesh_task",
            "args": { "mission_id": ctx.mission_id, "title": "pwn" } })).await;
    let body: serde_json::Value = res.json();
    assert_eq!(body["error"], "forbidden");
}
```

- [ ] **Step 6: Run + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/mcp.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "feat(tower): enforce domain authz on all MCP mesh handlers

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: Authorize the streams + notify WS, lock down `global_sse`

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs` (`domain_stream` ~2331, `mission_stream` ~2323, `agent_notify_ws` ~1609, `global_sse` ~2396)

**Interfaces:** these handlers currently take NO `Principal` (or discard `_principal`) and have NO auth. Add the extractor and gate before the upgrade/stream. Note: `WebSocketUpgrade`/`State`/`Path` are all `FromRequestParts`, so adding `principal: Principal` compiles (the existing `agent_notify_ws` already uses this quadruple). Change the return type to `Response` so the early `return resp;` unifies with the upgrade.

- [ ] **Step 1: `domain_stream`** — add `principal: Principal`; `authz_domain(&state.db,&principal,&domain_id)` before upgrade.
- [ ] **Step 2: `mission_stream`** — add `principal`; resolve domain via mission, then guard.
- [ ] **Step 3: `agent_notify_ws`** — promote `_principal`→`principal`; resolve domain via `domain_id_for_agent(&state.db,&agent_id)`, then guard, before subscribing.
- [ ] **Step 4: `global_sse`** — this streams **all** agents across **all** domains with no auth (cross-domain leak). Add `principal: Principal` and gate to **admin only**:

```rust
if !principal.is_admin {
    return (StatusCode::FORBIDDEN, Json(serde_json::json!({"detail":"admin required"}))).into_response();
}
```

- [ ] **Step 5: Compile** — `cargo check -p edgeplane-tower` → PASS.
- [ ] **Step 6: Test** (`tests/test_authz.rs`):

```rust
#[tokio::test]
async fn domain_stream_denied_for_outsider() {
    let Some((pool, ctx)) = common::setup().await else { return; };
    let s = server(pool);
    let res = s.get(&format!("/api/work/domains/{}/stream", ctx.other_domain_id))
        .add_header("authorization", format!("Bearer {}", ctx.outsider_sa_token)).await;
    assert_eq!(res.status_code(), 403);
}

#[tokio::test]
async fn global_sse_denied_for_non_admin() {
    let Some((pool, ctx)) = common::setup().await else { return; };
    let s = server(pool);
    let res = s.get("/api/work/events") // confirm the real global_sse route path
        .add_header("authorization", format!("Bearer {}", ctx.owner_session_token)).await;
    assert_eq!(res.status_code(), 403);
}
```

(Confirm `global_sse`'s registered route path with `rg -n 'global_sse' crates/edgeplane-tower/src/routes/work.rs`.)

- [ ] **Step 7: Run + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "feat(tower): authorize ledger/notify streams; admin-gate global_sse

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 8: Per-task ownership (lease) enforcement on lifecycle mutations

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs` (`complete_task` 1078, `fail_task` 1175, `block_task` 1231, `heartbeat_task` 947)
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs` (`complete_mesh_task`/`fail_mesh_task`/`block_mesh_task` 349, `heartbeat_mesh_task` 310)

**Interfaces:** consumes `is_full_trust` (Task 2). A non-full-trust caller may only mutate a task it holds — its subject matches `claimed_by_agent_id`, or it presents the matching `claim_lease_id`. Bounds a compromised agent to its own tasks (decided 2026-06-18, §9).

**Add a helper to `authz.rs`:**

```rust
/// After domain authz: a non-full-trust caller may only act on a task it holds.
/// Full-trust (session/node) and admin bypass. `lease_id` is the caller-presented
/// claim_lease_id (None for endpoints that don't take one).
pub async fn authz_task_owner(
    db: &PgPool, p: &Principal, task_id: &str, lease_id: Option<&str>,
) -> Result<(), Response> {
    if crate::auth::is_full_trust(p) || p.is_admin { return Ok(()); }
    let row = sqlx::query("SELECT claimed_by_agent_id, claim_lease_id FROM meshtask WHERE id=$1")
        .bind(task_id).fetch_optional(db).await
        .map_err(|e| { tracing::error!("authz_task_owner {task_id}: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() })?;
    let Some(row) = row else { return Err(deny(StatusCode::NOT_FOUND, "Task not found")); };
    let claimed: Option<String> = row.get("claimed_by_agent_id");
    let lease: Option<String> = row.get("claim_lease_id");
    let subject_id = p.subject.strip_prefix("agent:").unwrap_or(&p.subject);
    let owns = claimed.as_deref() == Some(subject_id)
        || (lease_id.is_some() && lease.as_deref() == lease_id);
    if owns { Ok(()) } else { Err(deny(StatusCode::FORBIDDEN, "not the task's claimer")) }
}
```

- [ ] **Step 1: REST** — in `complete_task`/`fail_task`/`block_task`/`heartbeat_task`, after the Task 5 domain guard, call `authz_task_owner(&state.db,&principal,&task_id, body_lease_id)` (extract the lease id the handler already reads, else `None`).
- [ ] **Step 2: MCP** — same in the four mesh arms, using `str_arg(args,"claim_lease_id")` (empty → `None`). Return `err_result("not the task's claimer")` on `Err`.
- [ ] **Step 3: Compile** — `cargo check -p edgeplane-tower` → PASS.
- [ ] **Step 4: Test** (`tests/test_authz.rs`): seed a task claimed by agent A (insert a meshtask row with `claimed_by_agent_id='A'`, `status='running'`); an SA/agent caller that is not A and presents no lease gets 403 on `complete_task`; the owner (or a full-trust session) succeeds. (Add a `seed_claimed_task(db, mission_id, claimed_by)` helper to `common`.)
- [ ] **Step 5: Run + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/src/routes/mcp.rs crates/edgeplane-tower/src/routes/authz.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "feat(tower): per-task lease ownership enforcement on lifecycle mutations

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Deferred from P0 (was v1 Tasks 8–9)

The **trust-tier template split** — `[dispatch_templates]` TOML config, restricting service-account/node tokens to an allowlist, infra-grade → `pending_approval`, and the `approve_task` endpoint — is moved to spec §5. No untrusted dispatch-token consumer exists at P0 (the node/daemon is full-trust, agents only claim/progress, humans are full-trust). Building it now broke the daemon and added an empty-allowlist outage hazard. The seam (`authorized_for` is `auth_type`-aware) is in place, so adding the restricted tier later is additive — no rework.

---

## Self-Review

**Spec coverage (Seam 1, post-revision):**
- `authorized_for_domain` default-deny + node full-trust + domain_scope → Tasks 1–2. ✔
- Applied to the **complete** verified handler set → Tasks 5 (REST: A/B/C/C′/D incl. dispatch_task, gates), 6 (MCP incl. progress/provision/publish), 7 (streams + notify WS + global_sse). ✔
- Per-task lease enforcement → Task 8. ✔
- Trust-tier template split → **deferred to §5** (intentional, documented above). ✔
- Real test harness → Task 0 (replaces v1's fictional `crate::common::setup()`). ✔

**Coverage honesty:** the handler list in Tasks 5–7 is the union of the 2026-06-17 investigation and the 2026-06-18 adversarial enumeration. Known acceptable exclusions: `commit_mission_workspace`/`release_mission_workspace`/`heartbeat_workspace_lease` retain their existing lease `actor_subject` owner-check (not domain-guarded — but note the Seam-2 migration concern: a workspace loaded under the old shared token can't be committed by a new per-agent subject); `transfer_owner` is admin-only already.

**Placeholder scan:** no "TBD"/"similar to Task N". Repetitive guards are full code per resolution group + a complete handler table.

**Type consistency:** `authorized_for(&str,&str,&str,&Principal)->bool`, `authz_domain(&PgPool,&Principal,&str)->Result<(),Response>`, `authz_task_owner(&PgPool,&Principal,&str,Option<&str>)->Result<(),Response>`, `is_full_trust(&Principal)->bool`, `domain_scope: Vec<String>` are used identically across tasks.

**Open verification items (confirm at execution — do NOT assume):**
1. Exact NOT-NULL column sets for the seed INSERTs (Task 0 Step 2).
2. `reviewgate` join column names for `domain_id_for_gate` (Task 4).
3. `global_sse`'s registered route path (Task 7).
4. The lease-id field name each lifecycle handler reads from its body/args (Task 8).
5. `hash_token` is `pub` in `auth.rs` (Task 0).

---

## Execution Handoff

1. **Subagent-Driven (recommended)** — fresh `rust-engineer` per task, `rust-reviewer` between tasks.
2. **Inline Execution** — checkpoints for review.

Tasks 0 → 8 are ordered by dependency; Task 0 (harness) must land first or every DB-backed test fails to compile.
