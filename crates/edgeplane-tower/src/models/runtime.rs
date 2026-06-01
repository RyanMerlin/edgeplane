//! Typed DTOs for the runtime/nodes API.
//!
//! `list_nodes` and `list_node_agents` both return `serde_json::Value` built
//! by `row_to_node` / `routes::work::row_to_agent`. The structs here are
//! **mirror DTOs** — they match the wire shape exactly but the handlers were
//! not converted (they rely on dynamic JSON construction from multi-join rows).
//!
//! MIRROR: needs real DTO conversion if any handler is refactored to return a
//! typed struct. Until then these exist solely to give openapi-typescript a
//! typed schema so the generated client covers the fleet/runtime screens.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A registered runtime node. Returned by `GET /api/runtime/nodes`.
///
/// MIRROR: handler (`routes/runtime.rs::list_nodes`) still builds JSON via
/// `row_to_node`. Field set matches `row_to_node` exactly; JSON-embedded
/// fields (`labels`, `capacity`, `capabilities`) stay as `serde_json::Value`
/// because their schema is caller-defined.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RuntimeNode {
    pub id: String,
    pub owner_subject: String,
    pub node_name: String,
    pub hostname: String,
    /// Node lifecycle status: `"registered"`, `"online"`, `"offline"`, `"cordoned"`, `"draining"`.
    pub status: String,
    /// Trust tier: `"untrusted"`, `"trusted"`, `"admin"`.
    pub trust_tier: String,
    /// Free-form key/value labels (JSON object).
    pub labels: serde_json::Value,
    /// Capacity declarations — CPU, memory, etc. (JSON object).
    pub capacity: serde_json::Value,
    /// List of capability strings the node advertises (JSON array of strings).
    pub capabilities: serde_json::Value,
    pub runtime_version: String,
    pub tailscale_ip: Option<String>,
    pub tailscale_fqdn: Option<String>,
    #[schema(value_type = Option<String>)]
    pub last_heartbeat_at: Option<chrono::NaiveDateTime>,
    #[schema(value_type = String)]
    pub registered_at: chrono::NaiveDateTime,
    #[schema(value_type = String)]
    pub updated_at: chrono::NaiveDateTime,
}

/// A mesh agent assigned to a runtime node. Returned by
/// `GET /api/runtime/nodes/{node_id}/agents`.
///
/// MIRROR: handler (`routes/runtime.rs::list_node_agents`) builds JSON via
/// `routes::work::row_to_agent` + domain join. Fields with open schemas
/// (`profile`, `machine`, `runtime`, `labels`, `capabilities`) stay as
/// `serde_json::Value` — their shape is runtime-class dependent.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NodeMeshAgent {
    /// Meshagent row UUID.
    pub id: String,
    /// Stable wire identifier (agent public_id or meshagent id as fallback).
    pub public_id: String,
    /// The linked persistent agent identity, if set at enrollment.
    pub agent_public_id: Option<String>,
    pub domain_id: String,
    pub node_id: Option<String>,
    pub runtime_kind: String,
    pub runtime_version: String,
    /// List of capability strings (JSON array).
    pub capabilities: serde_json::Value,
    /// Scheduling/placement labels (JSON object).
    pub labels: serde_json::Value,
    /// Agent status: `"online"`, `"offline"`, `"busy"`.
    pub status: String,
    pub current_task_id: Option<String>,
    #[schema(value_type = String)]
    pub enrolled_at: chrono::NaiveDateTime,
    #[schema(value_type = Option<String>)]
    pub last_heartbeat_at: Option<chrono::NaiveDateTime>,
    pub runtime_node_id: Option<String>,
    /// Supervision mode: `"task"` or `"persistent"`.
    pub supervision_mode: Option<String>,
    /// Runtime-class profile blob (shape varies by runtime_kind).
    pub profile: Option<serde_json::Value>,
    /// Machine descriptor blob (shape varies by runtime_kind).
    pub machine: Option<serde_json::Value>,
    /// Runtime config blob (shape varies by runtime_kind).
    pub runtime: Option<serde_json::Value>,
    /// Capabilities discovered at runtime (JSON array).
    pub discovered_capabilities: serde_json::Value,
    /// Human name of the assigned domain (joined server-side).
    pub domain_name: Option<String>,
    /// Kind of the assigned domain (joined server-side).
    pub domain_kind: Option<String>,
}
