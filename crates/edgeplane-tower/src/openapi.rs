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

// ── Governance ────────────────────────────────────────────────────────────────

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
        ("agent_id" = String, Path, description = "Numeric agent id or public_id string (e.g. `aria-work-qwn5eb33`)")
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

// ── AI console ───────────────────────────────────────────────────────────────

/// Return the list of supported AI runtime capabilities.
#[utoipa::path(
    get,
    path = "/api/ai/runtime-capabilities",
    tag = "ai",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "List of supported runtime capability sets", body = Vec<crate::models::ai::CapabilitySet>),
        (status = 401, description = "Missing or invalid token")
    )
)]
#[allow(dead_code)]
pub fn ai_runtime_capabilities_stub() {}

/// List AI sessions owned by the authenticated caller.
#[utoipa::path(
    get,
    path = "/api/ai/sessions",
    tag = "ai",
    security(("bearerAuth" = [])),
    params(
        ("limit" = Option<i64>, Query, description = "Maximum number of sessions to return (default 20, max 100)")
    ),
    responses(
        (status = 200, description = "List of AI sessions (turns/events/pending_actions are empty arrays in list responses)", body = Vec<crate::models::ai::AiSession>),
        (status = 401, description = "Missing or invalid token")
    )
)]
#[allow(dead_code)]
pub fn ai_sessions_list_stub() {}

/// Create a new AI session.
#[utoipa::path(
    post,
    path = "/api/ai/sessions",
    tag = "ai",
    security(("bearerAuth" = [])),
    request_body = crate::models::ai::CreateSessionRequest,
    responses(
        (status = 200, description = "Created AI session with empty turns/events/pending_actions", body = crate::models::ai::AiSession),
        (status = 401, description = "Missing or invalid token")
    )
)]
#[allow(dead_code)]
pub fn ai_sessions_create_stub() {}

/// Fetch a single AI session by id, including all turns, events, and pending actions.
#[utoipa::path(
    get,
    path = "/api/ai/sessions/{id}",
    tag = "ai",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "AI session id (e.g. `ais_a1b2c3d4e5f6g7h8`)")
    ),
    responses(
        (status = 200, description = "Full AI session with nested turns, events, and pending actions", body = crate::models::ai::AiSession),
        (status = 401, description = "Missing or invalid token"),
        (status = 404, description = "Session not found or not owned by caller")
    )
)]
#[allow(dead_code)]
pub fn ai_sessions_get_stub() {}

/// Append a user turn to an AI session.
#[utoipa::path(
    post,
    path = "/api/ai/sessions/{id}/turns",
    tag = "ai",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "AI session id")
    ),
    request_body = crate::models::ai::PostTurnRequest,
    responses(
        (status = 200, description = "Full session after the turn was appended", body = crate::models::ai::AiSession),
        (status = 401, description = "Missing or invalid token"),
        (status = 404, description = "Session not found"),
        (status = 422, description = "Message body is empty")
    )
)]
#[allow(dead_code)]
pub fn ai_sessions_create_turn_stub() {}

/// Approve a pending tool-execution action.
#[utoipa::path(
    post,
    path = "/api/ai/sessions/{id}/actions/{action_id}/approve",
    tag = "ai",
    security(("bearerAuth" = [])),
    params(
        ("id"        = String, Path, description = "AI session id"),
        ("action_id" = String, Path, description = "Pending action id")
    ),
    responses(
        (status = 200, description = "Full session after the action was approved", body = crate::models::ai::AiSession),
        (status = 401, description = "Missing or invalid token"),
        (status = 404, description = "Session or action not found"),
        (status = 422, description = "Action is not in pending status")
    )
)]
#[allow(dead_code)]
pub fn ai_sessions_approve_action_stub() {}

/// Reject a pending tool-execution action.
#[utoipa::path(
    post,
    path = "/api/ai/sessions/{id}/actions/{action_id}/reject",
    tag = "ai",
    security(("bearerAuth" = [])),
    params(
        ("id"        = String, Path, description = "AI session id"),
        ("action_id" = String, Path, description = "Pending action id"),
        ("note"      = Option<String>, Query, description = "Optional rejection note")
    ),
    responses(
        (status = 200, description = "Full session after the action was rejected", body = crate::models::ai::AiSession),
        (status = 401, description = "Missing or invalid token"),
        (status = 404, description = "Session or action not found"),
        (status = 422, description = "Action is not in pending status")
    )
)]
#[allow(dead_code)]
pub fn ai_sessions_reject_action_stub() {}

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
        governance_active_stub,
        governance_reload_stub,
        governance_events_stub,
        agents_list_stub,
        agents_get_stub,
        runtime_nodes_list_stub,
        runtime_node_agents_stub,
        onboarding_manifest_stub,
        explorer_tree_stub,
        explorer_node_stub,
        ai_runtime_capabilities_stub,
        ai_sessions_list_stub,
        ai_sessions_create_stub,
        ai_sessions_get_stub,
        ai_sessions_create_turn_stub,
        ai_sessions_approve_action_stub,
        ai_sessions_reject_action_stub,
    ),
    components(
        schemas(
            HealthResponse,
            // governance
            crate::models::governance::GovernancePolicyResponse,
            crate::models::governance::GovernanceReloadResponse,
            crate::models::governance::PolicyEvent,
            crate::models::governance::PolicyDoc,
            crate::models::governance::PolicyGlobal,
            crate::models::governance::PolicyTerminal,
            crate::models::governance::PolicyMcp,
            crate::models::governance::PolicyActionRule,
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
            // ai console
            crate::models::ai::CapabilitySet,
            crate::models::ai::AiSession,
            crate::models::ai::AiTurn,
            crate::models::ai::AiEvent,
            crate::models::ai::AiPendingAction,
            crate::models::ai::CreateSessionRequest,
            crate::models::ai::PostTurnRequest,
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
        (name = "governance", description = "Policy lifecycle management"),
        (name = "agents",     description = "Control-plane agent registry"),
        (name = "runtime",    description = "Runtime node fleet management"),
        (name = "explorer",   description = "Domain / mission / task explorer tree"),
        (name = "ai",         description = "AI session management console"),
    )
)]
pub struct ApiDoc;
