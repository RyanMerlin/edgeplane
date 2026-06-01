//! OpenAPI aggregator — DB-free spec generation.
//!
//! Only the three spike endpoints are listed here. Extending to more routes
//! is a separate task (Phase 0.8 bulk annotation). This module compiles and
//! runs without constructing AppState, opening a database connection, reading
//! environment secrets, or binding a network socket.

use serde::Serialize;
use utoipa::{OpenApi, ToSchema};

// ── Schema-only DTOs ─────────────────────────────────────────────────────────
//
// Health and governance handlers return ad-hoc `serde_json::Value` built from
// raw SQL rows. Annotating those handlers with a `serde_json::Value` body
// produces a useless "object" schema in OpenAPI. Instead, we define typed
// mirror structs here purely for the schema — the runtime wire format matches
// because we document what the handlers actually emit.

/// Response emitted by `GET /api/health`.
#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    /// Always `"ok"` when the server is reachable.
    pub status: String,
    /// Crate version string (e.g. `"0.12.0"`).
    pub version: String,
}

/// Response emitted by `GET /api/governance/policy/active`.
#[derive(Serialize, ToSchema)]
pub struct GovernancePolicyResponse {
    /// Database row ID.
    pub id: i32,
    /// Monotonically increasing version number.
    pub version: i32,
    /// Policy lifecycle state: `"active"`, `"draft"`, or `"archived"`.
    pub state: String,
    /// Parsed governance policy object.
    pub policy: serde_json::Value,
    /// Human-readable change note.
    pub change_note: String,
    /// Subject that created this policy record.
    pub created_by: String,
    /// Subject that published this policy (empty string for drafts).
    pub published_by: String,
    /// ISO-8601 UTC timestamp of publication; `null` for drafts.
    pub published_at: Option<String>,
    /// ISO-8601 UTC timestamp of record creation.
    pub created_at: String,
    /// ISO-8601 UTC timestamp of last update.
    pub updated_at: String,
}

// ── Path annotations ─────────────────────────────────────────────────────────
//
// These stubs exist solely to carry the `#[utoipa::path]` attribute so the
// macro can register the operation. They are never called at runtime — the
// real handlers live in `routes/health.rs`, `routes/auth.rs`, and
// `routes/governance.rs`. The `gen-openapi` binary only calls
// `ApiDoc::openapi()`, which is a pure compile-time type operation.

/// Liveness probe — returns `200 ok` with version string. No auth required.
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "system",
    responses(
        (status = 200, description = "Server is reachable", body = HealthResponse)
    )
)]
#[allow(dead_code)]
pub fn health_stub() {}

/// Return the authenticated caller's identity.
#[utoipa::path(
    get,
    path = "/api/auth/me",
    tag = "auth",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Caller identity", body = crate::models::auth::MeResponse),
        (status = 401, description = "Missing or invalid token")
    )
)]
#[allow(dead_code)]
pub fn auth_me_stub() {}

/// Return the currently active governance policy, seeding the default if none exists.
#[utoipa::path(
    get,
    path = "/api/governance/policy/active",
    tag = "governance",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Active governance policy", body = GovernancePolicyResponse),
        (status = 201, description = "Default policy seeded (first call on empty DB)", body = GovernancePolicyResponse),
        (status = 401, description = "Missing or invalid token")
    )
)]
#[allow(dead_code)]
pub fn governance_active_stub() {}

// ── Aggregator ────────────────────────────────────────────────────────────────

/// Minimal OpenAPI document covering the three spike endpoints.
///
/// Constructed entirely from types — no runtime state involved.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "EdgePlane Tower API",
        version = "0.1.0",
        description = "EdgePlane Tower — HTTP API (spike: 3-endpoint surface)"
    ),
    paths(
        health_stub,
        auth_me_stub,
        governance_active_stub,
    ),
    components(
        schemas(
            HealthResponse,
            GovernancePolicyResponse,
            crate::models::auth::MeResponse,
        )
    ),
    security(
        ("bearerAuth" = [])
    ),
    tags(
        (name = "system", description = "Infrastructure endpoints"),
        (name = "auth",   description = "Authentication and identity"),
        (name = "governance", description = "Policy lifecycle management"),
    )
)]
pub struct ApiDoc;
