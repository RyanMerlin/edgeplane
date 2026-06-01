//! OpenAPI aggregator — DB-free spec generation.
//!
//! Compiles and runs without constructing AppState, opening a database
//! connection, reading environment secrets, or binding a network socket.

use serde::Serialize;
use utoipa::{OpenApi, ToSchema};

// ── Schema-only DTO (health only — governance now uses real models) ───────────

/// Response emitted by `GET /api/health`.
#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    /// Always `"ok"` when the server is reachable.
    pub status: String,
    /// Crate version string (e.g. `"0.12.0"`).
    pub version: String,
}

// ── Path annotations ──────────────────────────────────────────────────────────
//
// These stubs exist solely to carry the `#[utoipa::path]` attribute so the
// macro can register the operation. They are never called at runtime — the real
// handlers live in `routes/health.rs`, `routes/auth.rs`, and
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
        (status = 200, description = "Active governance policy", body = crate::models::governance::GovernancePolicyResponse),
        (status = 201, description = "Default policy seeded (first call on empty DB)", body = crate::models::governance::GovernancePolicyResponse),
        (status = 401, description = "Missing or invalid token")
    )
)]
#[allow(dead_code)]
pub fn governance_active_stub() {}

/// Reload the in-memory governance policy from the database (admin only).
#[utoipa::path(
    post,
    path = "/api/governance/policy/reload",
    tag = "governance",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Policy reloaded successfully", body = crate::models::governance::GovernanceReloadResponse),
        (status = 401, description = "Missing or invalid token"),
        (status = 403, description = "Admin role required")
    )
)]
#[allow(dead_code)]
pub fn governance_reload_stub() {}

/// List recent governance policy audit-log events (admin only).
#[utoipa::path(
    get,
    path = "/api/governance/policy/events",
    tag = "governance",
    security(("bearerAuth" = [])),
    params(
        ("limit" = Option<i64>, Query, description = "Maximum number of events to return (default 50, max 500)")
    ),
    responses(
        (status = 200, description = "List of policy events, newest first", body = Vec<crate::models::governance::PolicyEvent>),
        (status = 401, description = "Missing or invalid token"),
        (status = 403, description = "Admin role required")
    )
)]
#[allow(dead_code)]
pub fn governance_events_stub() {}

// ── Aggregator ─────────────────────────────────────────────────────────────────

/// OpenAPI document covering the governance, auth, and health endpoints.
///
/// Constructed entirely from types — no runtime state involved.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "EdgePlane Tower API",
        version = "0.1.0",
        description = "EdgePlane Tower — HTTP API"
    ),
    paths(
        health_stub,
        auth_me_stub,
        governance_active_stub,
        governance_reload_stub,
        governance_events_stub,
    ),
    components(
        schemas(
            HealthResponse,
            crate::models::governance::GovernancePolicyResponse,
            crate::models::governance::GovernanceReloadResponse,
            crate::models::governance::PolicyEvent,
            crate::models::governance::PolicyDoc,
            crate::models::governance::PolicyGlobal,
            crate::models::governance::PolicyTerminal,
            crate::models::governance::PolicyMcp,
            crate::models::governance::PolicyActionRule,
            crate::models::auth::MeResponse,
        )
    ),
    security(
        ("bearerAuth" = [])
    ),
    tags(
        (name = "system",     description = "Infrastructure endpoints"),
        (name = "auth",       description = "Authentication and identity"),
        (name = "governance", description = "Policy lifecycle management"),
    )
)]
pub struct ApiDoc;
