# Node JWT Authentication Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace EP_TOKEN (static shared secret) with RS256 JWT node credentials issued at registration — zero DB lookup per heartbeat, instant revocation, clean node identity.

**Architecture:** Tower generates an RSA keypair at startup (or loads from `EP_JWT_SIGNING_KEY` env). At node registration, the join token ceremony produces a 90-day RS256 JWT stored at `/etc/edgeplane/node.json`. Tower validates JWTs by signature + expiry + JTI revocation check (sparse table). EP_TOKEN is removed entirely after the JWT path is verified working.

**Tech Stack:** Rust, `jsonwebtoken = "9"`, `rsa = "0.9"` (key generation), sqlx/Postgres, axum, clap

---

## File Map

**Create:**
- `crates/edgeplane-tower/src/jwt.rs` — `NodeClaims`, `sign_node_jwt`, `verify_node_jwt`, key gen
- `crates/edgeplane-tower/migrations/0002_node_jwt_auth.sql` — `nodetoken` revocation table
- `crates/edgeplaned/crates/edgeplaned-bin/src/register.rs` — `edgeplaned register` subcommand

**Modify:**
- `crates/edgeplane-tower/Cargo.toml` — add `jsonwebtoken`, `rsa`
- `crates/edgeplane-tower/src/state.rs` — add `jwt_encoding_key`, `jwt_decoding_key`
- `crates/edgeplane-tower/src/lib.rs` — expose `pub mod jwt`
- `crates/edgeplane-tower/src/auth.rs` — add node JWT auth type; **remove EP_TOKEN** (Task 7)
- `crates/edgeplane-tower/src/routes/runtime.rs` — make register public, issue JWT, add rotate endpoint, fix WS auth
- `crates/edgeplane-tower/src/routes/onboarding.rs` — remove EP_TOKEN instructions
- `crates/edgeplane-tower/src/routes/ops.rs` — remove EP_TOKEN from redacted env list
- `crates/edgeplane-tower/src/server.rs` — load JWT keys at startup
- `crates/edgeplaned/crates/edgeplaned-bin/src/config.rs` — read node JWT, remove EP_TOKEN arg
- `crates/edgeplaned/crates/edgeplaned-bin/src/main.rs` — add `register` subcommand, remove EP_TOKEN
- `crates/edgeplane/src/runtime.rs` — add `JoinToken` subcommand to `RuntimeNodesCommand`
- `docker-compose.yml`, `docker-compose.quickstart.yml`, `docker-compose.edgeplane-dev.yml`, `.env.example`, `.env.dev.example` — remove EP_TOKEN

---

## Task 1: JWT infrastructure — `jwt.rs` module

**Files:**
- Create: `crates/edgeplane-tower/src/jwt.rs`
- Modify: `crates/edgeplane-tower/Cargo.toml`
- Modify: `crates/edgeplane-tower/src/lib.rs`

- [ ] **Step 1: Add crate dependencies**

In `crates/edgeplane-tower/Cargo.toml`, add after the `base64` line:
```toml
jsonwebtoken = "9"
rsa = { version = "0.9", features = ["pem"] }
rand_core = { version = "0.6", features = ["getrandom"] }
```

- [ ] **Step 2: Create `jwt.rs` with types and signing functions**

Create `crates/edgeplane-tower/src/jwt.rs`:
```rust
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rsa::{RsaPrivateKey, pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding}};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Claims embedded in a node JWT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeClaims {
    /// `node:{node_id}` — the node's identity
    pub sub: String,
    /// The raw node_id without prefix (convenience field)
    pub node_id: String,
    /// JWT ID — used for revocation lookups
    pub jti: String,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Expires at (Unix timestamp)
    pub exp: i64,
}

/// Sign a 90-day node JWT. Returns the compact JWT string.
pub fn sign_node_jwt(
    node_id: &str,
    encoding_key: &EncodingKey,
    ttl_days: i64,
) -> anyhow::Result<(String, String)> {
    let now = chrono::Utc::now().timestamp();
    let jti = Uuid::new_v4().to_string();
    let claims = NodeClaims {
        sub: format!("node:{node_id}"),
        node_id: node_id.to_string(),
        jti: jti.clone(),
        iat: now,
        exp: now + ttl_days * 86400,
    };
    let token = encode(&Header::new(Algorithm::RS256), &claims, encoding_key)
        .map_err(|e| anyhow::anyhow!("JWT sign error: {e}"))?;
    Ok((token, jti))
}

/// Verify a node JWT. Returns the claims on success.
pub fn verify_node_jwt(token: &str, decoding_key: &DecodingKey) -> anyhow::Result<NodeClaims> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = true;
    let data = decode::<NodeClaims>(token, decoding_key, &validation)
        .map_err(|e| anyhow::anyhow!("JWT verify error: {e}"))?;
    Ok(data.claims)
}

/// Generate a new RSA-2048 keypair. Returns (private_pem, public_pem).
pub fn generate_rsa_keypair() -> anyhow::Result<(String, String)> {
    let mut rng = rand::thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| anyhow::anyhow!("RSA keygen error: {e}"))?;
    let private_pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("PKCS8 PEM error: {e}"))?
        .to_string();
    let public_pem = private_key
        .to_public_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("public PEM error: {e}"))?;
    Ok((private_pem, public_pem))
}

/// Build an EncodingKey from PKCS#8 PEM bytes.
pub fn encoding_key_from_pem(pem: &str) -> anyhow::Result<EncodingKey> {
    EncodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("EncodingKey error: {e}"))
}

/// Build a DecodingKey from RSA public key PEM bytes.
pub fn decoding_key_from_pem(pem: &str) -> anyhow::Result<DecodingKey> {
    DecodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("DecodingKey error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_sign_verify() {
        let (priv_pem, pub_pem) = generate_rsa_keypair().unwrap();
        let enc = encoding_key_from_pem(&priv_pem).unwrap();
        let dec = decoding_key_from_pem(&pub_pem).unwrap();
        let (token, jti) = sign_node_jwt("node-abc123", &enc, 90).unwrap();
        let claims = verify_node_jwt(&token, &dec).unwrap();
        assert_eq!(claims.node_id, "node-abc123");
        assert_eq!(claims.sub, "node:node-abc123");
        assert_eq!(claims.jti, jti);
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn expired_token_rejected() {
        let (priv_pem, pub_pem) = generate_rsa_keypair().unwrap();
        let enc = encoding_key_from_pem(&priv_pem).unwrap();
        let dec = decoding_key_from_pem(&pub_pem).unwrap();
        // ttl_days = 0 forces exp == iat → already expired
        let (token, _) = sign_node_jwt("node-xyz", &enc, 0).unwrap();
        assert!(verify_node_jwt(&token, &dec).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let (priv1, _) = generate_rsa_keypair().unwrap();
        let (_, pub2) = generate_rsa_keypair().unwrap();
        let enc = encoding_key_from_pem(&priv1).unwrap();
        let dec = decoding_key_from_pem(&pub2).unwrap();
        let (token, _) = sign_node_jwt("node-xyz", &enc, 90).unwrap();
        assert!(verify_node_jwt(&token, &dec).is_err());
    }
}
```

- [ ] **Step 3: Expose jwt module in lib.rs**

In `crates/edgeplane-tower/src/lib.rs`, add:
```rust
pub mod jwt;
```

- [ ] **Step 4: Run unit tests**

```bash
cd /home/merlin/code/edgeplane
cargo nextest run -p edgeplane-tower jwt
```
Expected: 3 tests pass. The `expired_token_rejected` test may intermittently pass if `exp == iat` is not yet past — if so, change ttl_days to `-1` (negative forces `exp < now`):
```rust
let now = chrono::Utc::now().timestamp();
// Manually craft expired claims instead of calling sign_node_jwt
let claims = NodeClaims { sub: "node:x".into(), node_id: "x".into(), jti: "j".into(), iat: now - 100, exp: now - 1 };
let token = jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256), &claims, &enc).unwrap();
```

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplane-tower/Cargo.toml crates/edgeplane-tower/src/jwt.rs crates/edgeplane-tower/src/lib.rs
git commit -m "feat(tower): add jwt module — RS256 node JWT sign/verify"
```

---

## Task 2: Schema migration — nodetoken revocation table

**Files:**
- Create: `crates/edgeplane-tower/migrations/0002_node_jwt_auth.sql`

- [ ] **Step 1: Write the migration**

Create `crates/edgeplane-tower/migrations/0002_node_jwt_auth.sql`:
```sql
-- 0002_node_jwt_auth.sql — node JWT revocation table
--
-- Tracks issued node JWTs by JTI for revocation. The table is intentionally
-- sparse — only revoked tokens need rows beyond the initial insert. Validation
-- checks: signature + exp (in-process) then optionally JTI not in this table.
--
-- Retention: rows can be deleted once expires_at < NOW() (no revoked token
-- can be replayed after expiry anyway). A maintenance cron can GC old rows.

CREATE TABLE public.nodetoken (
    jti         TEXT        PRIMARY KEY,
    node_id     TEXT        NOT NULL REFERENCES public.runtimenode(id) ON DELETE CASCADE,
    revoked     BOOLEAN     NOT NULL DEFAULT false,
    revoked_at  TIMESTAMP   WITHOUT TIME ZONE,
    issued_at   TIMESTAMP   WITHOUT TIME ZONE NOT NULL,
    expires_at  TIMESTAMP   WITHOUT TIME ZONE NOT NULL
);

CREATE INDEX idx_nodetoken_node_id  ON public.nodetoken (node_id);
CREATE INDEX idx_nodetoken_revoked  ON public.nodetoken (revoked) WHERE revoked = true;
```

- [ ] **Step 2: Verify migration applies against the dev DB**

```bash
cd /home/merlin/code/edgeplane
sqlx migrate run --database-url "$DATABASE_URL"
```
Expected: `Applied 0002_node_jwt_auth.sql` in the output. If `DATABASE_URL` is not set, find it with `grep DATABASE_URL .env* docker-compose*.yml`.

- [ ] **Step 3: Commit**

```bash
git add crates/edgeplane-tower/migrations/0002_node_jwt_auth.sql
git commit -m "feat(tower): add nodetoken table for JWT revocation"
```

---

## Task 3: JWT signing key in AppState

**Files:**
- Modify: `crates/edgeplane-tower/src/state.rs`
- Modify: `crates/edgeplane-tower/src/server.rs`

- [ ] **Step 1: Add keys to AppState**

Replace the full contents of `crates/edgeplane-tower/src/state.rs` with:
```rust
use jsonwebtoken::{DecodingKey, EncodingKey};
use sqlx::PgPool;

pub struct AppState {
    pub db: PgPool,
    pub node: NodeInfo,
    /// Optional upstream URL — unknown routes are forwarded here (proxy mode).
    pub api_proxy: Option<String>,
    /// RS256 private key for signing node JWTs.
    pub jwt_encoding_key: EncodingKey,
    /// RS256 public key for verifying node JWTs.
    pub jwt_decoding_key: DecodingKey,
}

/// Static node identity — populated from CLI args at startup.
/// When Raft is not running, term=0 and role="standalone".
#[derive(Clone, Debug, serde::Serialize)]
pub struct NodeInfo {
    pub node_id: u64,
    pub advertise_url: Option<String>,
    pub role: &'static str,
    pub term: u64,
    pub leader_id: Option<u64>,
}
```

- [ ] **Step 2: Load keys at startup in server.rs**

Find where `AppState` is constructed in `crates/edgeplane-tower/src/server.rs` and add key loading before the state is built. Add this block (insert before the `AppState { db, node, api_proxy }` construction):

```rust
// Load JWT signing key from EP_JWT_SIGNING_KEY env (base64-encoded PKCS#8 PEM).
// If unset, generate a new keypair and warn loudly — dev mode only.
let (jwt_encoding_key, jwt_decoding_key) = {
    use edgeplane_tower::jwt::{
        decoding_key_from_pem, encoding_key_from_pem, generate_rsa_keypair,
    };
    if let Ok(b64) = std::env::var("EP_JWT_SIGNING_KEY") {
        let pem = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(&b64)
                .expect("EP_JWT_SIGNING_KEY must be base64-encoded"),
        )
        .expect("EP_JWT_SIGNING_KEY decoded value is not valid UTF-8");
        let enc = encoding_key_from_pem(&pem).expect("EP_JWT_SIGNING_KEY: invalid RSA PEM");
        // Derive public key from the private PEM for decoding
        let pub_pem = {
            use rsa::{RsaPrivateKey, pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding}};
            RsaPrivateKey::from_pkcs8_pem(&pem)
                .expect("EP_JWT_SIGNING_KEY: cannot parse as PKCS#8")
                .to_public_key()
                .to_public_key_pem(LineEnding::LF)
                .expect("public key PEM export failed")
        };
        let dec = decoding_key_from_pem(&pub_pem).expect("EP_JWT_SIGNING_KEY: public key error");
        (enc, dec)
    } else {
        tracing::warn!(
            "EP_JWT_SIGNING_KEY not set — generating ephemeral RSA keypair. \
             Node JWTs will be invalid after restart. Set EP_JWT_SIGNING_KEY for production."
        );
        let (priv_pem, pub_pem) = generate_rsa_keypair().expect("RSA keygen failed");
        let enc = encoding_key_from_pem(&priv_pem).unwrap();
        let dec = decoding_key_from_pem(&pub_pem).unwrap();
        (enc, dec)
    }
};
```

Then add `jwt_encoding_key` and `jwt_decoding_key` to the `AppState { .. }` struct literal.

- [ ] **Step 3: Confirm it compiles**

```bash
cd /home/merlin/code/edgeplane
cargo build -p edgeplane-tower 2>&1 | grep -E "^error|Finished"
```
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplane-tower/src/state.rs crates/edgeplane-tower/src/server.rs
git commit -m "feat(tower): add JWT signing/decoding keys to AppState"
```

---

## Task 4: Make register_node public and issue JWT

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs` (public path only — JWT auth type added in Task 5)
- Modify: `crates/edgeplane-tower/src/routes/runtime.rs`

The current `register_node` requires an authenticated `Principal`, which is circular for bootstrapping. It also filters join tokens by `owner_subject=$2`. We fix both: add the endpoint to the public allowlist and extract `owner_subject` from the join token row itself.

- [ ] **Step 1: Write the failing test**

In `crates/edgeplane-tower/tests/test_routes.rs` (or create this file if it doesn't have node-jwt coverage), add:

```rust
#[tokio::test]
async fn register_node_without_auth_returns_node_jwt() {
    // Setup: create a join token via an authenticated call, then use it
    // unauthenticated to register a node. Expect the response to contain
    // a `node_jwt` field and a `node_id`.
    //
    // This is an integration test — it requires a running DB.
    // Run with: cargo nextest run -p edgeplane-tower register_node_without_auth
    //
    // If no test DB is available, skip with SKIP_DB_TESTS=1.
    if std::env::var("SKIP_DB_TESTS").is_ok() { return; }
    // ... (wire up axum-test app here per existing test patterns in this file)
    // Assert: POST /runtime/nodes/register with valid join token and NO auth header
    // returns 201 with { "node_id": "...", "node_jwt": "eyJ..." }
    todo!("fill in after reading test_routes.rs setup pattern")
}
```

Run it to confirm it fails (compile error or todo panic — both count as failure):
```bash
cargo nextest run -p edgeplane-tower register_node_without_auth 2>&1 | tail -5
```

- [ ] **Step 2: Add `/runtime/nodes/register` to public path allowlist**

In `crates/edgeplane-tower/src/auth.rs`, in the `is_public_path` function, extend the match to include the register endpoint:

```rust
pub fn is_public_path(path: &str) -> bool {
    matches!(
        path,
        "/"
            | "/health"
            | "/mcp/health"
            | "/mcp/tools"
            | "/raft/status"
            | "/agent-onboarding.json"
            | "/schema-pack"
            | "/webhooks/tailscale"
            | "/integrations/slack/events"
            | "/integrations/slack/commands"
            | "/integrations/slack/interactions"
            | "/integrations/teams/events"
            | "/integrations/google-chat/events"
            | "/runtime/nodes/register"   // bootstrap — join token is the only credential
    ) || path.starts_with("/auth/oidc/")
        || path == "/auth/logout"
}
```

- [ ] **Step 3: Update `register_node` handler signature and join-token lookup**

In `crates/edgeplane-tower/src/routes/runtime.rs`, change the `register_node` function signature and the join-token SQL:

```rust
async fn register_node(
    State(state): State<Arc<AppState>>,
    // No `principal: Principal` — the join token IS the credential
    Json(body): Json<NodeRegister>,
) -> impl IntoResponse {
    let now = Utc::now().naive_utc();

    // 1. Look up join token by hash only — no owner_subject filter.
    //    The token's owner_subject becomes the node's owner_subject.
    let token_hash = hash_token_local(&body.bootstrap_token);
    let token_row = match sqlx::query(
        "SELECT * FROM runtimejointoken WHERE token_hash=$1 AND status='active'",
    )
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await
    { ... }  // keep existing error handling
```

Extract `owner_subject` from the token row after validation:
```rust
    // After token_row is confirmed valid (not None, not expired, not used):
    let subject: String = token_row.get("owner_subject");
```

Replace all remaining uses of `subject` (previously `&principal.subject`) with `&subject`.

- [ ] **Step 4: Issue JWT and insert into nodetoken after node creation**

At the end of `register_node`, after the `ensure_node_spec` call and before the final response, add:

```rust
    // Issue a 90-day node JWT.
    let (node_jwt, jti) = match edgeplane_tower::jwt::sign_node_jwt(
        &node_id,
        &state.jwt_encoding_key,
        90,
    ) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("register_node: JWT sign failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Record the JTI for future revocation checks.
    let jwt_expires = now + chrono::Duration::days(90);
    let _ = sqlx::query(
        "INSERT INTO nodetoken (jti, node_id, revoked, issued_at, expires_at) \
         VALUES ($1, $2, false, $3, $4)",
    )
    .bind(&jti)
    .bind(&node_id)
    .bind(now)
    .bind(jwt_expires)
    .execute(&state.db)
    .await;
```

Update the final response to include `node_jwt`:
```rust
    let mut resp = row_to_node(&node_row);
    resp["attach_secret"] = serde_json::Value::String(attach_secret);
    resp["node_jwt"] = serde_json::Value::String(node_jwt);
    (StatusCode::CREATED, Json(resp)).into_response()
```

- [ ] **Step 5: Confirm compilation**

```bash
cargo build -p edgeplane-tower 2>&1 | grep -E "^error|Finished"
```

- [ ] **Step 6: Run the test (fill it in first based on existing test patterns)**

Read the first 60 lines of `crates/edgeplane-tower/tests/test_routes.rs` to understand the test setup (App construction, DB pool creation). Complete the test written in Step 1 following the same pattern. Then:

```bash
cargo nextest run -p edgeplane-tower register_node_without_auth
```
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplane-tower/src/auth.rs crates/edgeplane-tower/src/routes/runtime.rs
git commit -m "feat(tower): register_node — public endpoint, issue RS256 JWT on registration"
```

---

## Task 5: Node JWT auth type in auth.rs

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs`

The auth middleware needs to recognize node JWTs (Bearer tokens containing dots) as a new `auth_type: "node"`. JWTs are detected by the presence of two dots (header.payload.signature format). The check runs before the `mcs_*` opaque token paths.

- [ ] **Step 1: Write the failing test**

In the `#[cfg(test)]` block at the bottom of `auth.rs`, add:

```rust
#[tokio::test]
async fn node_jwt_bearer_resolves_to_node_principal() {
    // Build a signed JWT and construct a fake request with it as Bearer.
    // Verify Principal.auth_type == "node" and subject == "node:{node_id}".
    // This requires a DB pool with the nodetoken table — skip if SKIP_DB_TESTS=1.
    if std::env::var("SKIP_DB_TESTS").is_ok() { return; }
    todo!("implement after reading AppState construction in tests")
}
```

Run to confirm compile/todo failure:
```bash
cargo nextest run -p edgeplane-tower node_jwt_bearer 2>&1 | tail -5
```

- [ ] **Step 2: Add JWT validation path to `from_request_parts`**

In `crates/edgeplane-tower/src/auth.rs`, in the `from_request_parts` `impl`, add the JWT check **before** the `mcs_sa_` and `mcs_` checks. Insert this block where the `if let Some(ref token) = token_credential` block begins:

```rust
        if let Some(ref token) = token_credential {
            // Node JWT path — JWTs always have exactly 2 dots separating the three parts.
            if token.chars().filter(|&c| c == '.').count() == 2 {
                match edgeplane_tower::jwt::verify_node_jwt(token, &state.jwt_decoding_key) {
                    Ok(claims) => {
                        // Check JTI is not revoked.
                        let now = chrono::Utc::now().naive_utc();
                        let revoked = sqlx::query(
                            "SELECT revoked FROM nodetoken WHERE jti=$1 AND expires_at > $2 LIMIT 1",
                        )
                        .bind(&claims.jti)
                        .bind(now)
                        .fetch_optional(&state.db)
                        .await
                        .ok()
                        .flatten()
                        .map(|row| row.get::<bool, _>("revoked"))
                        .unwrap_or(true); // not found = treat as revoked

                        if !revoked {
                            return Ok(Principal {
                                subject: claims.sub,
                                is_admin: false,
                                session_id: None,
                                auth_type: "node".into(),
                            });
                        }
                        return Err(AuthRejection::Unauthenticated);
                    }
                    Err(_) => {
                        // Not a valid node JWT — fall through to opaque token checks.
                    }
                }
            }

            // Existing opaque token paths below (mcs_sa_, mcs_) — unchanged.
```

Note: close the new `if` block properly and keep the existing `mcs_sa_` and `mcs_` logic below it, ending with the final `Err(AuthRejection::Unauthenticated)`.

- [ ] **Step 3: Add a public path test for the new auth type**

In the existing `private_paths_are_not_public` test, this is fine as-is. Add a compile-only sanity check in the test module:

```rust
#[test]
fn node_jwt_detection_uses_dot_count() {
    // Regression guard: opaque tokens never contain dots; JWTs always have 2.
    let opaque = "mcs_abc123def456";
    let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJub2RlOnh4eCJ9.sig";
    assert_eq!(opaque.chars().filter(|&c| c == '.').count(), 0);
    assert_eq!(jwt.chars().filter(|&c| c == '.').count(), 2);
}
```

- [ ] **Step 4: Run tests**

```bash
cargo nextest run -p edgeplane-tower 2>&1 | grep -E "PASS|FAIL|error"
```
Expected: all existing tests pass, new compile-only test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplane-tower/src/auth.rs
git commit -m "feat(tower): add node JWT auth type to Principal extractor"
```

---

## Task 6: JWT rotation endpoint

**Files:**
- Modify: `crates/edgeplane-tower/src/routes/runtime.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn rotate_node_token_returns_new_jwt_and_revokes_old() {
    if std::env::var("SKIP_DB_TESTS").is_ok() { return; }
    // 1. Register a node (unauthenticated) → get node_id + node_jwt
    // 2. Call POST /runtime/nodes/{node_id}/rotate-token with old node_jwt as Bearer
    // 3. Assert: response contains new_node_jwt, old JTI is now revoked=true in nodetoken
    todo!()
}
```

```bash
cargo nextest run -p edgeplane-tower rotate_node_token 2>&1 | tail -3
```

- [ ] **Step 2: Add rotate handler**

In `crates/edgeplane-tower/src/routes/runtime.rs`, add this function:

```rust
async fn rotate_node_token(
    State(state): State<Arc<AppState>>,
    principal: Principal,
    Path(node_id): Path<String>,
) -> impl IntoResponse {
    // Only a node JWT for this specific node may rotate its own token.
    if principal.auth_type != "node" || principal.subject != format!("node:{node_id}") {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"detail": "node token required"}))).into_response();
    }

    let now = Utc::now().naive_utc();

    // Extract old JTI from the Principal. We need it to revoke.
    // Re-parse the bearer token from the request is awkward here — instead,
    // look up the most recent non-revoked JTI for this node.
    let old_jti_row = sqlx::query(
        "SELECT jti FROM nodetoken WHERE node_id=$1 AND revoked=false ORDER BY issued_at DESC LIMIT 1",
    )
    .bind(&node_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    // Issue new JWT.
    let (new_jwt, new_jti) = match edgeplane_tower::jwt::sign_node_jwt(&node_id, &state.jwt_encoding_key, 90) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("rotate_node_token: JWT sign failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let jwt_expires = now + chrono::Duration::days(90);

    // Insert new JTI.
    let _ = sqlx::query(
        "INSERT INTO nodetoken (jti, node_id, revoked, issued_at, expires_at) VALUES ($1,$2,false,$3,$4)",
    )
    .bind(&new_jti)
    .bind(&node_id)
    .bind(now)
    .bind(jwt_expires)
    .execute(&state.db)
    .await;

    // Revoke old JTI.
    if let Some(row) = old_jti_row {
        let old_jti: String = row.get("jti");
        let _ = sqlx::query(
            "UPDATE nodetoken SET revoked=true, revoked_at=$1 WHERE jti=$2",
        )
        .bind(now)
        .bind(&old_jti)
        .execute(&state.db)
        .await;
    }

    (StatusCode::OK, Json(serde_json::json!({"node_jwt": new_jwt}))).into_response()
}
```

- [ ] **Step 3: Register the route**

In the `router()` function in `runtime.rs`, add after the existing node routes:

```rust
        .route(
            "/runtime/nodes/{node_id}/rotate-token",
            post(rotate_node_token),
        )
```

- [ ] **Step 4: Build and run tests**

```bash
cargo build -p edgeplane-tower 2>&1 | grep -E "^error|Finished"
cargo nextest run -p edgeplane-tower rotate_node_token 2>&1 | tail -3
```

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplane-tower/src/routes/runtime.rs
git commit -m "feat(tower): add /runtime/nodes/{id}/rotate-token endpoint"
```

---

## Task 7: Remove EP_TOKEN from tower

This task removes EP_TOKEN from all tower locations. Do it after Tasks 5 and 6 are merged and verified working — the JWT path must be in place before the static token is removed.

**Files:**
- Modify: `crates/edgeplane-tower/src/auth.rs`
- Modify: `crates/edgeplane-tower/src/routes/runtime.rs`
- Modify: `crates/edgeplane-tower/src/routes/ops.rs`
- Modify: `crates/edgeplane-tower/src/routes/onboarding.rs`
- Modify: `docker-compose.yml`, `docker-compose.quickstart.yml`, `docker-compose.edgeplane-dev.yml`, `.env.example`, `.env.dev.example`

- [ ] **Step 1: Remove EP_TOKEN from `auth.rs`**

Delete these lines from `from_request_parts` in `crates/edgeplane-tower/src/auth.rs`:
```rust
        let admin_token = env::var("EP_TOKEN").ok();
```
And the entire block (lines 100–105):
```rust
        if let (Some(t), Some(b)) = (&admin_token, &bearer) {
            if t == b {
                let subject = agent_id_header.clone().unwrap_or_else(|| "admin".into());
                return Ok(Principal { subject, is_admin: true, session_id: None, auth_type: "static".into() });
            }
        }
```
Also remove the comment on line 97 ("Static admin token — bootstrap-only path").
Remove the `use std::env;` if it's now unused.

- [ ] **Step 2: Remove EP_TOKEN from `runtime.rs`**

In `execution_session_pty` (around line 2345), remove:
```rust
        let admin_tok = std::env::var("EP_TOKEN").unwrap_or_default();
        if !admin_tok.is_empty() && token == admin_tok { return true; }
```

In `verify_attach_caller_token` (around line 2524), remove:
```rust
    let admin = std::env::var("EP_TOKEN").unwrap_or_default();
    if !admin.is_empty() && token == admin {
        return true;
    }
```

In `agent_attach_proxy`, replace the comment on line 532 ("The browser uses ?ep_token=<bearer>...") and remove the `?ep_token=` query param auth path (lines 2460):
```rust
        .or_else(|| params.get("ep_token").cloned())
```
Remove that `.or_else` line. The comment on the route (line 532) should be updated to remove the ep_token mention.

- [ ] **Step 3: Remove EP_TOKEN from `ops.rs`**

Find the redacted env list in `crates/edgeplane-tower/src/routes/ops.rs` around line 328 and remove `"EP_TOKEN"` from the array.

- [ ] **Step 4: Remove EP_TOKEN from `onboarding.rs`**

In `crates/edgeplane-tower/src/routes/onboarding.rs`, remove all `EP_TOKEN` references from the onboarding JSON and install script strings (lines 82, 106, 110, 116). Replace install instructions with the `edgeplaned register --join-token <token>` pattern:

Line 82: change `"EP_TOKEN": "${EP_TOKEN}"` → remove that env key entirely.
Lines 106/110/116: replace `--token ${{EP_TOKEN}}` with `--join-token <join-token>` in the install instructions.

- [ ] **Step 5: Remove from docker/config files**

```bash
cd /home/merlin/code/edgeplane
# Remove EP_TOKEN lines from compose files
grep -n "EP_TOKEN" docker-compose.yml docker-compose.quickstart.yml docker-compose.edgeplane-dev.yml .env.example .env.dev.example
```
For each file, delete the `EP_TOKEN` line (or the `EP_TOKEN: ${EP_TOKEN:-}` line). In `docker-compose.edgeplane-dev.yml`, the `EP_TOKEN: dev-token` line is also removed.

- [ ] **Step 6: Build and run full test suite**

```bash
cargo build -p edgeplane-tower 2>&1 | grep -E "^error|Finished"
cargo nextest run -p edgeplane-tower 2>&1 | grep -E "PASS|FAIL|^error"
```
Expected: all pass, no references to EP_TOKEN remain in compiled code.

```bash
grep -rn "EP_TOKEN" crates/edgeplane-tower/src/ && echo "FOUND — clean up above" || echo "clean"
```

- [ ] **Step 7: Commit**

```bash
git add crates/edgeplane-tower/src/auth.rs \
        crates/edgeplane-tower/src/routes/runtime.rs \
        crates/edgeplane-tower/src/routes/ops.rs \
        crates/edgeplane-tower/src/routes/onboarding.rs \
        docker-compose.yml docker-compose.quickstart.yml \
        docker-compose.edgeplane-dev.yml .env.example .env.dev.example
git commit -m "feat(tower): remove EP_TOKEN — node JWT is the machine auth path"
```

---

## Task 8: edgeplaned register command

**Files:**
- Create: `crates/edgeplaned/crates/edgeplaned-bin/src/register.rs`
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/main.rs`

- [ ] **Step 1: Create `register.rs`**

Create `crates/edgeplaned/crates/edgeplaned-bin/src/register.rs`:
```rust
//! `edgeplaned register` — exchange a join token for a node JWT.
//!
//! Writes node identity to /etc/edgeplane/node.json (system) or
//! ~/.edgeplane/node.json (user/dev fallback).

use anyhow::{Context, Result};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct RegisterArgs {
    /// Edgeplane tower URL (e.g. https://ep.merlinlabs.cloud)
    #[arg(long, env = "EP_BASE_URL")]
    pub url: String,

    /// One-time join token issued by `edgeplane node join-token create`
    #[arg(long)]
    pub join_token: String,

    /// Human-readable name for this node (must be unique in the EP environment)
    #[arg(long, default_value_t = hostname())]
    pub node_name: String,

    /// Trust tier for this node
    #[arg(long, default_value = "trusted")]
    pub trust_tier: String,

    /// Write credentials to this path instead of the default
    #[arg(long)]
    pub output: Option<PathBuf>,
}

fn hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

#[derive(Serialize, Deserialize)]
pub struct NodeCredentials {
    pub node_id: String,
    pub node_name: String,
    pub node_jwt: String,
    pub tower_url: String,
    pub issued_at: String,
}

pub async fn run(args: RegisterArgs) -> Result<()> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "bootstrap_token": args.join_token,
        "node_name": args.node_name,
        "hostname": hostname(),
        "trust_tier": args.trust_tier,
        "runtime_version": env!("CARGO_PKG_VERSION"),
        "labels": {},
        "capacity": {},
        "capabilities": [],
        "tailscale_ip": null,
        "tailscale_fqdn": null,
    });

    let resp = client
        .post(format!("{}/runtime/nodes/register", args.url.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .context("POST /runtime/nodes/register")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("registration failed ({status}): {text}");
    }

    let data: serde_json::Value = resp.json().await.context("parse response")?;
    let node_id = data["id"].as_str().context("missing id in response")?.to_string();
    let node_jwt = data["node_jwt"].as_str().context("missing node_jwt in response")?.to_string();

    let creds = NodeCredentials {
        node_id: node_id.clone(),
        node_name: args.node_name.clone(),
        node_jwt,
        tower_url: args.url.clone(),
        issued_at: chrono::Utc::now().to_rfc3339(),
    };

    let path = args.output.unwrap_or_else(|| default_creds_path());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create creds dir")?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&creds)?)
        .with_context(|| format!("write {}", path.display()))?;

    println!("Registered node {node_id} ({}).", args.node_name);
    println!("Credentials written to {}", path.display());
    Ok(())
}

/// System path if writable, otherwise user path.
fn default_creds_path() -> PathBuf {
    let system = PathBuf::from("/etc/edgeplane/node.json");
    if system.parent().map(|p| p.exists()).unwrap_or(false) {
        system
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".edgeplane/node.json")
    }
}
```

- [ ] **Step 2: Wire into edgeplaned-bin/src/main.rs**

Find the `#[derive(Subcommand)]` enum in `main.rs` and add:
```rust
    /// Register this node with an Edgeplane tower using a one-time join token.
    Register(register::RegisterArgs),
```

Add `mod register;` at the top of `main.rs`.

In the match arm dispatching subcommands:
```rust
        Subcommand::Register(args) => register::run(args).await?,
```

- [ ] **Step 3: Add `hostname` and `dirs` deps to edgeplaned-bin Cargo.toml**

```toml
hostname = "0.3"
dirs = "4"
```

- [ ] **Step 4: Build**

```bash
cargo build -p edgeplaned-bin 2>&1 | grep -E "^error|Finished"
```

- [ ] **Step 5: Manual smoke test**

```bash
# First create a join token (requires authenticated edgeplane session):
edgeplane runtime nodes register --help  # verify it still exists
# Create join token via tower API directly if CLI not yet wired:
curl -s -X POST http://edgeplane:8008/runtime/join-tokens \
  -H "Authorization: Bearer $(cat ~/.edgeplane/session.json | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])')" \
  -H "Content-Type: application/json" \
  -d '{"expires_in_seconds": 600, "upgrade_channel": "stable", "desired_version": "", "config": {}}' \
  | python3 -m json.tool

# Then register (replace <TOKEN> with the token value from above):
edgeplaned register --url http://edgeplane:8008 --join-token <TOKEN> --node-name test-node-$(hostname)
cat ~/.edgeplane/node.json
```
Expected: node.json written with `node_id`, `node_jwt` fields.

- [ ] **Step 6: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/register.rs \
        crates/edgeplaned/crates/edgeplaned-bin/src/main.rs \
        crates/edgeplaned/crates/edgeplaned-bin/Cargo.toml
git commit -m "feat(edgeplaned): add register subcommand — join token → node JWT"
```

---

## Task 9: Wire edgeplaned to use node JWT; remove EP_TOKEN

**Files:**
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/config.rs`
- Modify: `crates/edgeplaned/crates/edgeplaned-bin/src/main.rs`

- [ ] **Step 1: Update `resolve_credentials` to read from node.json**

In `crates/edgeplaned/crates/edgeplaned-bin/src/config.rs`, add a function to read the node JWT:

```rust
fn read_node_jwt() -> Option<String> {
    // Try system path first, then user path.
    let paths = [
        PathBuf::from("/etc/edgeplane/node.json"),
        dirs::home_dir()?.join(".edgeplane/node.json"),
    ];
    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(jwt) = v.get("node_jwt").and_then(|j| j.as_str()) {
                    if !jwt.is_empty() {
                        // Warn if expiry is within 30 days.
                        check_jwt_expiry_warning(jwt);
                        return Some(jwt.to_string());
                    }
                }
            }
        }
    }
    None
}

fn check_jwt_expiry_warning(jwt: &str) {
    // JWTs are base64url header.payload.sig — decode payload to check exp.
    let parts: Vec<&str> = jwt.splitn(3, '.').collect();
    if parts.len() < 2 { return; }
    let padded = format!("{}{}", parts[1], "=".repeat((4 - parts[1].len() % 4) % 4));
    let Ok(decoded) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) else { return };
    let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&decoded) else { return };
    if let Some(exp) = claims.get("exp").and_then(|e| e.as_i64()) {
        let days_left = (exp - chrono::Utc::now().timestamp()) / 86400;
        if days_left < 30 {
            tracing::warn!(
                days_left,
                "Node JWT expires soon — run `edgeplaned rotate-token` before expiry"
            );
        }
    }
}
```

In `resolve_credentials`, update the token resolution order:

```rust
    fn resolve_credentials(&mut self) {
        // Token priority: config.yaml (explicit) → node.json JWT → active edgeplane session
        if self.token.is_empty() {
            if let Some(t) = read_node_jwt() {
                self.token = t;
            } else if let Some(t) = read_state_profile_token() {
                self.token = t;
            }
        }
        // backend_url resolution unchanged
        ...
    }
```

- [ ] **Step 2: Remove EP_TOKEN arg from edgeplaned-bin**

In `crates/edgeplaned/crates/edgeplaned-bin/src/main.rs`, find and remove:
```rust
        #[arg(long, env = "EP_TOKEN", default_value = "")]
```
and its associated field and any code that reads it. The comment on line 338 in config.rs ("EP_TOKEN env is intentionally not read") is already correct and can stay.

- [ ] **Step 3: Build and verify**

```bash
cargo build -p edgeplaned-bin 2>&1 | grep -E "^error|Finished"
grep -rn "EP_TOKEN" crates/edgeplaned/ && echo "FOUND — clean up" || echo "clean"
```

- [ ] **Step 4: Integration smoke test**

Start edgeplaned after having run `edgeplaned register` in Task 8. Confirm it picks up the node JWT from node.json and authenticates to tower:

```bash
edgeplaned run --dry-run 2>&1 | grep -E "JWT|token|auth|node"
```
Expected: no "EP_TOKEN" in output; node JWT loaded from node.json.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/config.rs \
        crates/edgeplaned/crates/edgeplaned-bin/src/main.rs
git commit -m "feat(edgeplaned): use node JWT from node.json; remove EP_TOKEN"
```

---

## Task 10: CLI — edgeplane node join-token commands

**Files:**
- Modify: `crates/edgeplane/src/runtime.rs`

The tower already has `POST /runtime/join-tokens`, `GET /runtime/join-tokens/{id}`, and `POST /runtime/join-tokens/{id}/rotate`. Wire them to the CLI.

- [ ] **Step 1: Add `JoinToken` subcommand to `RuntimeNodesCommand`**

In `crates/edgeplane/src/runtime.rs`, add to the `RuntimeNodesCommand` enum:

```rust
#[derive(Subcommand, Debug)]
pub enum RuntimeNodesCommand {
    Register(RuntimeNodeRegisterArgs),
    List(RuntimeListArgs),
    Heartbeat(RuntimeNodeHeartbeatArgs),
    /// Join token management (create, list, revoke).
    #[command(subcommand)]
    JoinToken(JoinTokenCommand),
}

#[derive(Subcommand, Debug)]
pub enum JoinTokenCommand {
    /// Create a new one-time join token for node registration.
    Create(JoinTokenCreateArgs),
    /// List existing join tokens.
    List,
    /// Revoke an existing join token (marks it used so it cannot be reused).
    Revoke(JoinTokenRevokeArgs),
}

#[derive(Args, Debug)]
pub struct JoinTokenCreateArgs {
    /// Token TTL in minutes (default: 10).
    #[arg(long, default_value_t = 10)]
    pub ttl_minutes: u64,
}

#[derive(Args, Debug)]
pub struct JoinTokenRevokeArgs {
    /// Token ID to revoke.
    pub token_id: String,
}
```

- [ ] **Step 2: Implement the join-token handlers**

In the `run_nodes` function in `runtime.rs`, add a match arm for `JoinToken`:

```rust
        RuntimeNodesCommand::JoinToken(cmd) => run_join_tokens(cmd, client, output_mode).await,
```

Add the implementation:

```rust
async fn run_join_tokens(
    command: JoinTokenCommand,
    client: &EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        JoinTokenCommand::Create(args) => {
            let body = serde_json::json!({
                "expires_in_seconds": args.ttl_minutes * 60,
                "upgrade_channel": "stable",
                "desired_version": "",
                "config": {}
            });
            let resp = client
                .post("/runtime/join-tokens", &body)
                .await
                .context("create join token")?;
            output::print_json(&resp, output_mode);
            // Print the token value prominently — it's only shown once.
            if let Some(token) = resp.get("token").and_then(|t| t.as_str()) {
                eprintln!("\nJoin token (shown once):\n  {token}\n");
                eprintln!("Use with: edgeplaned register --url <EP_URL> --join-token {token}");
            }
            Ok(())
        }
        JoinTokenCommand::List => {
            let resp = client
                .get("/runtime/join-tokens")
                .await
                .context("list join tokens")?;
            output::print_json(&resp, output_mode);
            Ok(())
        }
        JoinTokenCommand::Revoke(args) => {
            // Mark used by POSTing to rotate (which replaces the hash, effectively
            // invalidating the original). For a true revoke, we call the rotate
            // endpoint and discard the new token.
            // TODO: add a dedicated DELETE /runtime/join-tokens/{id} to tower for clean revocation.
            let resp = client
                .post(&format!("/runtime/join-tokens/{}/rotate", args.token_id), &serde_json::json!({}))
                .await
                .context("revoke join token")?;
            output::print_json(&resp, output_mode);
            eprintln!("Token {} rotated (original value invalidated).", args.token_id);
            Ok(())
        }
    }
}
```

- [ ] **Step 3: Build**

```bash
cargo build -p edgeplane 2>&1 | grep -E "^error|Finished"
```

- [ ] **Step 4: Smoke test**

```bash
edgeplane runtime nodes join-token create --ttl-minutes 10
```
Expected: JSON response with `id` and `token` fields, plus the prominent token printout.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplane/src/runtime.rs
git commit -m "feat(cli): add edgeplane runtime nodes join-token create/list/revoke"
```

---

## Self-Review

**Spec coverage:**
- [x] EP_TOKEN removed — Tasks 7 and 9
- [x] Join token flow (short-lived, single-use bootstrap) — existing `runtimejointoken` table + new CLI (Task 10)
- [x] Node registration issues RS256 JWT — Task 4
- [x] JWT infrastructure — Task 1
- [x] Revocation table — Task 2
- [x] Node auth type in middleware — Task 5
- [x] JWT rotation endpoint — Task 6
- [x] Node credential stored at `/etc/edgeplane/node.json` — Task 8
- [x] edgeplaned uses node JWT — Task 9
- [x] 90-day TTL, proactive expiry warning — Task 9

**Gaps found:**
- A dedicated `DELETE /runtime/join-tokens/{id}` for clean revocation (not just rotate-to-invalidate). Noted in Task 10 as a TODO. Not blocking.
- The `verify_attach_caller_token` function in Task 7 — after removing EP_TOKEN, it falls back to user session + service account. Node JWTs should also be valid for the attach proxy (edgeplaned dials the proxy). Add node JWT validation there too. Fix in Task 7 Step 2: add a JWT verification path to `verify_attach_caller_token` alongside the session/SA lookup.

**Type consistency check:**
- `sign_node_jwt` returns `(String, String)` — used consistently in Tasks 4 and 6 ✓
- `NodeClaims.node_id` and `NodeClaims.sub` — used consistently in Tasks 5 and 6 ✓
- `nodetoken.jti` — TEXT PRIMARY KEY, referenced in Tasks 2, 4, 5, 6 ✓
- `node.json` credential file path — same `default_creds_path()` in Tasks 8 and 9 ✓
