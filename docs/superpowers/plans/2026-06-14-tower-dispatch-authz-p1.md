# Tower Dispatch & Ledger Authorization (P1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce domain-scoped authorization on `create_task`, `submit_mesh_task`, `domain_stream`, and `mission_stream` — closing the live RCE-class gap where any authenticated principal can dispatch work to any domain's agents or read any domain's ledger.

**Architecture:** Add a shared `authz` module that reuses the tower's *already-canonical* membership idiom (`domain.owners`/`contributors` CSV + `domainrolemembership` table + `Principal.is_admin`) — the exact check `docs.rs`/`artifacts.rs`/`missions.rs` already use, applied to the four endpoints that currently skip it. Ship behind an `EP_AUTHZ_ENFORCE` env flag defaulting to **warn** (log would-be-denials, allow) so a caller audit confirms no legitimate caller breaks before flipping to **enforce** (403).

**Tech Stack:** Rust, axum 0.8, sqlx/Postgres, cargo nextest. Project: **edgeplane only** (`edgeplane-tower` crate). Zero aria dependency — if aria-rs did not exist, every change here compiles and runs identically.

**Spec:** `edgeplane/docs/superpowers/specs/2026-06-14-tower-dispatch-authz-hardening-design.md` (P1 scope).

---

## File structure

- **Create** `crates/edgeplane-tower/src/authz.rs` — `enforce()` (reads env), pure `decide()` (unit-tested), async `domain_writable()` (DB-backed). One responsibility: "may this principal act in this domain?"
- **Modify** crate root (`lib.rs` or `main.rs`) — declare `mod authz;`.
- **Modify** `crates/edgeplane-tower/src/routes/work.rs` — gate `create_task`; add `principal: Principal` + gate to `domain_stream` and `mission_stream`.
- **Modify** `crates/edgeplane-tower/src/routes/mcp.rs` — gate `submit_mesh_task`.
- **Test** unit tests for `decide()` live in `authz.rs` (`#[cfg(test)]`).

Why a pure `decide()` + thin async wrapper: the decision logic is unit-testable with no DB; the wrapper just supplies the three DB inputs. This keeps the testable core isolated.

---

## Task 0: Caller audit (gate for enforce — Task 6)

**No code.** Enumerate every current caller of the four endpoints and confirm each will pass `domain_writable` for the domain it uses, so flipping to enforce (Task 6) breaks nothing. The warn-mode logs (Tasks 1–5 deployed) are the *authoritative* audit; this static pass front-loads the known callers.

- [ ] **Step 1: List known callers**

Run:
```bash
# HA plugin dispatch + ledger consumer
rg -n "create_task|submit_mesh_task|/stream|domains/.*/stream|missions/.*/stream" /home/merlin/code/edgeplane-homeassistant
# Web UI
rg -n "create_task|submit_mesh_task|/stream" /home/merlin/code/edgeplane/web/src
# Agents using the MCP tool (incl. aria-the-agent as a *client*, which is allowed — aria calls edgeplane, never the reverse)
rg -n "submit_mesh_task" /home/merlin/code/edgeplane
```

- [ ] **Step 2: For each distinct caller, record principal subject + domain + auth_type, and confirm membership**

For each subject `S` and domain `D` found, verify `S` is an owner/contributor or has a `domainrolemembership` row:
```bash
# Example: confirm a subject is authorized for aria-merlinlabs (192428ad01c3)
psql "$EP_DATABASE_URL" -c "SELECT id, owners, contributors FROM domain WHERE id='192428ad01c3';"
psql "$EP_DATABASE_URL" -c "SELECT subject, role FROM domainrolemembership WHERE domain_id='192428ad01c3';"
```
Record a table (caller → subject → domain → passes? Y/N) in the PR description. Any "N" is a caller that warn-mode must show as benign or that needs a `domainrolemembership` row added **before** Task 6.

- [ ] **Step 3: Commit the audit notes**

```bash
git add docs/plans/2026-06-14-tower-dispatch-authz-p1.md
git commit -m "docs(authz): caller audit for dispatch/ledger authz P1"
```

---

## Task 1: The `authz` module — pure decision + unit tests (TDD)

**Files:**
- Create: `crates/edgeplane-tower/src/authz.rs`
- Modify: crate root module declaration (find with `rg -n "^(pub )?mod auth;" crates/edgeplane-tower/src/*.rs`)

- [ ] **Step 1: Write the failing test (create `authz.rs` with only `decide` + tests)**

```rust
// crates/edgeplane-tower/src/authz.rs

/// Split a comma-separated owners/contributors string into lowercased, trimmed,
/// non-empty entries. Mirrors the existing `split_csv` used across routes.
fn split_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_lowercase())
        .filter(|x| !x.is_empty())
        .collect()
}

/// Pure authorization decision. `subject_lower` must already be lowercased.
/// `member_row` is whether a `domainrolemembership` row exists for (domain, subject).
pub fn decide(
    is_admin: bool,
    owners_csv: &str,
    contributors_csv: &str,
    member_row: bool,
    subject_lower: &str,
) -> bool {
    if is_admin || member_row {
        return true;
    }
    let s = subject_lower.to_string();
    split_csv(owners_csv).contains(&s) || split_csv(contributors_csv).contains(&s)
}

#[cfg(test)]
mod tests {
    use super::decide;

    #[test]
    fn admin_always_allowed() {
        assert!(decide(true, "", "", false, "anyone"));
    }
    #[test]
    fn owner_allowed() {
        assert!(decide(false, "alice,bob", "", false, "bob"));
    }
    #[test]
    fn contributor_allowed() {
        assert!(decide(false, "alice", "carol", false, "carol"));
    }
    #[test]
    fn member_row_allowed() {
        assert!(decide(false, "", "", true, "dave"));
    }
    #[test]
    fn stranger_denied() {
        assert!(!decide(false, "alice,bob", "carol", false, "mallory"));
    }
    #[test]
    fn case_insensitive_owner() {
        assert!(decide(false, "Alice,BOB", "", false, "bob"));
    }
    #[test]
    fn unknown_domain_empty_csv_denies_non_member() {
        assert!(!decide(false, "", "", false, "bob"));
    }
}
```

- [ ] **Step 2: Declare the module at the crate root**

Run `rg -n "^(pub )?mod auth;" crates/edgeplane-tower/src/*.rs` to find where `auth` is declared (e.g. `main.rs` or `lib.rs`). Add directly beneath it:
```rust
mod authz;
```

- [ ] **Step 3: Run the tests — verify they pass**

Run: `cargo nextest run -p edgeplane-tower authz::`
Expected: 7 tests PASS. (If the crate doesn't compile yet because `authz` is unused, that's fine — the `decide` fn + tests reference each other; the unused-`split_csv`-in-non-test warning is acceptable until Task 2 adds the async user.)

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane-tower/src/authz.rs crates/edgeplane-tower/src/*.rs
git commit -m "feat(authz): pure domain-authorization decision + unit tests"
```

---

## Task 2: Async DB-backed `domain_writable` + `enforce()` flag

**Files:**
- Modify: `crates/edgeplane-tower/src/authz.rs`

- [ ] **Step 1: Add the env flag + async wrapper**

Append to `authz.rs` (above the `#[cfg(test)]` module):
```rust
use crate::auth::Principal;
use sqlx::{PgPool, Row};

/// Whether authorization failures return 403 (enforce) or only log (warn).
/// Default = warn (false). Set `EP_AUTHZ_ENFORCE=1` (or `true`) to enforce.
pub fn enforce() -> bool {
    matches!(
        std::env::var("EP_AUTHZ_ENFORCE").ok().as_deref(),
        Some("1") | Some("true")
    )
}

/// DB-backed: may `principal` dispatch/read within `domain_id`?
/// Unknown domain → Ok(false) (deny). DB errors propagate.
pub async fn domain_writable(
    db: &PgPool,
    principal: &Principal,
    domain_id: &str,
) -> sqlx::Result<bool> {
    if principal.is_admin {
        return Ok(true);
    }
    let subject_lower = principal.subject.to_lowercase();

    let row = sqlx::query("SELECT owners, contributors FROM domain WHERE id=$1")
        .bind(domain_id)
        .fetch_optional(db)
        .await?;
    let (owners, contributors) = match row {
        Some(r) => (
            r.get::<String, _>("owners"),
            r.get::<String, _>("contributors"),
        ),
        None => return Ok(false),
    };

    let member: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM domainrolemembership \
         WHERE domain_id=$1 AND lower(subject)=$2 AND role IN ('owner','contributor'))",
    )
    .bind(domain_id)
    .bind(&subject_lower)
    .fetch_one(db)
    .await?;

    Ok(decide(false, &owners, &contributors, member, &subject_lower))
}
```

- [ ] **Step 2: Verify the crate compiles**

Run: `cargo check -p edgeplane-tower`
Expected: compiles (warnings about `domain_writable`/`enforce` being unused are fine until Tasks 3–5 call them).

- [ ] **Step 3: Commit**

```bash
git add crates/edgeplane-tower/src/authz.rs
git commit -m "feat(authz): DB-backed domain_writable + EP_AUTHZ_ENFORCE flag"
```

---

## Task 3: Gate `create_task` (work.rs)

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:547` (immediately after `domain_id` is resolved)

- [ ] **Step 1: Insert the gate**

In `create_task`, the block ending at line 547 resolves `domain_id`. Immediately **after** that `let domain_id = match … };` block, insert:
```rust
    // Domain-scoped authorization (warn-then-enforce; see authz.rs).
    match crate::authz::domain_writable(&state.db, &principal, &domain_id).await {
        Ok(true) => {}
        Ok(false) => {
            if crate::authz::enforce() {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"detail": "not authorized for this domain"})),
                )
                    .into_response();
            }
            tracing::warn!(
                target: "authz",
                subject = %principal.subject,
                domain = %domain_id,
                endpoint = "create_task",
                "AUTHZ would-deny (warn mode)"
            );
        }
        Err(e) => {
            tracing::error!("authz create_task: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
```
(`StatusCode`, `Json`, `IntoResponse` are already imported in `work.rs`.)

- [ ] **Step 2: Compile check**

Run: `cargo check -p edgeplane-tower`
Expected: compiles.

- [ ] **Step 3: Caller-phase exercise (warn mode = default)**

Start the tower locally (or against the dev DB), then with a token whose subject is NOT a member of the target domain:
```bash
curl -sS -X POST "http://localhost:8008/work/missions/<mission_id>/tasks" \
  -H "Authorization: Bearer $NON_MEMBER_TOKEN" -H "Content-Type: application/json" \
  -d '{"title":"authz-probe","description":"{}"}' -o /dev/null -w "%{http_code}\n"
```
Expected (warn mode): `201` AND a `AUTHZ would-deny (warn mode)` line in the tower log. Confirms detection without breaking callers.

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs
git commit -m "feat(authz): gate create_task on domain membership (warn mode)"
```

---

## Task 4: Gate `submit_mesh_task` (mcp.rs)

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs:217` (after `domain_id` is validated non-empty)

- [ ] **Step 1: Insert the gate**

In the `"submit_mesh_task"` arm, the required-field check ends at line 217 (`return err_result(...)`). Immediately **after** that `if … { return err_result(...); }`, insert (note `principal` here is `&Principal` from `dispatch`):
```rust
            match crate::authz::domain_writable(&state.db, principal, &domain_id).await {
                Ok(true) => {}
                Ok(false) => {
                    if crate::authz::enforce() {
                        return err_result("not authorized for this domain");
                    }
                    tracing::warn!(
                        target: "authz",
                        subject = %principal.subject,
                        domain = %domain_id,
                        endpoint = "submit_mesh_task",
                        "AUTHZ would-deny (warn mode)"
                    );
                }
                Err(e) => {
                    tracing::error!("authz submit_mesh_task: {e}");
                    return err_result("database_error");
                }
            }
```

- [ ] **Step 2: Compile check**

Run: `cargo check -p edgeplane-tower`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add crates/edgeplane-tower/src/routes/mcp.rs
git commit -m "feat(authz): gate submit_mesh_task on domain membership (warn mode)"
```

---

## Task 5: Gate the ledger streams (work.rs)

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs:2326-2340` (`mission_stream`, `domain_stream`)

- [ ] **Step 1: Replace `domain_stream` with a gated version**

Replace the existing `domain_stream` (lines 2334-2340) with:
```rust
async fn domain_stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(domain_id): Path<String>,
) -> impl IntoResponse {
    match crate::authz::domain_writable(&state.db, &principal, &domain_id).await {
        Ok(true) => {}
        Ok(false) => {
            if crate::authz::enforce() {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"detail": "not authorized for this domain"})),
                )
                    .into_response();
            }
            tracing::warn!(target: "authz", subject = %principal.subject, domain = %domain_id, endpoint = "domain_stream", "AUTHZ would-deny (warn mode)");
        }
        Err(e) => {
            tracing::error!("authz domain_stream: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    ws.on_upgrade(move |socket| poll_ledger_stream(socket, state, "domain_id".into(), domain_id))
        .into_response()
}
```

- [ ] **Step 2: Replace `mission_stream` with a domain-resolving, gated version**

Replace the existing `mission_stream` (lines 2326-2332) with:
```rust
async fn mission_stream(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(mission_id): Path<String>,
) -> impl IntoResponse {
    let domain_id = match sqlx::query("SELECT domain_id FROM mission WHERE id=$1")
        .bind(&mission_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => r.get::<Option<String>, _>("domain_id").unwrap_or_default(),
        Ok(None) => return not_found("Mission not found"),
        Err(e) => {
            tracing::error!("mission_stream fetch domain: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    match crate::authz::domain_writable(&state.db, &principal, &domain_id).await {
        Ok(true) => {}
        Ok(false) => {
            if crate::authz::enforce() {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"detail": "not authorized for this domain"})),
                )
                    .into_response();
            }
            tracing::warn!(target: "authz", subject = %principal.subject, domain = %domain_id, endpoint = "mission_stream", "AUTHZ would-deny (warn mode)");
        }
        Err(e) => {
            tracing::error!("authz mission_stream: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    ws.on_upgrade(move |socket| poll_ledger_stream(socket, state, "mission_id".into(), mission_id))
        .into_response()
}
```
(`not_found` is defined at `work.rs:117`; `Principal` is already imported in `work.rs`.)

- [ ] **Step 3: Compile check**

Run: `cargo check -p edgeplane-tower`
Expected: compiles. If axum complains about extractor ordering, ensure `ws: WebSocketUpgrade` is first and `Path` last (both are `FromRequestParts`, so order is otherwise flexible).

- [ ] **Step 4: Caller-phase exercise**

```bash
# Non-member token, warn mode → upgrade still succeeds but logs would-deny
websocat "ws://localhost:8008/work/domains/<domain_id>/stream" \
  -H "Authorization: Bearer $NON_MEMBER_TOKEN" --ping-interval 5 &
# Expect: connection opens (warn mode) + "AUTHZ would-deny ... endpoint=domain_stream" in tower log
```

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplane-tower/src/routes/work.rs
git commit -m "feat(authz): gate domain/mission ledger streams on membership (warn mode)"
```

---

## Task 6: Flip to enforce (gated on Task 0 + clean warn logs)

**Precondition:** the static caller audit (Task 0) shows every legitimate caller passes, AND the deployed warn-mode logs show no `AUTHZ would-deny` lines for legitimate callers over the observation window. Do NOT do this task until both hold.

**Files:**
- Modify: the tower deployment manifest in the gitops repo (set `EP_AUTHZ_ENFORCE=1` in the tower Deployment env). The flag default stays `warn` in code so a rollback is an env change, not a code revert.

- [ ] **Step 1: Add the env var to the tower Deployment**

In the gitops tower Deployment env block, add:
```yaml
- name: EP_AUTHZ_ENFORCE
  value: "1"
```

- [ ] **Step 2: Deploy and verify enforce**

After rollout, repeat the Task 3 probe with a non-member token:
```bash
curl -sS -X POST "https://<tower>/work/missions/<mission_id>/tasks" \
  -H "Authorization: Bearer $NON_MEMBER_TOKEN" -H "Content-Type: application/json" \
  -d '{"title":"authz-probe","description":"{}"}' -o /dev/null -w "%{http_code}\n"
```
Expected: `403`. And a member token still returns `201`.

- [ ] **Step 3: Verify legitimate callers unaffected**

Confirm the HA plugin (`binary_sensor.edgeplane_agent_online` stays on) and any agent `submit_mesh_task` calls still succeed. Watch tower logs for unexpected `403`s.

- [ ] **Step 4: Commit (gitops repo)**

```bash
git -C <gitops-repo> add <tower-deployment.yaml>
git -C <gitops-repo> commit -m "feat(authz): enable EP_AUTHZ_ENFORCE on tower"
```

---

## Self-Review

**Spec coverage (P1 section of the design):**
- "Domain-scoped authz on create_task" → Task 3 ✓
- "…submit_mesh_task" → Task 4 ✓
- "…domain_stream / mission_stream (thread Principal, authorize before upgrade)" → Task 5 ✓
- "default deny; reuse existing owner/`is_admin` pattern + `domainrolemembership`" → Tasks 1–2 (`decide` + `domain_writable`) ✓
- "warn-then-enforce rollout + caller audit" (the concern raised pre-plan) → Tasks 0 + 6 ✓
- **Out of P1 scope, intentionally deferred:** dispatchable-template allowlist, infra-grade confirm-at-creation, plugin capability fail-closed, SA least-privilege → follow-on P2/P3 plans (noted in spec).

**Placeholder scan:** none — every step has exact code/commands/paths.

**Type consistency:** `decide(is_admin, owners_csv, contributors_csv, member_row, subject_lower)` is defined in Task 1 and called by `domain_writable` in Task 2 with the same arity/order; `domain_writable(db, principal, domain_id)` is defined in Task 2 and called identically in Tasks 3/4/5; `enforce()` is defined in Task 2 and called in Tasks 3/4/5. `principal` is `&Principal` in mcp.rs (passed as `principal`) and owned `Principal` in work.rs handlers (passed as `&principal`).

---

## Separation note

Every change is in `edgeplane-tower` (+ the gitops tower Deployment env for the flip). No aria-rs reference, import, or runtime dependency. Postgres is shared infrastructure. **If aria did not exist, this plan compiles, deploys, and enforces identically.**
