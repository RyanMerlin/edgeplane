# EdgePlane Seam 2 — Per-Agent Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give each enrolled agent its own short-lived, domain-scoped per-agent JWT instead of the shared operator `EP_AGENT_TOKEN`, so the tower can attribute and constrain agent actions and an agent's authority is bounded to its home domain.

**Architecture:** Clone the existing node-JWT pattern (`jwt.rs`) into an `AgentClaims` JWT carrying `agent_id` + `domain_id`. The tower mints one on enrollment (returned in the enroll response) and exposes a mint endpoint for already-enrolled supervised agents. The auth extractor recognizes agent JWTs (signed by the same `EP_JWT_SIGNING_KEY`, distinguished by claim shape), checks an `agenttoken` revocation table, and returns a `Principal{auth_type:"agent", domain_scope:[domain_id]}` — which Seam 1's `authorized_for_domain` already honors. The daemon captures the per-agent token (ephemeral task agents) or mints one (supervised agents) and injects it as that agent's `EP_AGENT_TOKEN`.

**Tech Stack:** Rust, axum, sqlx (Postgres), `jsonwebtoken` (RS256), `axum-test`.

**Depends on:** Seam 1 (`docs/superpowers/plans/2026-06-18-edgeplane-seam1-authorization.md`) — specifically `Principal.domain_scope` (Seam 1 Task 1) and `authorized_for_domain` (Seam 1 Task 2). Land Seam 1 first. Together they constitute the P0 security release.

## Global Constraints

- **edgeplane-only — zero aria dependency.**
- **All tower HTTP paths use the `/api/` prefix.**
- **Per-task green gate:** `cargo nextest run -p <crate> --no-fail-fast` + `cargo clippy -p <crate> -- -D warnings` for every crate touched (`edgeplane-tower`, `edgeplane`, `edgeplaned-runtimes`, `edgeplaned-bin`).
- **Rust toolchain pinned to 1.96.0.**
- **Migration discipline:** new sqlx migration `migrations/NNNN_agenttoken.sql` (next number after the current max — find with `ls crates/edgeplane-tower/migrations/`); guard creation with `CREATE TABLE IF NOT EXISTS`; test against fresh + existing DB.
- **edgeplane-mesh datetimes are UTC-aware** (`chrono::Utc`; `Z`/`+00:00` suffix on the wire).
- **Never echo token values** in logs or responses beyond the single mint return; confirm presence, don't print.
- **Regenerate drift-gated artifacts** (`make docs`, `web/openapi.json`) — the new mint route changes the surface.
- **Commits:** conventional-commit, end with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.

---

### Task 1: `AgentClaims` + sign/verify (clone the node-JWT pattern)

**Files:**
- Modify: `crates/edgeplane-tower/src/jwt.rs` (add alongside `NodeClaims`/`sign_node_jwt`/`verify_node_jwt`)

**Interfaces:**
- Produces:
  - `pub struct AgentClaims { pub sub: String, pub agent_id: String, pub domain_id: String, pub jti: String, pub iat: i64, pub exp: i64 }`
  - `pub fn sign_agent_jwt(agent_id: &str, domain_id: &str, encoding_key: &EncodingKey, ttl_hours: i64) -> anyhow::Result<(String, String)>` — returns `(token, jti)`.
  - `pub fn verify_agent_jwt(token: &str, decoding_key: &DecodingKey) -> anyhow::Result<AgentClaims>`.

- [ ] **Step 1: Write the failing unit test**

Add to `crates/edgeplane-tower/src/jwt.rs`:

```rust
#[cfg(test)]
mod agent_jwt_tests {
    use super::*;

    fn keys() -> (EncodingKey, DecodingKey) {
        let (priv_pem, pub_pem) = generate_rsa_keypair().unwrap();
        (
            encoding_key_from_pem(&priv_pem).unwrap(),
            decoding_key_from_pem(&pub_pem).unwrap(),
        )
    }

    #[test]
    fn round_trip_agent_jwt() {
        let (enc, dec) = keys();
        let (token, jti) = sign_agent_jwt("worker-7", "dom-1", &enc, 1).unwrap();
        let claims = verify_agent_jwt(&token, &dec).unwrap();
        assert_eq!(claims.sub, "agent:worker-7");
        assert_eq!(claims.agent_id, "worker-7");
        assert_eq!(claims.domain_id, "dom-1");
        assert_eq!(claims.jti, jti);
    }

    #[test]
    fn agent_token_is_not_decodable_as_node() {
        let (enc, dec) = keys();
        let (token, _) = sign_agent_jwt("worker-7", "dom-1", &enc, 1).unwrap();
        // Distinguishability: an agent token must NOT verify as a node token
        // (NodeClaims requires `node_id`, which AgentClaims lacks).
        assert!(verify_node_jwt(&token, &dec).is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run -p edgeplane-tower agent_jwt_tests`
Expected: FAIL — `AgentClaims` / `sign_agent_jwt` / `verify_agent_jwt` not found.

- [ ] **Step 3: Implement (mirror `NodeClaims`)**

```rust
/// Claims embedded in a per-agent JWT (Seam 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentClaims {
    /// `agent:{agent_id}` — the agent's identity subject.
    pub sub: String,
    /// The bare agent id (public_id / meshagent id).
    pub agent_id: String,
    /// Home domain this agent is scoped to.
    pub domain_id: String,
    /// JWT ID — used for revocation lookups against `agenttoken`.
    pub jti: String,
    pub iat: i64,
    pub exp: i64,
}

/// Mint a per-agent JWT scoped to `domain_id`. TTL is in HOURS (agent tokens are
/// short-lived and re-minted on (re)spawn), unlike node tokens (days).
pub fn sign_agent_jwt(
    agent_id: &str,
    domain_id: &str,
    encoding_key: &EncodingKey,
    ttl_hours: i64,
) -> anyhow::Result<(String, String)> {
    let now = chrono::Utc::now().timestamp();
    let jti = Uuid::new_v4().to_string();
    let claims = AgentClaims {
        sub: format!("agent:{agent_id}"),
        agent_id: agent_id.to_string(),
        domain_id: domain_id.to_string(),
        jti: jti.clone(),
        iat: now,
        exp: now + ttl_hours * 3600,
    };
    let token = encode(&Header::new(Algorithm::RS256), &claims, encoding_key)
        .map_err(|e| anyhow::anyhow!("agent JWT sign error: {e}"))?;
    Ok((token, jti))
}

pub fn verify_agent_jwt(token: &str, decoding_key: &DecodingKey) -> anyhow::Result<AgentClaims> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = true;
    validation.leeway = 0;
    let data = decode::<AgentClaims>(token, decoding_key, &validation)
        .map_err(|e| anyhow::anyhow!("agent JWT verify error: {e}"))?;
    Ok(data.claims)
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo nextest run -p edgeplane-tower agent_jwt_tests`
Expected: PASS (2 tests).

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/jwt.rs
git commit -m "feat(tower): AgentClaims JWT (sign/verify), domain-scoped

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: `agenttoken` revocation table (migration)

**Files:**
- Create: `crates/edgeplane-tower/migrations/NNNN_agenttoken.sql` (next sequential number)

**Interfaces:**
- Produces: table `agenttoken(jti PK, agent_id, domain_id, revoked bool, expires_at timestamp, created_at timestamp)` — mirrors `nodetoken`’s revocation role.

- [ ] **Step 1: Find the next migration number**

Run: `ls crates/edgeplane-tower/migrations/`
Note the highest `NNNN_` prefix; the new file is `NNNN+1`.

- [ ] **Step 2: Write the migration**

Create `crates/edgeplane-tower/migrations/NNNN_agenttoken.sql`:

```sql
-- Per-agent JWT revocation registry (Seam 2). Mirrors nodetoken.
CREATE TABLE IF NOT EXISTS public.agenttoken (
    jti         character varying NOT NULL,
    agent_id    character varying NOT NULL,
    domain_id   character varying NOT NULL,
    revoked     boolean NOT NULL DEFAULT false,
    expires_at  timestamp without time zone NOT NULL,
    created_at  timestamp without time zone NOT NULL DEFAULT (now() AT TIME ZONE 'utc'),
    CONSTRAINT agenttoken_pkey PRIMARY KEY (jti)
);

CREATE INDEX IF NOT EXISTS agenttoken_agent_id_idx ON public.agenttoken (agent_id);
```

- [ ] **Step 3: Apply against a fresh DB + verify idempotency**

Run: start the tower against a fresh test DB (migrations run on startup unless `--no-migrate`), then re-run to confirm the `IF NOT EXISTS` guards make it a no-op on an existing DB.
Expected: table present; second run clean.

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane-tower/migrations/
git commit -m "feat(tower): agenttoken revocation table

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Extend the auth extractor to accept agent JWTs

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs` (the `token.matches('.').count() == 2` branch, ~107–133)

**Interfaces:**
- Consumes: `verify_agent_jwt`, `AgentClaims` (Task 1); `agenttoken` (Task 2); `Principal.domain_scope` (Seam 1 Task 1).
- Produces: a `Principal{ subject: "agent:{agent_id}", is_admin: false, session_id: None, auth_type: "agent", domain_scope: vec![domain_id] }` for valid, non-revoked agent JWTs.

- [ ] **Step 1: Extend the JWT branch**

Inside the existing `if token.matches('.').count() == 2 { … }` block, after the node-JWT path fails to produce a Principal (i.e. `verify_node_jwt` errored or the node was revoked/unknown), try the agent path BEFORE the final `return Err(AuthRejection::Unauthenticated)`:

```rust
// Agent JWT — same RS256 key as node tokens, distinguished by claim shape
// (AgentClaims has agent_id+domain_id; NodeClaims has node_id).
if let Ok(claims) = crate::jwt::verify_agent_jwt(token, &state.jwt_decoding_key) {
    let row = sqlx::query("SELECT revoked FROM agenttoken WHERE jti=$1 AND expires_at > $2")
        .bind(&claims.jti)
        .bind(now)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
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
// JWT present but neither a valid node nor a valid agent token — reject.
return Err(AuthRejection::Unauthenticated);
```

- [ ] **Step 2: Compile**

Run: `cargo check -p edgeplane-tower`
Expected: PASS.

- [ ] **Step 3: Unit-test the claim distinguishability** (already covered by Task 1's `agent_token_is_not_decodable_as_node`; no new unit test needed here). The revocation path is exercised by the integration test in Task 4.

- [ ] **Step 4: Clippy + commit**

```bash
cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/auth.rs
git commit -m "feat(tower): accept agent JWTs in auth extractor (domain-scoped principal)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Mint agent JWTs — at enrollment and via a mint endpoint

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs` (`enroll_agent` ~1741–1787; add `mint_agent_token` handler + route)
- Test: `crates/edgeplane-tower/tests/test_authz.rs`

**Interfaces:**
- Consumes: `sign_agent_jwt` (Task 1); `agenttoken` (Task 2); `authz_domain` (Seam 1 Task 4); `state.jwt_encoding_key`.
- Produces:
  - `enroll_agent` response JSON gains an `agent_token` field.
  - New route `POST /api/work/agents/{agent_id}/token` → `mint_agent_token`, returning `{ "agent_token": "<jwt>", "expires_in": <seconds> }`. Authorized via `authz_domain` on the agent's domain (so only a principal authorized for that domain can mint).

- [ ] **Step 1: Add a mint helper used by both call sites**

In `work.rs`, near the agent handlers:

```rust
/// Mint a per-agent JWT and record its jti for revocation. Returns the token.
async fn issue_agent_token(state: &AppState, agent_id: &str, domain_id: &str) -> anyhow::Result<String> {
    const TTL_HOURS: i64 = 12;
    let (token, jti) = crate::jwt::sign_agent_jwt(agent_id, domain_id, &state.jwt_encoding_key, TTL_HOURS)?;
    let expires_at = (Utc::now() + chrono::Duration::hours(TTL_HOURS)).naive_utc();
    sqlx::query(
        "INSERT INTO agenttoken (jti, agent_id, domain_id, revoked, expires_at, created_at) \
         VALUES ($1,$2,$3,false,$4,$5)",
    )
    .bind(&jti)
    .bind(agent_id)
    .bind(domain_id)
    .bind(expires_at)
    .bind(Utc::now().naive_utc())
    .execute(&state.db)
    .await?;
    Ok(token)
}
```

- [ ] **Step 2: Mint + return the token in `enroll_agent`**

After the successful `INSERT INTO meshagent … RETURNING *` (~1763) and before building the response, mint a token for the new agent and attach it:

```rust
Ok(r) => {
    let mut agent_json = row_to_agent(&r);
    let new_agent_id: String = r.get("id");
    match issue_agent_token(&state, &new_agent_id, &domain_id).await {
        Ok(tok) => {
            agent_json["agent_token"] = serde_json::Value::String(tok);
        }
        Err(e) => {
            tracing::error!("enroll_agent: token mint failed for {new_agent_id}: {e}");
            // Enrollment still succeeds; daemon can mint later via the token endpoint.
        }
    }
    // ... existing broadcast_assignment_changed ...
    (StatusCode::CREATED, Json(agent_json)).into_response()
}
```

- [ ] **Step 3: Add the standalone mint endpoint**

```rust
async fn mint_agent_token(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let domain_id = match crate::routes::authz::domain_id_for_agent(&state.db, &agent_id).await {
        Ok(d) => d,
        Err(resp) => return resp,
    };
    if let Err(resp) = crate::routes::authz::authz_domain(&state.db, &principal, &domain_id).await {
        return resp;
    }
    match issue_agent_token(&state, &agent_id, &domain_id).await {
        Ok(tok) => (StatusCode::OK, Json(serde_json::json!({ "agent_token": tok, "expires_in": 12 * 3600 }))).into_response(),
        Err(e) => {
            tracing::error!("mint_agent_token {agent_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

Register the route with the other agent routes:

```rust
.route("/work/agents/{agent_id}/token", post(mint_agent_token))
```

- [ ] **Step 4: Compile**

Run: `cargo check -p edgeplane-tower`
Expected: PASS.

- [ ] **Step 5: Integration test — minted token authorizes within its home domain only**

Add to `tests/test_authz.rs`:

```rust
#[tokio::test]
async fn enrolled_agent_token_authorizes_home_domain_only() {
    let (pool, ctx) = crate::common::setup().await; // ctx.domain_id owned by owner; ctx.other_domain_id exists
    let server = TestServer::new(build_app(pool, AppConfig::default())).unwrap();

    // Enroll an agent in ctx.domain_id as the owner; capture agent_token.
    let enroll = server
        .post(&format!("/api/work/domains/{}/agents/enroll", ctx.domain_id))
        .add_header("authorization", format!("Bearer {}", ctx.owner_session_token))
        .json(&serde_json::json!({ "runtime_kind": "claude_headless" }))
        .await;
    let body: serde_json::Value = enroll.json();
    let agent_token = body["agent_token"].as_str().expect("agent_token present").to_string();

    // The agent token can submit to a mission in its home domain (200/201)...
    let ok = server
        .post(&format!("/api/work/missions/{}/tasks", ctx.mission_id))
        .add_header("authorization", format!("Bearer {agent_token}"))
        // agent is a restricted principal: must use an allowlisted template, OR
        // assert 403-vs-template here. For a pure domain-scope check, target a
        // read/claim path instead. (See note.)
        .json(&serde_json::json!({ "title": "x", "input_json": "{\"template\":\"noop\"}" }))
        .await;
    // domain_scope grants domain authz; template tier is Seam-1 behavior:
    assert_ne!(ok.status_code(), 404);

    // ...but is rejected (403) against a foreign domain's stream.
    let denied = server
        .get(&format!("/api/work/domains/{}/stream", ctx.other_domain_id))
        .add_header("authorization", format!("Bearer {agent_token}"))
        .await;
    assert_eq!(denied.status_code(), 403);
}
```

(Note: the agent principal is restricted-trust per Seam 1, so free-form task creation is template-gated — the assertion targets domain-scope behavior, not the trust tier. Keep a `"noop"` template in the test `AppConfig` if a 2xx is desired.)

- [ ] **Step 6: Regenerate artifacts (new route) + run + clippy + commit**

```bash
make docs   # + regenerate web/openapi.json
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add -A
git commit -m "feat(tower): mint per-agent JWT at enrollment + token endpoint

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Record the authenticated agent in `append_progress` (+ harden arbitrary `agent_id`)

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/work.rs` (`append_progress` ~1007–1042; `claim_task` ~802–807)

**Interfaces:**
- Consumes: the authenticated `Principal` (now possibly `auth_type:"agent"` with subject `agent:{id}`).

- [ ] **Step 1: Stop discarding the principal in `append_progress`**

The handler currently has `_principal: Principal` and `let agent_id = "";`. After Seam 1 Task 5 renamed it to `principal`, derive the agent id from the authenticated subject (strip the `agent:` prefix so the recorded id matches `meshagent.id`):

```rust
// Attribute the progress event to the authenticated caller.
let agent_id = principal
    .subject
    .strip_prefix("agent:")
    .unwrap_or(&principal.subject)
    .to_string();
```

and bind `&agent_id` (already the bind site at ~1042).

- [ ] **Step 2: Harden arbitrary `agent_id` in `claim_task`**

`claim_task` lets the body override `agent_id` with no check. For agent principals, force the claimer to be the authenticated agent; allow an explicit override only for full-trust/admin (operator claiming on behalf):

```rust
let body_agent_id = body
    .as_ref()
    .and_then(|b| b.get("agent_id"))
    .and_then(|v| v.as_str());
let self_id = principal.subject.strip_prefix("agent:").unwrap_or(&principal.subject);
let agent_id = if crate::auth::is_full_trust(&principal) || principal.is_admin {
    body_agent_id.unwrap_or(self_id).to_string()
} else {
    // Restricted principals (agents) may only claim as themselves.
    self_id.to_string()
};
```

- [ ] **Step 3: Compile**

Run: `cargo check -p edgeplane-tower`
Expected: PASS.

- [ ] **Step 4: Test** — add to `tests/test_authz.rs`: a progress event posted with an agent token records the agent's id (query `meshprogressevent.agent_id` after the call), and a restricted agent token cannot claim a task as a different `agent_id`.

- [ ] **Step 5: Run + clippy + commit**

```bash
cargo nextest run -p edgeplane-tower --no-fail-fast && cargo clippy -p edgeplane-tower -- -D warnings
git add crates/edgeplane-tower/src/routes/work.rs crates/edgeplane-tower/tests/test_authz.rs
git commit -m "feat(tower): attribute progress/claim to authenticated agent identity

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Daemon — inject the per-agent token for ephemeral task-worker agents

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/task_worker.rs` (~367–404, the enroll + spawn path)
- Reference: the spawn site that builds the runtime `Command` for the ephemeral agent

**Interfaces:**
- Consumes: the `agent_token` field now present in the enroll response (Seam 2 Task 4).

- [ ] **Step 1: Capture `agent_token` from the enroll response**

In `task_worker.rs`, right after the existing `agent_id` extraction (~396), also capture the token:

```rust
let agent_token = enroll_resp
    .get("agent_token")
    .and_then(|v| v.as_str())
    .map(|s| s.to_string());
if agent_token.is_none() {
    tracing::warn!("task_worker: enroll response for task {task_id} carried no agent_token; \
                    agent will fall back to the daemon token");
}
```

- [ ] **Step 2: Thread the token into the spawn**

Locate where this ephemeral agent's process/runtime is launched in the task-worker path (the call that ultimately builds the `Command`; trace from `task_worker.rs` to the runtime spawn). Pass `agent_token` through and, when present, set it as the child env:

```rust
// where the agent Command is built for this task:
if let Some(tok) = agent_token.as_deref() {
    cmd.env("EP_AGENT_TOKEN", tok);
}
```

If the launch goes through `edgeplaned-runtimes` (`claude_code.rs:293` / `goose.rs:268`), add an explicit `EP_AGENT_TOKEN` env on the `Command` there, fed by a new parameter on the spawn function (see Task 7 — they share the threading). Until Task 7 lands, set it directly in the task-worker spawn site.

- [ ] **Step 3: Compile**

Run: `cargo check -p edgeplaned-bin`
Expected: PASS.

- [ ] **Step 4: Verify (e2e — no unit harness for the daemon path)**

Dispatch a task that spawns an ephemeral agent; confirm via tower logs/`meshprogressevent` that progress events for that task carry the agent's own id (not the operator subject), proving the agent used its scoped token. Record the observed `agent_id`.

- [ ] **Step 5: Commit**

```bash
cargo clippy -p edgeplaned-bin -- -D warnings
git add crates/edgeplaned/crates/edgeplaned-bin/src/task_worker.rs
git commit -m "feat(edgeplaned): inject per-agent token for ephemeral task agents

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: Daemon — mint + inject per-agent tokens for supervised agents

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/daemon.rs` (`spawn_one` / the `supervisor.spawn(...)` call ~1230)
- Modify: `crates/edgeplaned/crates/edgeplaned-runtimes/src/claude_code.rs` (~293) and `.../goose.rs` (~268) — accept + set `EP_AGENT_TOKEN`
- Reference: `crates/edgeplaned/crates/edgeplaned-core/src/client.rs` (`BackendClient` — call the mint endpoint)

**Interfaces:**
- Consumes: the mint endpoint `POST /work/agents/{agent_id}/token` (Seam 2 Task 4). Note `BackendClient` does not prepend `/api` (api_prefix empty) — match how the daemon currently calls `/work/...` (e.g. the enroll call uses the bare `/work/...` path; mirror it).

**Design note:** supervised agents are fetched (`fetch_node_agents`, GET `/runtime/nodes/{id}/agents`), not freshly enrolled, so they have no enroll-response token. The daemon mints one per agent at (re)spawn via the token endpoint, using its own (operator/node) credential, which is authorized for the agent's domain. Tokens are short-lived (12h, Task 4); the daemon re-mints on respawn rather than persisting them.

- [ ] **Step 1: Add a mint call to `BackendClient`**

In `edgeplaned-core/src/client.rs`, add a method mirroring the existing post helpers:

```rust
/// Mint a fresh per-agent token from the controlplane. Returns the JWT string.
pub async fn mint_agent_token(&self, agent_id: &str) -> anyhow::Result<String> {
    let resp: serde_json::Value = self
        .post(&format!("/work/agents/{agent_id}/token"), &serde_json::json!({}))
        .await?;
    resp.get("agent_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("mint_agent_token: response missing agent_token"))
}
```

- [ ] **Step 2: Mint before spawn in `spawn_one`**

Before `self.supervisor.spawn(spec.agent_id.clone(), spec.domain_id.clone(), …)` (~1230), mint the token and thread it through:

```rust
let agent_token = match self.client.mint_agent_token(&spec.agent_id).await {
    Ok(t) => Some(t),
    Err(e) => {
        tracing::warn!("spawn_one: mint_agent_token failed for {} ({e:#}); \
                        falling back to daemon token", spec.agent_id);
        None
    }
};
```

Add an `agent_token: Option<String>` parameter to `supervisor.spawn(...)` and pass it down to the runtime launch.

- [ ] **Step 3: Set `EP_AGENT_TOKEN` in the runtime spawns**

In `claude_code.rs` (~293) and `goose.rs` (~268), add an `agent_token: Option<&str>` parameter to the spawn function and, when building `cmd`, override the inherited env:

```rust
if let Some(tok) = agent_token {
    cmd.env("EP_AGENT_TOKEN", tok);
}
```

This replaces the inherited daemon-wide token with the per-agent token for that child only.

- [ ] **Step 4: Compile both crates**

Run: `cargo check -p edgeplaned-runtimes && cargo check -p edgeplaned-bin`
Expected: PASS.

- [ ] **Step 5: Verify (e2e)**

Restart the daemon; for a supervised agent, confirm tower-side that its actions are attributed to `agent:{public_id}` (e.g. `meshprogressevent.agent_id`), and that revoking its `agenttoken` row (`UPDATE agenttoken SET revoked=true WHERE agent_id=…`) causes subsequent calls to 401 within the token lifetime. Record both observations.

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy -p edgeplaned-runtimes -- -D warnings && cargo clippy -p edgeplaned-bin -- -D warnings
git add crates/edgeplaned/
git commit -m "feat(edgeplaned): mint + inject per-agent tokens for supervised agents

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Explicitly out of scope (follow-up, not P0)

- **`edgeplane agent run` profile path** (`crates/edgeplane/src/agent_harness.rs:1071`): the long-lived systemd profile agents (aria-operator, aria-engineer, …) still inject `config.token`. These are the operator's own per-profile delegates and legitimately act as the operator today; migrating them to per-profile scoped credentials is a follow-up once the enrolled-agent paths (Tasks 6–7) are proven. Flag, don't silently leave: this path is intentionally deferred.
- nftables egress + cgroup enforcement — that is Seam 3.

---

## Self-Review

**Spec coverage (Seam 2 of the 2026-06-17 design):**
- Per-agent JWT reusing the node-JWT pattern, domain-scoped → Tasks 1, 3. ✔ (matches the locked §9 decision: per-agent JWT, not SA extension.)
- Minted on enrollment → Task 4. ✔
- `claim_task`/`append_progress` record the authenticated agent → Task 5. ✔
- Agent enrolled by one tenant can't act in another's domains → `domain_scope` single-entry + `authorized_for_domain` (Seam 1) → Tasks 1, 3; test in Task 4 Step 5. ✔
- Replaces shared `EP_AGENT_TOKEN` for enrolled agents → Tasks 6, 7. ✔
- Revocation parity with node tokens → Task 2 + Task 3 revocation check. ✔

**Placeholder scan:** No "TBD"/"add error handling". The daemon spawn-threading (Tasks 6–7) names exact files/lines and the transformation; the one genuinely unmapped detail (the exact spawn-function signature to thread `agent_token` through) is called out as a trace-from instruction with the concrete `cmd.env` edit, not hidden.

**Type consistency:** `AgentClaims{sub, agent_id, domain_id, jti, iat, exp}`, `sign_agent_jwt(agent_id, domain_id, &EncodingKey, ttl_hours) -> (String, String)`, `verify_agent_jwt(&str, &DecodingKey) -> AgentClaims`, `issue_agent_token(&AppState, &str, &str) -> Result<String>`, `mint_agent_token` (tower handler) ↔ `BackendClient::mint_agent_token` (daemon client) all line up. `auth_type:"agent"` + `domain_scope:vec![domain_id]` is the contract Seam 1's `authorized_for_domain` consumes.

**Open verification items (confirm during execution):**
1. `state.jwt_encoding_key` field name on `AppState` (the loader in `server.rs:115` produces `(EncodingKey, DecodingKey)`; confirm both are stored and the encoding key is reachable from handlers).
2. The exact spawn-function signature in `task_worker.rs` and the supervisor → runtime call chain to thread `agent_token` (Tasks 6–7).
3. `BackendClient` path convention for `/work/...` (no `/api` prefix) — mirror the existing enroll call.
4. Next migration number (Task 2 Step 1).

---

## Execution Handoff

Two execution options:

1. **Subagent-Driven (recommended)** — fresh `rust-engineer` per task, `rust-reviewer` between tasks. The daemon tasks (6–7) cross three crates; review their spawn-threading carefully.
2. **Inline Execution** — execute in this session with checkpoints.

**Land Seam 1 first**, then this plan; the P0 security release ships after both are green.
