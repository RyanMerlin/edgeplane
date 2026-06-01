//! Typed DTOs for the governance policy API.
//!
//! These structs mirror the JSON shape emitted by the governance handlers and
//! are used both as the OpenAPI schema source (via `ToSchema`) and as the
//! runtime return types from the handlers. This replaces the hand-written
//! mirror in `openapi.rs` and removes the `serde_json::Value` field that
//! caused `policy` to appear as `unknown` in the generated TypeScript types.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ── Nested policy-document types ──────────────────────────────────────────────

/// Per-action rule — controls whether an action is enabled and whether it
/// requires an approval gate before execution.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PolicyActionRule {
    /// Whether the action is allowed at all.
    pub enabled: bool,
    /// Whether the action requires an approval workflow before proceeding.
    pub requires_approval: bool,
}

/// Global policy flags that apply across all actions.
///
/// All fields use `#[serde(default)]` so older policy documents that lack a
/// field deserialize without error (field defaults to `false`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PolicyGlobal {
    #[serde(default)]
    pub require_approval_for_mutations: bool,
    #[serde(default)]
    pub allow_create_without_approval: bool,
    #[serde(default)]
    pub allow_update: bool,
    #[serde(default)]
    pub allow_delete: bool,
    #[serde(default)]
    pub allow_publish: bool,
}

/// Terminal-subsystem flags.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PolicyTerminal {
    #[serde(default)]
    pub allow_create_actions: bool,
    #[serde(default)]
    pub allow_publish_actions: bool,
}

/// MCP-subsystem flags.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PolicyMcp {
    #[serde(default)]
    pub allow_mutation_tools: bool,
}

/// Parsed governance policy document stored in the `policy_json` column.
///
/// All top-level sections are optional so that partial documents (e.g. the
/// seeded default before all subsystems existed) still round-trip cleanly.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PolicyDoc {
    /// Global flags that apply across all entity actions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global: Option<PolicyGlobal>,
    /// Per-action rules keyed by `"<entity>.<verb>"` (e.g. `"domain.create"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<std::collections::HashMap<String, PolicyActionRule>>,
    /// Terminal-subsystem flags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<PolicyTerminal>,
    /// MCP-subsystem flags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<PolicyMcp>,
}

// ── Top-level response types ──────────────────────────────────────────────────

/// Response emitted by `GET /api/governance/policy/active` and related
/// policy-record endpoints.
///
/// The `policy` field is now a concrete typed struct rather than
/// `serde_json::Value`, which allows utoipa to emit a real JSON Schema for the
/// `policy` object and openapi-typescript to generate a typed TypeScript
/// interface instead of `unknown`.
///
/// Timestamp fields use `NaiveDateTime` which matches the existing wire format
/// (chrono serializes without a timezone suffix, preserving byte-identical
/// output vs. the previous `serde_json::json!` construction).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GovernancePolicyResponse {
    /// Database row ID.
    pub id: i32,
    /// Monotonically increasing version number.
    pub version: i32,
    /// Policy lifecycle state: `"active"`, `"draft"`, or `"archived"`.
    pub state: String,
    /// Parsed governance policy document.
    pub policy: PolicyDoc,
    /// Human-readable change note.
    pub change_note: String,
    /// Subject that created this policy record.
    pub created_by: String,
    /// Subject that published this policy (empty string for drafts).
    pub published_by: String,
    /// Timestamp of publication; `null` for drafts.
    #[schema(value_type = Option<String>)]
    pub published_at: Option<NaiveDateTime>,
    /// Timestamp of record creation.
    #[schema(value_type = String)]
    pub created_at: NaiveDateTime,
    /// Timestamp of last update.
    #[schema(value_type = String)]
    pub updated_at: NaiveDateTime,
}

/// Response emitted by `POST /api/governance/policy/reload`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GovernanceReloadResponse {
    /// Always `true` on success.
    pub ok: bool,
}

/// A single governance policy audit-log event.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PolicyEvent {
    /// Database row ID.
    pub id: i32,
    /// ID of the policy record this event refers to; `null` for system events.
    pub policy_id: Option<i32>,
    /// Policy version at the time of the event.
    pub version: i32,
    /// Event type string (e.g. `"seeded"`, `"published"`, `"rollback"`).
    pub event_type: String,
    /// Subject (user or service) that triggered the event.
    pub actor_subject: String,
    /// Arbitrary JSON detail payload (shape varies by event type).
    pub detail: serde_json::Value,
    /// Timestamp of the event.
    #[schema(value_type = String)]
    pub created_at: NaiveDateTime,
}
