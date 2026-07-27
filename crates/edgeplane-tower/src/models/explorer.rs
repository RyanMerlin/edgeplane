//! Typed DTOs for the explorer tree and node-detail endpoints.
//!
//! Both `GET /api/explorer/tree` and `GET /api/explorer/node/{type}/{id}` build
//! their responses via `serde_json::json!` macros inline. The structs here are
//! **mirror DTOs** — they replicate the wire shape for schema generation only.
//!
//! MIRROR — explorer/node/{type}/{id}:
//! The response shape is heterogeneous by `node_type` (`domain`, `mission`,
//! `task`). A proper typed representation would require a serde-tagged enum
//! or three separate paths. Forcing that here would add significant complexity
//! with no wire-compatibility benefit, so we use a single `ExplorerNodeDetail`
//! with all optional fields instead. Fields that are always absent for a given
//! node_type will be `null` in the response — the TypeScript consumer already
//! handles this with optional chaining.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// A task summary inside an explorer tree node.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExplorerTaskSummary {
    /// `character varying` post migration 0014 (task/meshtask unification) — was `i32`.
    pub id: String,
    pub mission_id: String,
    pub title: String,
    pub status: String,
    pub owner: Option<String>,
    #[schema(value_type = String)]
    pub updated_at: chrono::NaiveDateTime,
}

/// A mission (workstream) node inside the explorer tree.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExplorerMissionNode {
    pub id: String,
    pub domain_id: Option<String>,
    pub name: String,
    pub description: String,
    pub status: String,
    pub owners: String,
    pub tags: Option<String>,
    #[schema(value_type = String)]
    pub updated_at: chrono::NaiveDateTime,
    /// Total visible task count for this mission.
    pub task_count: usize,
    /// Per-status task counts (`"open"`, `"done"`, etc.).
    pub task_status_counts: HashMap<String, i64>,
    /// Most recently updated tasks (up to `limit_tasks_per_cluster`).
    pub recent_tasks: Vec<ExplorerTaskSummary>,
}

/// A domain node in the explorer tree (top-level container).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExplorerDomainNode {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub visibility: String,
    pub owners: String,
    pub tags: Option<String>,
    #[schema(value_type = String)]
    pub updated_at: chrono::NaiveDateTime,
    /// Number of missions in this domain (after filters).
    pub mission_count: usize,
    /// Total task count across all missions (after filters).
    pub task_count: usize,
    pub missions: Vec<ExplorerMissionNode>,
}

/// The explorer tree response (`GET /api/explorer/tree`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExplorerTreeResponse {
    #[schema(value_type = String)]
    pub generated_at: chrono::NaiveDateTime,
    pub domain_count: usize,
    pub mission_count: usize,
    pub task_count: usize,
    /// Domains (each containing their missions and task summaries).
    pub domains: Vec<ExplorerDomainNode>,
    /// Missions not assigned to any domain.
    pub unassigned_missions: Vec<ExplorerMissionNode>,
}

// ── Node-detail types ─────────────────────────────────────────────────────────
//
// MIRROR — heterogeneous shape by node_type. Using a flat struct with all
// optional fields rather than a serde-tagged enum to avoid custom
// deserializer complexity. The TypeScript consumer uses optional chaining.

/// A domain detail record (nested inside ExplorerNodeDetail).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExplorerDomain {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub visibility: String,
    pub owners: String,
    pub contributors: String,
    pub tags: Option<String>,
    #[schema(value_type = String)]
    pub updated_at: chrono::NaiveDateTime,
}

/// A mission detail record (nested inside ExplorerNodeDetail).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExplorerMission {
    pub id: String,
    pub domain_id: Option<String>,
    pub name: String,
    pub description: String,
    pub status: String,
    pub owners: String,
    pub tags: Option<String>,
    #[schema(value_type = String)]
    pub updated_at: chrono::NaiveDateTime,
    #[schema(value_type = String)]
    pub created_at: chrono::NaiveDateTime,
}

/// A task detail record (nested inside ExplorerNodeDetail).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExplorerTask {
    /// `character varying` post migration 0014 (task/meshtask unification) — was `i32`.
    pub id: String,
    pub public_id: String,
    pub mission_id: String,
    /// `kind` discriminator ('assigned' | 'claimable') — new column, exposed
    /// rather than filtered at the SQL layer (see routes/explorer.rs).
    pub kind: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub owner: String,
    pub contributors: String,
    #[schema(value_type = String)]
    pub updated_at: chrono::NaiveDateTime,
    #[schema(value_type = String)]
    pub created_at: chrono::NaiveDateTime,
}

/// Response from `GET /api/explorer/node/{node_type}/{node_id}`.
///
/// Shape varies by `node_type`:
/// - `"domain"`: `domain` + `missions[]` + `tasks[]` are populated; `mission` and `task` are absent.
/// - `"mission"`: `domain` (nullable), `mission`, `tasks[]` are populated; `missions` is absent.
/// - `"task"`: `domain` (nullable), `mission`, `task` are populated; `missions` and `tasks` are absent.
///
/// MIRROR: heterogeneous response combined into one struct. Optional fields
/// are `null` when not applicable to the `node_type`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExplorerNodeDetail {
    /// The requested node type: `"domain"`, `"mission"`, or `"task"`.
    pub node_type: String,
    pub node_id: String,
    /// Present for all node types; `null` when the mission/task has no parent domain.
    pub domain: Option<ExplorerDomain>,
    /// Present only when `node_type == "mission"` or `node_type == "task"`.
    pub mission: Option<ExplorerMission>,
    /// Present only when `node_type == "task"`.
    pub task: Option<ExplorerTask>,
    /// Present only when `node_type == "domain"`.
    pub missions: Option<Vec<ExplorerMission>>,
    /// Present when `node_type == "domain"` or `node_type == "mission"`.
    pub tasks: Option<Vec<ExplorerTask>>,
}
