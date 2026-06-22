//! OpenAPI aggregator — DB-free spec generation.
//!
//! Compiles and runs without constructing AppState, opening a database
//! connection, reading environment secrets, or binding a network socket.

use serde::Serialize;
use utoipa::{OpenApi, ToSchema};

// ── Schema-only DTO ───────────────────────────────────────────────────────────

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
// handlers live in `routes/`. The `gen-openapi` binary only calls
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

// ── Agents ────────────────────────────────────────────────────────────────────

/// List all registered control-plane agents.
#[utoipa::path(
    get,
    path = "/api/agents",
    tag = "agents",
    security(("bearerAuth" = [])),
    params(
        ("status"           = Option<String>, Query, description = "Filter by agent status (e.g. `online`, `offline`)"),
        ("limit"            = Option<i64>,    Query, description = "Maximum number of results (default 100, max 500)"),
        ("include_archived" = Option<bool>,   Query, description = "Include archived agents (default false)")
    ),
    responses(
        (status = 200, description = "List of agents", body = Vec<crate::models::agent::Agent>),
        (status = 401, description = "Missing or invalid token")
    )
)]
#[allow(dead_code)]
pub fn agents_list_stub() {}

/// Return a single agent by numeric id or public_id.
#[utoipa::path(
    get,
    path = "/api/agents/{agent_id}",
    tag = "agents",
    security(("bearerAuth" = [])),
    params(
        ("agent_id" = String, Path, description = "Numeric agent id or public_id string (e.g. `my-agent-work-qwn5eb33`)")
    ),
    responses(
        (status = 200, description = "Agent record", body = crate::models::agent::Agent),
        (status = 401, description = "Missing or invalid token"),
        (status = 404, description = "Agent not found")
    )
)]
#[allow(dead_code)]
pub fn agents_get_stub() {}

// ── Runtime / fleet ───────────────────────────────────────────────────────────

/// List runtime nodes registered by the authenticated subject.
#[utoipa::path(
    get,
    path = "/api/runtime/nodes",
    tag = "runtime",
    security(("bearerAuth" = [])),
    params(
        ("status" = Option<String>, Query, description = "Filter by node status"),
        ("limit"  = Option<i64>,   Query, description = "Maximum number of results (default 100, max 500)")
    ),
    responses(
        (status = 200, description = "List of runtime nodes", body = Vec<crate::models::runtime::RuntimeNode>),
        (status = 401, description = "Missing or invalid token")
    )
)]
#[allow(dead_code)]
pub fn runtime_nodes_list_stub() {}

/// List mesh agents assigned to a specific runtime node.
#[utoipa::path(
    get,
    path = "/api/runtime/nodes/{node_id}/agents",
    tag = "runtime",
    security(("bearerAuth" = [])),
    params(
        ("node_id" = String, Path, description = "Runtime node UUID")
    ),
    responses(
        (status = 200, description = "List of mesh agents on this node", body = Vec<crate::models::runtime::NodeMeshAgent>),
        (status = 401, description = "Missing or invalid token"),
        (status = 403, description = "Node does not belong to the caller"),
        (status = 404, description = "Node not found")
    )
)]
#[allow(dead_code)]
pub fn runtime_node_agents_stub() {}

/// Delete a runtime node, detaching its agents and revoking its credentials.
#[utoipa::path(
    delete,
    path = "/api/runtime/nodes/{node_id}",
    tag = "runtime",
    security(("bearerAuth" = [])),
    params(
        ("node_id" = String, Path, description = "Runtime node UUID"),
        ("force" = Option<bool>, Query, description = "Detach assigned agents and delete anyway")
    ),
    responses(
        (status = 200, description = "Node deleted; agents detached and tokens revoked"),
        (status = 401, description = "Missing or invalid token"),
        (status = 403, description = "Node does not belong to the caller"),
        (status = 404, description = "Node not found"),
        (status = 409, description = "Node has assigned agents; pass force=true to detach and delete")
    )
)]
#[allow(dead_code)]
pub fn runtime_node_delete_stub() {}

// ── Onboarding ────────────────────────────────────────────────────────────────

/// Return the agent onboarding manifest for this EdgePlane instance.
///
/// The manifest describes all integration endpoints, MCP server configuration,
/// and bootstrap steps needed to connect an agent runtime (edgeplaned) to this
/// controlplane. No authentication required.
#[utoipa::path(
    get,
    path = "/api/agent-onboarding.json",
    tag = "system",
    params(
        ("endpoint" = Option<String>, Query, description = "Base URL to embed in the manifest (defaults to `Host` header)")
    ),
    responses(
        (status = 200, description = "Onboarding manifest", body = crate::models::onboarding::OnboardingManifest)
    )
)]
#[allow(dead_code)]
pub fn onboarding_manifest_stub() {}

// ── Explorer ──────────────────────────────────────────────────────────────────

/// Return the full explorer tree — domains → missions → task summaries.
///
/// Supports text search (`q`), domain/status filtering, and per-cluster task limits.
#[utoipa::path(
    get,
    path = "/api/explorer/tree",
    tag = "explorer",
    security(("bearerAuth" = [])),
    params(
        ("domain_id"              = Option<String>, Query, description = "Restrict to a single domain"),
        ("status"                 = Option<String>, Query, description = "Filter tasks by status"),
        ("q"                      = Option<String>, Query, description = "Full-text filter across domain/mission/task names, descriptions, owners, and tags"),
        ("limit_tasks_per_cluster"= Option<i64>,    Query, description = "Max tasks per mission (default 5, max 50)"),
        ("limit_missions"          = Option<i64>,   Query, description = "Max missions fetched (default 100, max 200)")
    ),
    responses(
        (status = 200, description = "Explorer tree", body = crate::models::explorer::ExplorerTreeResponse),
        (status = 401, description = "Missing or invalid token")
    )
)]
#[allow(dead_code)]
pub fn explorer_tree_stub() {}

/// Return full detail for a specific explorer node (domain, mission, or task).
///
/// The response shape varies by `node_type`; see `ExplorerNodeDetail` for field
/// presence rules by type.
#[utoipa::path(
    get,
    path = "/api/explorer/node/{node_type}/{node_id}",
    tag = "explorer",
    security(("bearerAuth" = [])),
    params(
        ("node_type" = String, Path, description = "One of: `domain`, `mission`, `task`"),
        ("node_id"   = String, Path, description = "Entity id (UUID for domain/mission; numeric or public_id for task)"),
        ("limit_tasks" = Option<i64>, Query, description = "Max tasks to return (default 50, max 200)")
    ),
    responses(
        (status = 200, description = "Node detail", body = crate::models::explorer::ExplorerNodeDetail),
        (status = 400, description = "Invalid node_type"),
        (status = 401, description = "Missing or invalid token"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Node not found")
    )
)]
#[allow(dead_code)]
pub fn explorer_node_stub() {}

// ── Aggregator ─────────────────────────────────────────────────────────────────

/// OpenAPI document covering the full read-only API surface consumed by the
/// React v2 frontend.
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
        agents_list_stub,
        agents_get_stub,
        runtime_nodes_list_stub,
        runtime_node_agents_stub,
        runtime_node_delete_stub,
        onboarding_manifest_stub,
        explorer_tree_stub,
        explorer_node_stub,
    ),
    components(
        schemas(
            HealthResponse,
            // auth
            crate::models::auth::MeResponse,
            // agents
            crate::models::agent::Agent,
            crate::models::agent::AgentSession,
            crate::models::agent::AgentMessage,
            crate::models::agent::TaskAssignment,
            // runtime
            crate::models::runtime::RuntimeNode,
            crate::models::runtime::NodeMeshAgent,
            // onboarding
            crate::models::onboarding::OnboardingManifest,
            crate::models::onboarding::OnboardingEndpoints,
            crate::models::onboarding::McpServerConfig,
            crate::models::onboarding::McpDefaults,
            crate::models::onboarding::OnboardingBootstrap,
            crate::models::onboarding::OnboardingAutomation,
            // explorer
            crate::models::explorer::ExplorerTreeResponse,
            crate::models::explorer::ExplorerDomainNode,
            crate::models::explorer::ExplorerMissionNode,
            crate::models::explorer::ExplorerTaskSummary,
            crate::models::explorer::ExplorerNodeDetail,
            crate::models::explorer::ExplorerDomain,
            crate::models::explorer::ExplorerMission,
            crate::models::explorer::ExplorerTask,
        )
    ),
    security(
        ("bearerAuth" = [])
    ),
    tags(
        (name = "system",     description = "Infrastructure endpoints"),
        (name = "auth",       description = "Authentication and identity"),
        (name = "agents",     description = "Control-plane agent registry"),
        (name = "runtime",    description = "Runtime node fleet management"),
        (name = "explorer",   description = "Domain / mission / task explorer tree"),
    )
)]
pub struct ApiDoc;
