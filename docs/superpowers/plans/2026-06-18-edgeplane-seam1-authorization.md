# EdgePlane Seam 1 — Domain Authorization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add default-deny domain authorization to every privileged dispatch/ledger/stream action in `edgeplane-tower`, plus a trust-tier split that restricts service-account/node tokens to a TOML template allowlist — closing the live RCE-class hole where any authenticated token can dispatch arbitrary work to any domain.

**Architecture:** A single shared predicate `authorized_for_domain(domain, principal)` lives in `auth.rs` (default deny: admin, or subject ∈ owners/contributors, or domain.id ∈ principal.domain_scope). A thin async guard `authz_domain(db, principal, domain_id)` loads the domain and enforces it; every unguarded handler calls it after resolving its target domain (directly, via mission, or via task→mission). Human/admin sessions are full-trust (free-form task creation); service-account and node principals may only instantiate server-registered templates from a TOML allowlist, with infra-grade templates landing in a non-claimable `pending_approval` state. The `domain_scope` field is added to `Principal` now (empty for all existing principal kinds) and is populated by the per-agent JWT in Seam 2.

**Tech Stack:** Rust, axum, sqlx (Postgres), `axum-test` for integration tests, `toml` crate for config, `serde`.

## Global Constraints

- **edgeplane-only — zero aria dependency.** If aria-rs did not exist, every change here must still work.
- **All tower HTTP paths use the `/api/` prefix.** `EdgeplaneClient` prepends it; raw clients/tests must include it (e.g. `/api/work/...`, `/api/mcp/...`).
- **Per-task green gate:** `cargo nextest run -p edgeplane-tower --no-fail-fast` and `cargo clippy -p edgeplane-tower -- -D warnings` both pass before commit.
- **Rust toolchain pinned to 1.96.0** (workspace `rust-toolchain.toml`).
- **Migration discipline:** sqlx migrations are linear by filename (`migrations/NNNN_*.sql`); no `pending_approval` migration is needed (`meshtask.status` is a free-text column). Test against both a fresh and an existing DB.
- **Default deny:** any authorization helper returns `false`/`403` when in doubt.
- **Datetimes are UTC-aware** (`chrono::Utc::now().naive_utc()` matches existing handler convention).
- **Regenerate drift-gated artifacts in the same PR** if routes/DTOs change: `make docs` (COMMAND-MAP + docs-catalog) and `web/openapi.json`. The new `POST /api/work/tasks/{id}/approve` route (Task 9) triggers this.
- **Commits:** conventional-commit style, end every commit message with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

### Task 1: Add `domain_scope` to `Principal`

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs` (struct at ~36–42; three construction sites at ~126, ~169, ~199)
- Modify: any other `Principal { … }` construction site (find with grep — Step 1)

**Interfaces:**
- Produces: `Principal { subject: String, is_admin: bool, session_id: Option<i32>, auth_type: String, domain_scope: Vec<String> }` — `domain_scope` is the list of domain ids this principal is intrinsically scoped to (empty for session/service_account/node today; filled by the agent JWT in Seam 2).

- [ ] **Step 1: Enumerate every `Principal` construction site**

Run: `rg -n 'Principal\s*\{' crates/edgeplane-tower/src crates/edgeplane-tower/tests`
Expected: the struct definition plus the three extractor return sites in `auth.rs` (node, service_account, session branches) and any test constructors. Note each file:line — every one needs the new field.

- [ ] **Step 2: Add the field to the struct**

In `crates/edgeplane-tower/src/auth.rs`, extend the struct:

```rust
/// Caller identity extracted from request headers.
///
/// Note: `auth_type` is one of `"session"`, `"service_account"`, `"node"`, or `"agent"`.
#[derive(Clone)]
pub struct Principal {
    pub subject: String,
    pub is_admin: bool,
    pub session_id: Option<i32>,
    /// One of: "session", "service_account", "node", "agent".
    pub auth_type: String,
    /// Domain ids this principal is intrinsically authorized for, regardless of
    /// owners/contributors membership. Empty for session/service_account/node;
    /// populated from the per-agent JWT's `domain_id` claim (Seam 2). Used by
    /// `authorized_for_domain`.
    pub domain_scope: Vec<String>,
}
```

- [ ] **Step 3: Set `domain_scope: Vec::new()` at every construction site found in Step 1**

For each of the three extractor branches in `auth.rs` (and any test constructor), add `domain_scope: Vec::new(),` to the `Principal { … }` literal. Example (node branch, ~line 126):

```rust
return Ok(Principal {
    subject: claims.sub,
    is_admin: false,
    session_id: None,
    auth_type: "node".into(),
    domain_scope: Vec::new(),
});
```

- [ ] **Step 4: Compile**

Run: `cargo check -p edgeplane-tower`
Expected: PASS (a missing-field error here means a construction site was missed in Step 3 — fix it).

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplane-tower/src/auth.rs
git commit -m "feat(tower): add domain_scope to Principal

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Shared `split_csv` + `authorized_for_domain` predicate

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs` (add `split_csv`, `authorized_for`, `authorized_for_domain`, and a `#[cfg(test)]` module)
- Reference: `crates/edgeplane-tower/src/models/domain.rs` (the `Domain` type, fields `id`, `owners`, `contributors`)

**Interfaces:**
- Consumes: `Principal` (Task 1); `crate::models::domain::Domain`.
- Produces:
  - `pub fn split_csv(s: &str) -> Vec<String>` — lowercased, trimmed, non-empty CSV parts.
  - `pub fn authorized_for_domain(domain: &Domain, p: &Principal) -> bool` — the canonical predicate.

- [ ] **Step 1: Write the failing unit tests**

Add to `crates/edgeplane-tower/src/auth.rs`:

```rust
#[cfg(test)]
mod authz_tests {
    use super::*;

    fn principal(subject: &str, is_admin: bool, scope: &[&str]) -> Principal {
        Principal {
            subject: subject.into(),
            is_admin,
            session_id: None,
            auth_type: "session".into(),
            domain_scope: scope.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn admin_is_authorized_for_any_domain() {
        assert!(authorized_for("d1", "", "", &principal("anyone@x.com", true, &[])));
    }

    #[test]
    fn owner_is_authorized_case_insensitive() {
        let p = principal("Alice@Example.COM", false, &[]);
        assert!(authorized_for("d1", "alice@example.com,bob@example.com", "", &p));
    }

    #[test]
    fn contributor_is_authorized() {
        let p = principal("bob@example.com", false, &[]);
        assert!(authorized_for("d1", "alice@example.com", "bob@example.com", &p));
    }

    #[test]
    fn domain_scope_match_is_authorized() {
        let p = principal("agent:worker-7", false, &["d1", "d2"]);
        assert!(authorized_for("d2", "", "", &p));
    }

    #[test]
    fn unrelated_subject_is_denied() {
        let p = principal("mallory@evil.com", false, &["d9"]);
        assert!(!authorized_for("d1", "alice@example.com", "bob@example.com", &p));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -p edgeplane-tower authz_tests`
Expected: FAIL — `split_csv` / `authorized_for` not found.

- [ ] **Step 3: Implement the predicate**

Add to `crates/edgeplane-tower/src/auth.rs` (top-level, near `is_admin_email`):

```rust
use crate::models::domain::Domain;

/// Split a comma-separated subject list into normalized (trimmed, lowercased,
/// non-empty) entries. Matches the historical `domains.rs` behavior.
pub fn split_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_lowercase())
        .filter(|x| !x.is_empty())
        .collect()
}

/// Pure core of the domain authorization predicate. Default deny.
fn authorized_for(domain_id: &str, owners: &str, contributors: &str, p: &Principal) -> bool {
    if p.is_admin {
        return true;
    }
    if p.domain_scope.iter().any(|d| d == domain_id) {
        return true;
    }
    let id = p.subject.to_lowercase();
    split_csv(owners).contains(&id) || split_csv(contributors).contains(&id)
}

/// Canonical authorization predicate: may `principal` write/dispatch within `domain`?
///
/// True iff the principal is admin, is intrinsically scoped to the domain
/// (per-agent JWT), or appears in the domain's owners/contributors. Default deny.
pub fn authorized_for_domain(domain: &Domain, p: &Principal) -> bool {
    authorized_for(&domain.id, &domain.owners, &domain.contributors, p)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo nextest run -p edgeplane-tower authz_tests`
Expected: PASS (5 tests).

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/auth.rs
git commit -m "feat(tower): add authorized_for_domain predicate (default deny)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Refactor `domains.rs` to use the shared predicate (DRY)

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/domains.rs:33–54` (`split_csv`, `can_read`, `can_write`, `can_own`)

**Interfaces:**
- Consumes: `crate::auth::{authorized_for_domain, split_csv}` (Task 2).

- [ ] **Step 1: Replace the private helpers with delegations**

In `crates/edgeplane-tower/src/routes/domains.rs`, remove the local `split_csv` (lines ~33–35) and rewrite the three predicates to delegate. `can_write` becomes the shared predicate; `can_read` adds the public-visibility shortcut; `can_own` keeps the owners-only rule:

```rust
use crate::auth::{authorized_for_domain, split_csv};

fn can_read(domain: &Domain, p: &Principal) -> bool {
    if domain.visibility.to_lowercase() == "public" {
        return true;
    }
    authorized_for_domain(domain, p)
}

fn can_write(domain: &Domain, p: &Principal) -> bool {
    authorized_for_domain(domain, p)
}

fn can_own(domain: &Domain, p: &Principal) -> bool {
    if p.is_admin {
        return true;
    }
    let id = p.subject.to_lowercase();
    split_csv(&domain.owners).contains(&id)
}
```

- [ ] **Step 2: Compile + run existing domain tests**

Run: `cargo nextest run -p edgeplane-tower --no-fail-fast`
Expected: PASS — existing domain CRUD behavior unchanged (the predicate is identical, just relocated).

- [ ] **Step 3: Clippy + commit**

```bash
cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/domains.rs
git commit -m "refactor(tower): domains.rs uses shared authorized_for_domain

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: `authz_domain` guard + domain resolvers

**Files:**
- Create: `crates/edgeplane-tower/src/routes/authz.rs`
- Modify: `crates/edgeplane-tower/src/routes/mod.rs` (add `pub(crate) mod authz;`)

**Interfaces:**
- Consumes: `Principal`, `authorized_for_domain` (Tasks 1–2); `Domain` (FromRow); `sqlx::PgPool`.
- Produces:
  - `pub async fn authz_domain(db: &PgPool, p: &Principal, domain_id: &str) -> Result<Domain, Response>` — loads the domain; `Ok(domain)` if authorized, else `Err(403)`; `Err(404)` if the domain doesn't exist; `Err(500)` on DB error.
  - `pub async fn domain_id_for_mission(db: &PgPool, mission_id: &str) -> Result<String, Response>` — `Err(404)` if mission missing.
  - `pub async fn domain_id_for_task(db: &PgPool, task_id: &str) -> Result<String, Response>` — resolves task → `domain_id` column; `Err(404)` if task missing.
  - `pub async fn domain_id_for_agent(db: &PgPool, agent_id: &str) -> Result<String, Response>` — resolves meshagent → `domain_id`; `Err(404)` if agent missing.

- [ ] **Step 1: Write the guard module**

Create `crates/edgeplane-tower/src/routes/authz.rs`:

```rust
//! Shared domain-authorization guard used by every privileged dispatch/ledger
//! handler. See docs/superpowers/specs/2026-06-17-edgeplane-layered-tenancy-...
use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use sqlx::PgPool;

use crate::auth::{authorized_for_domain, Principal};
use crate::models::domain::Domain;

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "detail": "not authorized for domain" })),
    )
        .into_response()
}

fn not_found(what: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "detail": what }))).into_response()
}

/// Load `domain_id` and authorize `p` to write it. Default deny.
pub async fn authz_domain(db: &PgPool, p: &Principal, domain_id: &str) -> Result<Domain, Response> {
    let domain = sqlx::query_as::<_, Domain>("SELECT * FROM domain WHERE id = $1")
        .bind(domain_id)
        .fetch_optional(db)
        .await
        .map_err(|e| {
            tracing::error!("authz_domain load {domain_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?
        .ok_or_else(|| not_found("Domain not found"))?;
    if authorized_for_domain(&domain, p) {
        Ok(domain)
    } else {
        Err(forbidden())
    }
}

async fn scalar_lookup(db: &PgPool, sql: &str, id: &str, missing: &str) -> Result<String, Response> {
    sqlx::query_scalar::<_, Option<String>>(sql)
        .bind(id)
        .fetch_optional(db)
        .await
        .map_err(|e| {
            tracing::error!("domain resolver ({sql}) for {id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?
        .flatten()
        .ok_or_else(|| not_found(missing))
}

pub async fn domain_id_for_mission(db: &PgPool, mission_id: &str) -> Result<String, Response> {
    scalar_lookup(db, "SELECT domain_id FROM mission WHERE id = $1", mission_id, "Mission not found").await
}

pub async fn domain_id_for_task(db: &PgPool, task_id: &str) -> Result<String, Response> {
    scalar_lookup(db, "SELECT domain_id FROM meshtask WHERE id = $1", task_id, "Task not found").await
}

pub async fn domain_id_for_agent(db: &PgPool, agent_id: &str) -> Result<String, Response> {
    scalar_lookup(db, "SELECT domain_id FROM meshagent WHERE id = $1", agent_id, "Agent not found").await
}

// Keep Arc<AppState> import paths consistent for callers.
pub type SharedState = Arc<crate::AppState>;
```

- [ ] **Step 2: Register the module**

In `crates/edgeplane-tower/src/routes/mod.rs`, add alongside the other `mod` lines:

```rust
pub(crate) mod authz;
```

- [ ] **Step 3: Compile**

Run: `cargo check -p edgeplane-tower`
Expected: PASS. (If `Domain` is not `pub` from `models`, add `pub use` as needed — confirm with `rg -n 'pub struct Domain' crates/edgeplane-tower/src`.)

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane-tower/src/routes/authz.rs crates/edgeplane-tower/src/routes/mod.rs
git commit -m "feat(tower): authz_domain guard + domain resolvers

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Guard the REST dispatch/agent/message handlers

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs` (handlers enumerated below)
- Test: `crates/edgeplane-tower/tests/test_authz.rs` (new)

**Interfaces:**
- Consumes: `crate::routes::authz::{authz_domain, domain_id_for_mission, domain_id_for_task, domain_id_for_agent}` (Task 4). Every handler already receives `principal: Principal` and `State<Arc<AppState>>`.

The handlers fall into three resolution groups. The guard call is inserted **after the handler has its `principal` and target id, before its first DB write**. On `Err(resp)` return `resp` directly (it is already a `Response`).

**Group A — direct `domain_id` (path or body param):** `enroll_agent` (~1666), `send_domain_message` (~2121).

```rust
// after `Path(domain_id)` is in scope, before any insert:
if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
    return resp;
}
```

**Group B — via `mission_id`:** `create_task` (~529, resolves `domain_id` from the mission already — reuse it), `send_mission_message` (~2260).

```rust
// create_task: it already computes `domain_id` from the mission at ~541.
// Insert the guard immediately after that block:
if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
    return resp;
}
// send_mission_message: resolve first.
let domain_id = match crate::routes::authz::domain_id_for_mission(&state.db, &mission_id).await {
    Ok(d) => d,
    Err(resp) => return resp,
};
if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
    return resp;
}
```

**Group C — via `task_id`:** `claim_task` (~796), `complete_task` (~1078), `fail_task` (~1175), `cancel_task` (~712), `retry_task` (~754), `block_task` (~1231), `unblock_task` (~1327), `heartbeat_task` (~947), `append_progress` (~1007).

```rust
// after `Path(task_id)` is in scope, before the status-changing UPDATE/INSERT:
let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
    Ok(d) => d,
    Err(resp) => return resp,
};
if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
    return resp;
}
```

**Group D — via `agent_id`:** `agent_heartbeat` (~1817), `set_agent_status` (~1908), `update_agent_profile` (~1942). `delete_agent` (~1856) keeps its existing `enrolled_by_subject` owner-check AND adds the domain guard.

```rust
let domain_id = match crate::routes::authz::domain_id_for_agent(&state.db, &agent_id).await {
    Ok(d) => d,
    Err(resp) => return resp,
};
if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
    return resp;
}
```

- [ ] **Step 1: Apply Group A guards** (`enroll_agent`, `send_domain_message`). For `append_progress`, also un-prefix the unused `_principal` to `principal` (Task 8 of Seam 2 uses it; for now the guard uses it).

- [ ] **Step 2: Apply Group B guards** (`create_task`, `send_mission_message`).

- [ ] **Step 3: Apply Group C guards** (the nine task-mutation handlers). For `claim_task`, leave the existing `agent_id` body-fallback as-is for now; Seam 2 Task 5 hardens arbitrary `agent_id`.

- [ ] **Step 4: Apply Group D guards** (`agent_heartbeat`, `set_agent_status`, `update_agent_profile`, `delete_agent`).

- [ ] **Step 5: Compile**

Run: `cargo check -p edgeplane-tower`
Expected: PASS.

- [ ] **Step 6: Write the integration test** (DB-backed — follow the pool/migration setup in `crates/edgeplane-tower/tests/test_work.rs`; these run against a migrated test Postgres)

Create `crates/edgeplane-tower/tests/test_authz.rs`:

```rust
//! Domain-authorization enforcement on dispatch handlers.
use axum_test::TestServer;
use edgeplane_tower::{build_app, AppConfig};
// NOTE: reuse the migrated-pool + domain/mission/service-account seed helpers
// from test_work.rs (copy the `setup()` helper or extract it to a shared module).

#[tokio::test]
async fn create_task_denied_for_non_member_service_account() {
    let (pool, ctx) = crate::common::setup().await; // domain owned by alice@x.com; mission in it
    let server = TestServer::new(build_app(pool, AppConfig::default())).unwrap();
    // A service-account token NOT in the domain's owners/contributors:
    let res = server
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header("authorization", format!("Bearer {}", ctx.outsider_sa_token))
        .json(&serde_json::json!({ "title": "pwn" }))
        .await;
    assert_eq!(res.status_code(), 403);
}

#[tokio::test]
async fn create_task_allowed_for_domain_owner() {
    let (pool, ctx) = crate::common::setup().await;
    let server = TestServer::new(build_app(pool, AppConfig::default())).unwrap();
    let res = server
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header("authorization", format!("Bearer {}", ctx.owner_session_token))
        .json(&serde_json::json!({ "title": "legit" }))
        .await;
    assert_eq!(res.status_code(), 201);
}
```

- [ ] **Step 7: Run tests**

Run: `cargo nextest run -p edgeplane-tower --no-fail-fast`
Expected: PASS (the two authz tests + all existing tests). If the test DB is unavailable in this environment, mark the DB-backed tests `#[ignore]` with a `// requires test Postgres` note and record that they were not run.

- [ ] **Step 8: Clippy + commit**

```bash
cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "feat(tower): enforce domain authz on REST dispatch/agent/message handlers

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Guard the MCP mesh handlers

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs` (the `dispatch()` match arms; `dispatch` already takes `principal: &Principal` and `state: &AppState`)

**Interfaces:**
- Consumes: the Task 4 resolvers. Note `dispatch` has `state: &AppState` (not `Arc`), so call `authz_domain(&state.db, principal, &domain_id)`.

The MCP arms that write and currently lack authz: `submit_mesh_task` (~209), `claim_mesh_task` (~289), `heartbeat_mesh_task` (~310), `progress_mesh_task` (~328), `complete_mesh_task` / `fail_mesh_task` / `block_mesh_task` (~349), `send_mesh_message` (~374), `load_mission_workspace` (~899).

Because `dispatch` returns a `Value` (not a `Response`), add a small local helper at the top of `mcp.rs`:

```rust
/// Authorize a domain inside the MCP dispatcher; returns an MCP error `Value` on failure.
async fn mcp_authz_domain(state: &AppState, p: &Principal, domain_id: &str) -> Result<(), Value> {
    match crate::routes::authz::authz_domain(&state.db, p, domain_id).await {
        Ok(_) => Ok(()),
        Err(_) => Err(json!({ "ok": false, "error": "forbidden", "detail": "not authorized for domain" })),
    }
}
```

- [ ] **Step 1: Guard `submit_mesh_task`** — it accepts BOTH `mission_id` and a client `domain_id`. Resolve the canonical domain from the mission and ignore the client-supplied one (closes the mismatch hole):

```rust
"submit_mesh_task" => {
    let mission_id = str_arg(args, "mission_id");
    let title = str_arg(args, "title");
    if mission_id.is_empty() || title.is_empty() {
        return err_result("mission_id and title are required");
    }
    let domain_id = match crate::routes::authz::domain_id_for_mission(&state.db, &mission_id).await {
        Ok(d) => d,
        Err(_) => return err_result("mission not found"),
    };
    if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
        return e;
    }
    // ... existing INSERT, binding the resolved domain_id ...
}
```

- [ ] **Step 2: Guard the task-id arms** (`claim_mesh_task`, `heartbeat_mesh_task`, `progress_mesh_task`, `complete_mesh_task`, `fail_mesh_task`, `block_mesh_task`):

```rust
let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
    Ok(d) => d,
    Err(_) => return err_result("task not found"),
};
if let Err(e) = mcp_authz_domain(state, principal, &domain_id).await {
    return e;
}
```

- [ ] **Step 3: Guard `send_mesh_message`** (direct `domain_id` arg) and `load_mission_workspace` (resolve via `mission_id`).

- [ ] **Step 4: Compile + parity test**

Run: `cargo nextest run -p edgeplane-tower mcp_parity --no-fail-fast`
Expected: PASS (the catalogue↔dispatch parity must still hold — no arms added/removed, only guarded).

- [ ] **Step 5: Add an MCP authz test** to `tests/test_authz.rs`:

```rust
#[tokio::test]
async fn mcp_submit_mesh_task_denied_for_outsider() {
    let (pool, ctx) = crate::common::setup().await;
    let server = TestServer::new(build_app(pool, AppConfig::default())).unwrap();
    let res = server
        .post("/api/mcp/call")
        .add_header("authorization", format!("Bearer {}", ctx.outsider_sa_token))
        .json(&serde_json::json!({
            "tool": "submit_mesh_task",
            "args": { "mission_id": ctx.mission_id, "title": "pwn" }
        }))
        .await;
    let body: serde_json::Value = res.json();
    assert_eq!(body["error"], "forbidden");
}
```

- [ ] **Step 6: Run + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/mcp.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "feat(tower): enforce domain authz on MCP mesh handlers

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: Authorize the ledger streams

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs` (`domain_stream` ~2331, `mission_stream` ~2323)

**Interfaces:**
- Consumes: Task 4 guard + resolvers. These handlers currently take NO `Principal` and have NO auth — add the extractor and gate before `ws.on_upgrade`.

- [ ] **Step 1: Add auth to `domain_stream`**

```rust
async fn domain_stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
) -> impl IntoResponse {
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    ws.on_upgrade(move |socket| poll_ledger_stream(socket, state, "domain_id".into(), domain_id))
}
```

Note: the return type must unify the `Response` (from `resp`) and the upgrade. `WebSocketUpgrade::on_upgrade` returns `Response`; the early `return resp;` is also a `Response`. Change the signature to `-> Response` and `.into_response()` the upgrade if the compiler requires it.

- [ ] **Step 2: Add auth to `mission_stream`** (resolve domain via mission first):

```rust
async fn mission_stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    let domain_id = match crate::routes::authz::domain_id_for_mission(&state.db, &mission_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    ws.on_upgrade(move |socket| poll_ledger_stream(socket, state, "mission_id".into(), mission_id))
}
```

- [ ] **Step 3: Compile**

Run: `cargo check -p edgeplane-tower`
Expected: PASS. (Adding a `Principal` FromRequestParts extractor to a `get` WebSocket handler is supported — `Principal` implements `FromRequestParts`.)

- [ ] **Step 4: Add a stream authz test** to `tests/test_authz.rs` — a WS upgrade with an outsider token must return 403 before upgrade:

```rust
#[tokio::test]
async fn domain_stream_denied_for_outsider() {
    let (pool, ctx) = crate::common::setup().await;
    let server = TestServer::new(build_app(pool, AppConfig::default())).unwrap();
    let res = server
        .get(&format!("/api/work/domains/{}/stream", ctx.domain_id))
        .add_header("authorization", format!("Bearer {}", ctx.outsider_sa_token))
        .await;
    assert_eq!(res.status_code(), 403);
}
```

- [ ] **Step 5: Run + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "feat(tower): authorize domain/mission ledger streams

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 8: TOML config loader + `DispatchTemplate` types

**Files:**
- Create: `crates/edgeplane-tower/src/config.rs`
- Modify: `crates/edgeplane-tower/src/server.rs:15–23` (`AppConfig`), `crates/edgeplane-tower/src/main.rs:68–78` (load + populate)
- Modify: `crates/edgeplane-tower/Cargo.toml` (add `toml = "0.8"` if not already a dependency — check first)

**Interfaces:**
- Produces:
  - `pub struct DispatchTemplate { pub name: String, pub allowed_params: Vec<String>, pub infra_grade: bool }`
  - `pub fn load_file_config() -> HashMap<String, DispatchTemplate>` — reads `EP_CONFIG_FILE` (default `/etc/edgeplane/config.toml`); missing file → empty map; parse error → log + empty map.
  - `AppConfig.dispatch_templates: Arc<HashMap<String, DispatchTemplate>>` consumed by Task 9.

- [ ] **Step 1: Confirm/add the `toml` dependency**

Run: `rg -n '^toml' crates/edgeplane-tower/Cargo.toml`
If absent, add under `[dependencies]`: `toml = "0.8"`.

- [ ] **Step 2: Write the failing unit test**

Create `crates/edgeplane-tower/src/config.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

/// A server-registered task template that service-account / node principals are
/// permitted to instantiate (Seam 1 trust-tier split).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DispatchTemplate {
    pub name: String,
    #[serde(default)]
    pub allowed_params: Vec<String>,
    /// Infra-grade templates land in non-claimable `pending_approval` state.
    #[serde(default)]
    pub infra_grade: bool,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    dispatch_templates: Vec<DispatchTemplate>,
}

/// Parse a TOML config string into the template map keyed by `name`.
pub fn parse_templates(toml_str: &str) -> HashMap<String, DispatchTemplate> {
    let fc: FileConfig = toml::from_str(toml_str).unwrap_or_else(|e| {
        tracing::warn!("config parse error: {e}; ignoring dispatch_templates");
        FileConfig::default()
    });
    fc.dispatch_templates
        .into_iter()
        .map(|t| (t.name.clone(), t))
        .collect()
}

/// Load the template allowlist from `EP_CONFIG_FILE` (default
/// `/etc/edgeplane/config.toml`). Missing file → empty map (zero ceremony).
pub fn load_file_config() -> Arc<HashMap<String, DispatchTemplate>> {
    let path = std::env::var("EP_CONFIG_FILE")
        .unwrap_or_else(|_| "/etc/edgeplane/config.toml".to_string());
    match std::fs::read_to_string(&path) {
        Ok(s) => Arc::new(parse_templates(&s)),
        Err(_) => Arc::new(HashMap::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_templates_and_infra_flag() {
        let toml_str = r#"
            [[dispatch_templates]]
            name = "run-ceph-doctor"
            allowed_params = ["target"]

            [[dispatch_templates]]
            name = "reboot-node"
            allowed_params = ["node_id"]
            infra_grade = true
        "#;
        let m = parse_templates(toml_str);
        assert_eq!(m.len(), 2);
        assert!(!m["run-ceph-doctor"].infra_grade);
        assert!(m["reboot-node"].infra_grade);
        assert_eq!(m["run-ceph-doctor"].allowed_params, vec!["target".to_string()]);
    }

    #[test]
    fn empty_on_garbage() {
        assert!(parse_templates("not valid toml :::").is_empty());
    }
}
```

- [ ] **Step 3: Register the module + run the test**

In `crates/edgeplane-tower/src/lib.rs` (or wherever modules are declared), add `pub mod config;`.
Run: `cargo nextest run -p edgeplane-tower config::tests`
Expected: PASS (2 tests).

- [ ] **Step 4: Extend `AppConfig` and populate it**

In `server.rs`, add to `AppConfig`:

```rust
pub dispatch_templates: std::sync::Arc<std::collections::HashMap<String, crate::config::DispatchTemplate>>,
```

`AppConfig` derives `Default`; `Arc<HashMap>` defaults to an empty map, so `AppConfig::default()` (used throughout the tests) still works. In `main.rs` where `AppConfig { … }` is built (~68), add:

```rust
dispatch_templates: crate::config::load_file_config(),
```

Ensure the field flows into `AppState` (mirror how `admin_emails` is carried). Confirm with `rg -n 'admin_emails' crates/edgeplane-tower/src`.

- [ ] **Step 5: Compile + full test + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/config.rs crates/edgeplane-tower/src/server.rs crates/edgeplane-tower/src/main.rs crates/edgeplane-tower/src/lib.rs crates/edgeplane-tower/Cargo.toml
git commit -m "feat(tower): TOML dispatch-template allowlist config

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 9: Trust-tier enforcement + `pending_approval` + approve endpoint

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs` (add `is_full_trust`)
- Modify: `crates/edgeplane-tower/src/routes/work.rs` (`create_task`, new `approve_task`, route registration)
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs` (`submit_mesh_task`)
- Test: `crates/edgeplane-tower/tests/test_authz.rs`

**Interfaces:**
- Consumes: `AppConfig.dispatch_templates` (Task 8); `authz_domain` (Task 4).
- Produces:
  - `pub fn is_full_trust(p: &Principal) -> bool` — `p.auth_type == "session"` (humans). Service-account/node/agent are restricted.
  - New route `POST /api/work/tasks/{task_id}/approve` → `approve_task` (owner/admin flips `pending_approval` → `ready`).
  - Task status value `pending_approval` (non-claimable: claim queries already filter `status='ready'`).

- [ ] **Step 1: Add the trust-tier helper + unit test**

In `auth.rs`:

```rust
/// Full-trust principals (interactive human/admin sessions) may create free-form
/// tasks. Service-account, node, and agent principals are template-restricted.
pub fn is_full_trust(p: &Principal) -> bool {
    p.auth_type == "session"
}
```

Add to the `authz_tests` module:

```rust
#[test]
fn only_sessions_are_full_trust() {
    let mut p = principal("svc", false, &[]);
    p.auth_type = "service_account".into();
    assert!(!is_full_trust(&p));
    p.auth_type = "session".into();
    assert!(is_full_trust(&p));
}
```

- [ ] **Step 2: Run the unit test**

Run: `cargo nextest run -p edgeplane-tower authz_tests::only_sessions_are_full_trust`
Expected: PASS.

- [ ] **Step 3: Enforce the trust-tier split in `create_task`**

After the domain guard (Task 5, Group B) and before computing `initial_status`, branch on trust. Restricted principals must name an allowlisted template via `body.input_json` (parse `{"template": "<name>", "params": {…}}`); infra-grade → `pending_approval`:

```rust
// Trust-tier split (Seam 1): restricted principals may only instantiate
// server-registered templates with constrained params.
let mut template_forced_status: Option<&'static str> = None;
if !crate::auth::is_full_trust(&principal) {
    let parsed: serde_json::Value =
        serde_json::from_str(&body.input_json).unwrap_or(serde_json::json!({}));
    let template_name = parsed.get("template").and_then(|v| v.as_str()).unwrap_or("");
    let Some(tmpl) = state.config.dispatch_templates.get(template_name) else {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "detail": format!("service-account principals may only dispatch allowlisted templates; '{template_name}' is not registered")
            })),
        )
            .into_response();
    };
    // Reject params not in the template's allowlist.
    if let Some(params) = parsed.get("params").and_then(|v| v.as_object()) {
        for k in params.keys() {
            if !tmpl.allowed_params.contains(k) {
                return bad_request(&format!("param '{k}' not allowed for template '{}'", tmpl.name));
            }
        }
    }
    if tmpl.infra_grade {
        template_forced_status = Some("pending_approval");
    }
}
```

Then where `initial_status` is finalized, let the template force it:

```rust
let initial_status = template_forced_status.unwrap_or(initial_status);
```

(Confirm the `AppState` field holding `AppConfig` is reachable as `state.config` — verify with `rg -n 'pub config' crates/edgeplane-tower/src/server.rs`; adjust the accessor if the field is named differently, e.g. `state.app_config`.)

- [ ] **Step 4: Mirror the split in MCP `submit_mesh_task`** — same `is_full_trust` check against `state.config.dispatch_templates`, forcing `pending_approval` for infra-grade templates instead of `'ready'`.

- [ ] **Step 5: Add the `approve_task` handler + route**

```rust
async fn approve_task(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let domain_id = match crate::routes::authz::domain_id_for_task(&state.db, &task_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    // Only owners/admins approve (reuse can_own semantics via authz_domain + admin).
    let domain = match crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if !principal.is_admin
        && !crate::auth::split_csv(&domain.owners).contains(&principal.subject.to_lowercase())
    {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "detail": "owner or admin required to approve" }))).into_response();
    }
    let row = sqlx::query(
        "UPDATE meshtask SET status='ready', updated_at=$2 WHERE id=$1 AND status='pending_approval' RETURNING *",
    )
    .bind(&task_id)
    .bind(Utc::now().naive_utc())
    .fetch_optional(&state.db)
    .await;
    match row {
        Ok(Some(r)) => {
            let mission_id: String = r.get("mission_id");
            broadcast_task_available(&domain_id, &mission_id, &task_id).await;
            (StatusCode::OK, Json(row_to_task(&r))).into_response()
        }
        Ok(None) => (StatusCode::CONFLICT, Json(serde_json::json!({ "detail": "task not pending approval" }))).into_response(),
        Err(e) => {
            tracing::error!("approve_task: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

Register the route alongside the other `/work/tasks/...` routes:

```rust
.route("/work/tasks/{task_id}/approve", post(approve_task))
```

- [ ] **Step 6: Compile**

Run: `cargo check -p edgeplane-tower`
Expected: PASS.

- [ ] **Step 7: Add trust-tier integration tests** to `tests/test_authz.rs`:

```rust
#[tokio::test]
async fn sa_token_blocked_from_unlisted_template() {
    // AppConfig with one non-infra template "run-ceph-doctor".
    let (pool, ctx) = crate::common::setup().await;
    let mut cfg = AppConfig::default();
    cfg.dispatch_templates = std::sync::Arc::new(std::collections::HashMap::from([
        ("run-ceph-doctor".to_string(), edgeplane_tower::config::DispatchTemplate {
            name: "run-ceph-doctor".into(), allowed_params: vec!["target".into()], infra_grade: false,
        }),
    ]));
    let server = TestServer::new(build_app(pool, cfg)).unwrap();
    // SA is a domain contributor (so authz passes) but uses an unlisted template:
    let res = server
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header("authorization", format!("Bearer {}", ctx.member_sa_token))
        .json(&serde_json::json!({ "title": "x", "input_json": "{\"template\":\"reboot-node\"}" }))
        .await;
    assert_eq!(res.status_code(), 403);
}

#[tokio::test]
async fn session_token_free_form_allowed() {
    let (pool, ctx) = crate::common::setup().await;
    let server = TestServer::new(build_app(pool, AppConfig::default())).unwrap();
    let res = server
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header("authorization", format!("Bearer {}", ctx.owner_session_token))
        .json(&serde_json::json!({ "title": "free form" }))
        .await;
    assert_eq!(res.status_code(), 201);
}
```

- [ ] **Step 8: Regenerate drift-gated artifacts (new route)**

Run: `make docs` and regenerate `web/openapi.json` (the project's OpenAPI export step). Commit the regenerated files in this task's commit.

- [ ] **Step 9: Run + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add -A
git commit -m "feat(tower): trust-tier dispatch split + pending_approval + approve endpoint

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage (Seam 1 of the 2026-06-17 design):**
- `authorized_for_domain` helper, default deny, one shared helper reused everywhere → Tasks 2, 4, 5, 6, 7. ✔
- Corrected 0009 predicate (owners + contributors + is_admin, no `domainrolemembership`) → Task 2. ✔
- Trust-tier split (full-trust vs SA/dispatch tokens) → Task 9. ✔
- Template allowlist via TOML config → Tasks 8, 9. ✔
- Infra-grade → non-claimable `pending_approval` → Task 9. ✔
- Streams authorized + domain-scoped → Task 7. ✔
- `domain_scope` seam for Seam 2 → Task 1. ✔
- Every enumerated unguarded site (investigation 2026-06-17) is covered by Tasks 5–7.

**Placeholder scan:** No "TBD"/"add error handling"/"similar to Task N". Repetitive handler guards are given as full code per resolution group (A–D) with the per-site insertion point named — not a back-reference.

**Type consistency:** `authorized_for_domain(&Domain, &Principal)`, `authz_domain(&PgPool, &Principal, &str) -> Result<Domain, Response>`, `is_full_trust(&Principal)`, `DispatchTemplate{name, allowed_params, infra_grade}`, `domain_scope: Vec<String>` are used identically across Tasks 1–9.

**Open verification items (confirm during execution, do not assume):**
1. The `AppState` accessor for `AppConfig` (`state.config` vs other name) — Task 9 Step 3.
2. Whether `Domain` is re-exported `pub` from `crate::models` — Task 4 Step 3.
3. The exact module-declaration file for new `mod config;`/`mod authz;` (`lib.rs` vs `routes/mod.rs`).
4. Whether the test DB is available; if not, DB-backed tests are `#[ignore]`d and that is recorded (not silently skipped).

---

## Execution Handoff

Two execution options:

1. **Subagent-Driven (recommended)** — dispatch a fresh `rust-engineer` subagent per task, `rust-reviewer` between tasks, fast iteration.
2. **Inline Execution** — execute tasks in this session with checkpoints for review.
