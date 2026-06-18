# EdgePlane Seam 2 — Per-Agent Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Revised 2026-06-18 after adversarial review.** Changes from v1: the mint endpoint is re-gated to full-trust/admin only (v1 let any co-domain agent mint a peer's token = impersonation); MCP `claim_mesh_task`/`progress_mesh_task` attribution hardened (v1 only fixed REST); the auth-extractor insertion point is now unambiguous (v1's snippet risked dead-coding the agent path); `deny_unknown_fields` added to both claim structs; the daemon tasks (6–7) are redesigned around the real stateless `AgentRuntime` trait — v1 targeted "spawn functions" that are actually trait-impl `inject_task` methods, and `LaunchContext.env`/`backend_token` are never applied to spawned processes.

**Goal:** Give each enrolled agent its own short-lived, domain-scoped per-agent JWT instead of the shared operator/daemon `EP_AGENT_TOKEN`, so the tower can attribute and constrain agent actions and an agent's authority is bounded to its home domain (and, with Seam 1 Task 8, its own tasks).

**Architecture:** Clone the node-JWT pattern (`jwt.rs`) into an `AgentClaims` JWT carrying `agent_id` + `domain_id`. The tower mints one on enrollment and via a mint endpoint gated to full-trust/admin (so the daemon — a `node`, hence full-trust per Seam 1 — can mint, but a peer agent cannot). The auth extractor recognizes agent JWTs (same `EP_JWT_SIGNING_KEY`, distinguished by claim shape + `deny_unknown_fields`), checks an `agenttoken` revocation table, and returns `Principal{auth_type:"agent", domain_scope:[domain_id]}` — which Seam 1's `authorized_for` honors. The daemon mints per-agent tokens and injects each as that agent's `EP_AGENT_TOKEN` (replacing the shared token), for both ephemeral task-worker agents and supervised agents.

**Tech Stack:** Rust, axum 0.8, sqlx (Postgres), `jsonwebtoken` (RS256), `axum-test`.

**Depends on:** Seam 1 (`2026-06-18-edgeplane-seam1-authorization.md`) — `Principal.domain_scope` (S1 Task 1), `authorized_for`/`is_full_trust` (S1 Task 2), the `authz.rs` guard + resolvers (S1 Task 4), and the **test harness `tests/common`** (S1 Task 0). Land Seam 1 first; together they are the P0 release.

## Global Constraints

- **edgeplane-only — zero aria dependency.**
- **All tower HTTP paths use the `/api/` prefix.**
- **Per-task green gate:** `cargo nextest run -p <crate> --no-fail-fast` + `cargo clippy -p <crate> -- -D warnings` for every crate touched.
- **Rust toolchain pinned to 1.96.0.**
- **Migration discipline:** new `migrations/NNNN_agenttoken.sql` (next number after current max — `ls crates/edgeplane-tower/migrations/`). sqlx migrations are versioned and applied once; verification = "applies cleanly on a DB already at the prior head", NOT "re-run is a no-op". Match the existing `nodetoken` column style.
- **Datetimes UTC-aware** (`chrono::Utc`).
- **Never echo token values** beyond the single mint return; confirm presence, don't print.
- **Drift-gated artifacts:** the mint route changes the surface → run `make docs` + regenerate `web/openapi.json` in the route-adding commit.
- **Commits:** conventional-commit, end with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

### Task 1: `AgentClaims` + sign/verify (clone the node-JWT pattern)

**Files:**
- Modify: `crates/edgeplane-tower/src/jwt.rs`

**Interfaces:**
- Produces: `pub struct AgentClaims { sub, agent_id, domain_id, jti, iat, exp }`; `pub fn sign_agent_jwt(agent_id, domain_id, &EncodingKey, ttl_hours) -> anyhow::Result<(String,String)>`; `pub fn verify_agent_jwt(&str, &DecodingKey) -> anyhow::Result<AgentClaims>`.

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod agent_jwt_tests {
    use super::*;
    fn keys() -> (EncodingKey, DecodingKey) {
        let (pr, pu) = generate_rsa_keypair().unwrap();
        (encoding_key_from_pem(&pr).unwrap(), decoding_key_from_pem(&pu).unwrap())
    }
    #[test]
    fn round_trip() {
        let (e, d) = keys();
        let (t, jti) = sign_agent_jwt("w7", "dom-1", &e, 1).unwrap();
        let c = verify_agent_jwt(&t, &d).unwrap();
        assert_eq!((c.sub.as_str(), c.agent_id.as_str(), c.domain_id.as_str()), ("agent:w7", "w7", "dom-1"));
        assert_eq!(c.jti, jti);
    }
    #[test]
    fn agent_token_not_decodable_as_node() {
        let (e, d) = keys();
        let (t, _) = sign_agent_jwt("w7", "dom-1", &e, 1).unwrap();
        assert!(verify_node_jwt(&t, &d).is_err()); // NodeClaims requires node_id
    }
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement** (TTL in HOURS — agent tokens are short-lived; `#[serde(deny_unknown_fields)]` so node/agent tokens are mutually non-decodable even if fields later overlap):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentClaims {
    pub sub: String,        // "agent:{agent_id}"
    pub agent_id: String,
    pub domain_id: String,  // home domain scope
    pub jti: String,
    pub iat: i64,
    pub exp: i64,
}

pub fn sign_agent_jwt(agent_id: &str, domain_id: &str, encoding_key: &EncodingKey, ttl_hours: i64)
    -> anyhow::Result<(String, String)> {
    let now = chrono::Utc::now().timestamp();
    let jti = Uuid::new_v4().to_string();
    let claims = AgentClaims {
        sub: format!("agent:{agent_id}"), agent_id: agent_id.into(), domain_id: domain_id.into(),
        jti: jti.clone(), iat: now, exp: now + ttl_hours * 3600,
    };
    let token = encode(&Header::new(Algorithm::RS256), &claims, encoding_key)
        .map_err(|e| anyhow::anyhow!("agent JWT sign error: {e}"))?;
    Ok((token, jti))
}

pub fn verify_agent_jwt(token: &str, decoding_key: &DecodingKey) -> anyhow::Result<AgentClaims> {
    let mut v = Validation::new(Algorithm::RS256);
    v.validate_exp = true; v.leeway = 0;
    Ok(decode::<AgentClaims>(token, decoding_key, &v).map_err(|e| anyhow::anyhow!("agent JWT verify error: {e}"))?.claims)
}
```

- [ ] **Step 4: Also add `#[serde(deny_unknown_fields)]` to `NodeClaims`** (existing node tokens carry exactly those 5 fields, so unaffected; this makes an agent token explicitly fail node-decode). Run the node-JWT tests to confirm no regression.

- [ ] **Step 5: Run → PASS; clippy; commit**

```bash
cargo nextest run -p edgeplane-tower agent_jwt_tests && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/jwt.rs
git commit -m "feat(tower): AgentClaims JWT (sign/verify), deny_unknown_fields on both claims

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: `agenttoken` revocation table (migration)

**Files:**
- Create: `crates/edgeplane-tower/migrations/NNNN_agenttoken.sql`

- [ ] **Step 1: Next number** — `ls crates/edgeplane-tower/migrations/`; use highest + 1.
- [ ] **Step 2: Migration (match `nodetoken` column style — confirm it with `rg -n 'CREATE TABLE.*nodetoken' -A12 crates/edgeplane-tower/migrations/`):**

```sql
-- Per-agent JWT revocation registry (Seam 2). Mirrors nodetoken.
CREATE TABLE agenttoken (
    jti         character varying NOT NULL PRIMARY KEY,
    agent_id    character varying NOT NULL,
    domain_id   character varying NOT NULL,
    revoked     boolean NOT NULL DEFAULT false,
    expires_at  timestamp without time zone NOT NULL,
    created_at  timestamp without time zone NOT NULL DEFAULT now()
);
CREATE INDEX agenttoken_agent_id_idx ON agenttoken (agent_id);
```

- [ ] **Step 3: Verify** — apply against a DB already at the prior migration head (start the tower, or `sqlx migrate run`); confirm `agenttoken` exists and the migration is recorded in `_sqlx_migrations`. (Do NOT "re-run to confirm no-op" — sqlx applies each version once.)
- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane-tower/migrations/
git commit -m "feat(tower): agenttoken revocation table

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Recognize agent JWTs in the auth extractor

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs` (inside the `if token.matches('.').count() == 2 { … }` block, ~107–133)

**Exact placement (unambiguous):** the block today ends with a bare `return Err(AuthRejection::Unauthenticated);` (~line 132). **Replace that single line** with the agent-try-then-reject below, keeping it INSIDE the `count()==2` block and AFTER the existing `if let Ok(claims) = verify_node_jwt(...) { … }` block. A valid agent token makes `verify_node_jwt` error (different claim shape), so control reaches here.

- [ ] **Step 1: Replace the bare `return Err`**

```rust
    // Agent JWT — same RS256 key as node tokens, distinguished by claim shape.
    if let Ok(claims) = crate::jwt::verify_agent_jwt(token, &state.jwt_decoding_key) {
        let row = sqlx::query("SELECT revoked FROM agenttoken WHERE jti=$1 AND expires_at > $2")
            .bind(&claims.jti).bind(now)
            .fetch_optional(&state.db).await.ok().flatten();
        if let Some(row) = row {
            let revoked: bool = row.get("revoked");
            if !revoked {
                return Ok(Principal {
                    subject: claims.sub,
                    is_admin: false,
                    session_id: None,
                    auth_type: "agent".into(),
                    domain_scope: vec![claims.domain_id],
                });
            }
        }
    }
    // Neither a valid node nor a valid/non-revoked agent token → reject.
    return Err(AuthRejection::Unauthenticated);
```

(Fail-closed: a signed agent JWT with no `agenttoken` row, expired, or revoked falls through to reject.)

- [ ] **Step 2: Compile** — `cargo check -p edgeplane-tower` → PASS.
- [ ] **Step 3: Clippy + commit**

```bash
cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/auth.rs
git commit -m "feat(tower): accept agent JWTs (domain-scoped principal) in auth extractor

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Mint agent JWTs — at enrollment + a full-trust-gated mint endpoint

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs` (`enroll_agent` ~1741–1787; add `mint_agent_token` + route)
- Test: `crates/edgeplane-tower/tests/test_authz.rs`

**Interfaces:** `sign_agent_jwt` (T1); `agenttoken` (T2); `is_full_trust`/`authz_domain`/`domain_id_for_agent` (Seam 1); `state.jwt_encoding_key` (verified present on `AppState`).

**Mint endpoint authorization (review fix):** gate to `is_full_trust(p) || p.is_admin` — this admits sessions, admins, and **nodes** (the daemon is a node → full-trust per Seam 1), and **denies `auth_type:"agent"`**, killing the peer-impersonation hole. Then additionally `authz_domain` on the agent's domain (so a non-admin session can only mint for agents in domains it owns; a node passes via the node clause).

- [ ] **Step 1: Shared mint helper**

```rust
async fn issue_agent_token(state: &AppState, agent_id: &str, domain_id: &str) -> anyhow::Result<String> {
    const TTL_HOURS: i64 = 12;
    let (token, jti) = crate::jwt::sign_agent_jwt(agent_id, domain_id, &state.jwt_encoding_key, TTL_HOURS)?;
    let expires_at = (Utc::now() + chrono::Duration::hours(TTL_HOURS)).naive_utc();
    sqlx::query("INSERT INTO agenttoken (jti, agent_id, domain_id, revoked, expires_at, created_at) VALUES ($1,$2,$3,false,$4,$5)")
        .bind(&jti).bind(agent_id).bind(domain_id).bind(expires_at).bind(Utc::now().naive_utc())
        .execute(&state.db).await?;
    Ok(token)
}
```

- [ ] **Step 2: Return the token from `enroll_agent`** — after the `RETURNING *` insert, mint and attach. Mint failure is best-effort (the daemon re-mints via the endpoint), but log it:

```rust
Ok(r) => {
    let mut agent_json = row_to_agent(&r);
    let new_agent_id: String = r.get("id");
    match issue_agent_token(&state, &new_agent_id, &domain_id).await {
        Ok(tok) => { agent_json["agent_token"] = serde_json::Value::String(tok); }
        Err(e) => tracing::error!("enroll_agent: token mint failed for {new_agent_id}: {e}"),
    }
    // ... existing broadcast_assignment_changed ...
    (StatusCode::CREATED, Json(agent_json)).into_response()
}
```

- [ ] **Step 3: Mint endpoint**

```rust
async fn mint_agent_token(
    State(state): State<Arc<AppState>>, principal: Principal, Path(agent_id): Path<String>,
) -> impl IntoResponse {
    if !(crate::auth::is_full_trust(&principal) || principal.is_admin) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"detail":"full-trust principal required to mint agent tokens"}))).into_response();
    }
    let domain_id = match crate::routes::authz::domain_id_for_agent(&state.db, &agent_id).await {
        Ok(d) => d, Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await { return resp; }
    match issue_agent_token(&state, &agent_id, &domain_id).await {
        Ok(tok) => (StatusCode::OK, Json(serde_json::json!({"agent_token": tok, "expires_in": 12*3600}))).into_response(),
        Err(e) => { tracing::error!("mint_agent_token {agent_id}: {e}"); StatusCode::INTERNAL_SERVER_ERROR.into_response() }
    }
}
```

Register: `.route("/work/agents/{agent_id}/token", post(mint_agent_token))`.

- [ ] **Step 4: Compile** → PASS.
- [ ] **Step 5: Tests** (`tests/test_authz.rs`):

```rust
#[tokio::test]
async fn agent_cannot_mint_peer_token() {
    let Some((pool, ctx)) = common::setup().await else { return; };
    let s = server(pool);
    // owner enrolls agent A, capture its token
    let enroll = s.post(&format!("/api/work/domains/{}/agents/enroll", ctx.domain_id))
        .add_header("authorization", format!("Bearer {}", ctx.owner_session_token))
        .json(&serde_json::json!({ "runtime_kind": "claude_headless" })).await;
    let a_token = enroll.json::<serde_json::Value>()["agent_token"].as_str().unwrap().to_string();
    // A tries to mint for some other agent id in the same domain → 403 (not full-trust)
    let res = s.post("/api/work/agents/some-peer/token")
        .add_header("authorization", format!("Bearer {a_token}")).await;
    assert_eq!(res.status_code(), 403);
}

#[tokio::test]
async fn enrolled_agent_token_denied_in_foreign_domain() {
    let Some((pool, ctx)) = common::setup().await else { return; };
    let s = server(pool);
    let enroll = s.post(&format!("/api/work/domains/{}/agents/enroll", ctx.domain_id))
        .add_header("authorization", format!("Bearer {}", ctx.owner_session_token))
        .json(&serde_json::json!({ "runtime_kind": "claude_headless" })).await;
    let a_token = enroll.json::<serde_json::Value>()["agent_token"].as_str().unwrap().to_string();
    // agent token (domain_scope=[domain_id]) rejected on a foreign domain's stream
    let res = s.get(&format!("/api/work/domains/{}/stream", ctx.other_domain_id))
        .add_header("authorization", format!("Bearer {a_token}")).await;
    assert_eq!(res.status_code(), 403);
}
```

- [ ] **Step 6: Regen artifacts (new route) + run + clippy + commit**

```bash
make docs   # + regenerate web/openapi.json
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add -A
git commit -m "feat(tower): mint per-agent JWT at enrollment + full-trust-gated token endpoint

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Attribute the authenticated agent (REST + MCP)

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs` (`append_progress` ~1007, `claim_task` ~802)
- Modify: `crates/edgeplane-tower/src/routes/mcp.rs` (`claim_mesh_task` ~289, `progress_mesh_task` ~328)

**Interfaces:** `is_full_trust` (Seam 1). Helper to derive the caller's agent id: `principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject)`.

- [ ] **Step 1: `append_progress`** — replace `let agent_id = "";` with the authenticated id (promote `_principal`→`principal` if Seam 1 hasn't already):

```rust
let agent_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject).to_string();
```

- [ ] **Step 2: `claim_task`** — replace the body-`agent_id` extraction (delete the existing ~802–807 block) so restricted principals can only claim as themselves:

```rust
let self_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
let agent_id = if crate::auth::is_full_trust(&principal) || principal.is_admin {
    body.as_ref().and_then(|b| b.get("agent_id")).and_then(|v| v.as_str()).unwrap_or(self_id).to_string()
} else {
    self_id.to_string()
};
```

- [ ] **Step 3: MCP `claim_mesh_task` + `progress_mesh_task`** — same rule; non-full-trust callers' `agent_id` is forced to `self_id`, ignoring the client `agent_id` arg:

```rust
let self_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
let agent_id = if crate::auth::is_full_trust(principal) || principal.is_admin {
    str_arg(args, "agent_id")
} else { self_id.to_string() };
```

- [ ] **Step 4: Compile** → PASS.
- [ ] **Step 5: Test** — with an agent token, post progress and assert `meshprogressevent.agent_id` equals the agent's id; assert a restricted agent token cannot claim a task as a different `agent_id` (the recorded `claimed_by_agent_id` is the caller's, not the spoofed one).
- [ ] **Step 6: Run + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/src/routes/mcp.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "feat(tower): attribute claim/progress to authenticated agent (REST + MCP)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Daemon prerequisite — understand the runtime spawn lifecycle

> The review found v1's Tasks 6–7 targeted code that doesn't exist as assumed: `claude_code.rs`/`goose.rs` spawn inside `inject_task` (stateless `AgentRuntime` trait-impl methods), not free "spawn functions"; and `LaunchContext.env`/`backend_token` have **zero consumers** — the runtimes rely on env inheritance from the daemon, so naively passing an env list is silently dropped. This task grounds the redesign before editing.

**Files (read-only):**
- `crates/edgeplaned/crates/edgeplaned-runtimes/src/{types.rs, claude_code.rs, goose.rs}` (the `AgentRuntime` trait, `LaunchContext`, `launch`, `inject_task`)
- `crates/edgeplaned/crates/edgeplaned-bin/src/{daemon.rs, reconcile.rs, task_worker.rs}` (`supervisor.spawn`, `RunningAgent`, the enroll call)
- `crates/edgeplaned/crates/edgeplaned-core/src/client.rs` (`BackendClient`)

- [ ] **Step 1: Document the answers** (write findings into this task before coding):
  1. The exact `AgentRuntime` trait signature for `launch` and `inject_task`, and whether the runtime struct can hold state set during `launch`.
  2. How `inject_task` builds its `Command` and where `EP_AGENT_TOKEN` would be set (it must be an explicit `cmd.env`, since inheritance is the current mechanism).
  3. Whether `LaunchContext` already carries a usable token field (`backend_token`) and whether threading a per-agent token means a new `LaunchContext` field or a runtime-struct field.
  4. `BackendClient`'s post/get signatures and the `/work/...` path convention (no `/api` prefix, matching the existing enroll call).

- [ ] **Step 2: Choose the wiring** based on Step 1, defaulting to: **store the per-agent token on the runtime instance at `launch` (from a new `LaunchContext.agent_token: Option<String>`) and set `cmd.env("EP_AGENT_TOKEN", tok)` in `inject_task`.** No code yet — this task is the grounded design note that Tasks 7a/7b implement.

(No commit — this is investigation captured in the plan.)

---

### Task 7a: Daemon — per-agent token for ephemeral task-worker agents

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/task_worker.rs` (~367–404 enroll path + the spawn it drives)
- Modify: `crates/edgeplaned/crates/edgeplaned-runtimes/src/{types.rs, claude_code.rs, goose.rs}` per Task 6 Step 2

**Interfaces:** the `agent_token` field in the enroll response (Task 4); `LaunchContext.agent_token` (new).

- [ ] **Step 1: Capture the token** — after the `id` extraction (~396):

```rust
let agent_token = enroll_resp.get("agent_token").and_then(|v| v.as_str()).map(str::to_string);
if agent_token.is_none() {
    tracing::warn!("task_worker: enroll for task {task_id} returned no agent_token; falling back to daemon token");
}
```

- [ ] **Step 2: Thread it into the spawn** — add `agent_token: Option<String>` to `LaunchContext` (`types.rs`); set it when the task-worker builds the context for this agent; in `claude_code.rs`/`goose.rs`, persist it on the runtime instance at `launch` and, in `inject_task`, `if let Some(tok) = &self.agent_token { cmd.env("EP_AGENT_TOKEN", tok); }`.
- [ ] **Step 3: Compile** — `cargo check -p edgeplaned-runtimes -p edgeplaned-bin` → PASS.
- [ ] **Step 4: Verify (e2e — no daemon unit harness)** — dispatch a task that spawns an ephemeral agent; confirm tower-side (`meshprogressevent.agent_id`) the progress is attributed to the agent's own id, not the operator/node subject. Record the observed id.
- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p edgeplaned-runtimes -p edgeplaned-bin -- -D warnings
git add crates/edgeplaned/
git commit -m "feat(edgeplaned): inject per-agent token for ephemeral task agents

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7b: Daemon — mint + inject per-agent tokens for supervised agents

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-core/src/client.rs` (add `mint_agent_token`)
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs` (`spawn_one` / the `supervisor.spawn(...)` ~1230 path)

**Design note:** supervised agents are *fetched* (`fetch_node_agents`, GET `/runtime/nodes/{id}/agents`), not freshly enrolled, so they have no enroll-response token. The daemon mints one per agent at (re)spawn via the endpoint, using its own **node** credential — which is full-trust (Seam 1) and therefore authorized to mint (Task 4's gate). Short-lived (12h); re-mint on respawn rather than persist.

- [ ] **Step 1: `BackendClient::mint_agent_token`**

```rust
pub async fn mint_agent_token(&self, agent_id: &str) -> anyhow::Result<String> {
    let resp: serde_json::Value = self.post(&format!("/work/agents/{agent_id}/token"), &serde_json::json!({})).await?;
    resp.get("agent_token").and_then(|v| v.as_str()).map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("mint_agent_token: response missing agent_token"))
}
```

- [ ] **Step 2: Mint before spawn** — in `spawn_one`, before `supervisor.spawn(...)`:

```rust
let agent_token = match self.client.mint_agent_token(&spec.agent_id).await {
    Ok(t) => Some(t),
    Err(e) => { tracing::warn!("spawn_one: mint failed for {} ({e:#}); falling back to daemon token", spec.agent_id); None }
};
```

Thread `agent_token` into the `LaunchContext` the supervisor builds for this agent (same `LaunchContext.agent_token` field added in Task 7a). The runtime applies it in `inject_task` (already wired by Task 7a).

- [ ] **Step 3: Compile** — `cargo check -p edgeplaned-core -p edgeplaned-bin` → PASS.
- [ ] **Step 4: Verify (e2e)** — restart the daemon; confirm a supervised agent's tower actions are attributed to `agent:{public_id}`; then `UPDATE agenttoken SET revoked=true WHERE agent_id='<id>'` and confirm its next tower call 401s within the token lifetime. Record both.
- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p edgeplaned-core -p edgeplaned-bin -- -D warnings
git add crates/edgeplaned/
git commit -m "feat(edgeplaned): mint + inject per-agent tokens for supervised agents

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Explicitly out of scope (follow-up, not P0)

- **`edgeplane agent run` profile path** (`crates/edgeplane/src/agent_harness.rs:1071`): the systemd profile agents (aria-operator/engineer/…) still inject `config.token` (the operator's per-profile session token). They are the operator's own delegates and act as the operator legitimately; migrating them to per-profile scoped credentials is a follow-up. Flagged, not silently left.
- **Workspace-lease identity migration:** a workspace loaded under the old shared token records `actor_subject` = operator; after Seam 2 a per-agent identity can't commit it. Track as a one-time concern when cutting over live workspaces.
- nftables egress + cgroup enforcement — Seam 3.

## Self-Review

**Spec coverage (Seam 2, post-revision):**
- Per-agent JWT, domain-scoped, node-JWT pattern → T1, T3. ✔ (§9 decision: per-agent JWT, not SA extension.)
- Minted at enrollment + endpoint, **gated so peers can't mint** → T4 (review fix). ✔
- `claim`/`append_progress` record the authenticated agent, **REST and MCP** → T5 (review fix). ✔
- Agent in A can't act in B → `domain_scope` single-entry + `authorized_for` (Seam 1); tested T4. ✔
- Replaces shared `EP_AGENT_TOKEN` for enrolled agents → T7a (ephemeral), T7b (supervised), grounded by T6. ✔
- Revocation parity → T2 + T3. ✔

**Placeholder scan:** the daemon wiring (T6/7a/7b) is no longer hand-waved — T6 forces the trait/lifecycle to be read and the wiring decided before T7a/7b edit, with the concrete `cmd.env` and `LaunchContext.agent_token` approach named.

**Type consistency:** `AgentClaims{sub,agent_id,domain_id,jti,iat,exp}`, `sign_agent_jwt(...)->(String,String)`, `verify_agent_jwt(...)->AgentClaims`, `issue_agent_token(&AppState,&str,&str)->Result<String>`, tower `mint_agent_token` ↔ `BackendClient::mint_agent_token`, `LaunchContext.agent_token: Option<String>`, `auth_type:"agent"` + `domain_scope:vec![domain_id]` all line up with Seam 1's `authorized_for`/`is_full_trust`.

**Open verification items (confirm at execution):**
1. `state.jwt_encoding_key` reachable from handlers (verified present on `AppState`).
2. `AgentRuntime` trait + `LaunchContext` lifecycle (Task 6 — gates T7a/7b).
3. `BackendClient` `/work/...` path convention (no `/api`) — mirror the enroll call.
4. Next migration number (T2).

## Execution Handoff

1. **Subagent-Driven (recommended)** — fresh `rust-engineer` per task, `rust-reviewer` between tasks; review T6→T7b carefully (cross-crate).
2. **Inline Execution** — checkpoints.

**Land Seam 1 first** (it provides the harness, the guard, `is_full_trust`, and node full-trust this plan depends on), then this plan; the P0 release ships after both are green.
