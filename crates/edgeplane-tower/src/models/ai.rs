//! Typed DTOs for the AI console API.
//!
//! These structs mirror the JSON shape emitted by the AI session handlers and
//! are used both as the OpenAPI schema source (via `ToSchema`) and as the
//! runtime return types. They replace the hand-written `serde_json::json!()`
//! builders in `routes/ai.rs`.
//!
//! # Wire-compatibility notes
//!
//! * Datetime fields use a **custom serializer** (`ser_naive_dt_micros`) that
//!   emits `%Y-%m-%dT%H:%M:%S%.6f` — matching the explicit `.format(...)` call
//!   that was in the original `json!()` builders. Using `NaiveDateTime`'s
//!   default serde representation would drop the microsecond suffix and break
//!   clients that already consume this API.
//!
//! * `content`, `payload`, `args`, `capability_snapshot`, and `policy` are
//!   **genuinely polymorphic** leaf fields that vary by event type or runtime
//!   kind. They stay as `serde_json::Value` (→ `unknown` in TypeScript), exactly
//!   like `PolicyEvent.detail` in the governance model.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ── Datetime serializer ───────────────────────────────────────────────────────

/// Serialize a `NaiveDateTime` with microsecond precision and no timezone
/// suffix — matches the `%Y-%m-%dT%H:%M:%S%.6f` format previously used in the
/// hand-written `serde_json::json!()` builders.
pub mod ser_naive_dt_micros {
    use chrono::NaiveDateTime;
    use serde::Serializer;

    pub fn serialize<S: Serializer>(dt: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&dt.format("%Y-%m-%dT%H:%M:%S%.6f").to_string())
    }
}

/// Same as `ser_naive_dt_micros` but for `Option<NaiveDateTime>`.
pub mod ser_opt_naive_dt_micros {
    use chrono::NaiveDateTime;
    use serde::Serializer;

    pub fn serialize<S: Serializer>(
        opt: &Option<NaiveDateTime>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match opt {
            Some(dt) => s.serialize_str(&dt.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()),
            None => s.serialize_none(),
        }
    }
}

// ── Sub-structs ───────────────────────────────────────────────────────────────

/// A single conversational turn in an AI session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiTurn {
    /// Row id (stored as i32 in DB, cast to i64 for wire compatibility with
    /// the original `row.get::<i32,_>("id") as i64` conversion).
    pub id: i64,
    /// `"user"`, `"assistant"`, or `"tool"`.
    pub role: String,
    /// Polymorphic turn content — shape varies by role and runtime kind.
    pub content: serde_json::Value,
    /// Turn creation timestamp, microsecond precision, no timezone suffix.
    #[serde(serialize_with = "ser_naive_dt_micros::serialize")]
    #[schema(value_type = String)]
    pub created_at: NaiveDateTime,
}

/// A lifecycle / IO / tool / approval event associated with an AI session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiEvent {
    /// Row id (stored as i32 in DB, cast to i64 for wire compatibility).
    pub id: i64,
    /// Optional parent turn; `null` for session-level events.
    pub turn_id: Option<i32>,
    /// Event type string (e.g. `"user_message"`, `"approval_outcome"`).
    pub event_type: String,
    /// Polymorphic event payload — shape varies by `event_type`.
    pub payload: serde_json::Value,
    /// Event creation timestamp, microsecond precision, no timezone suffix.
    #[serde(serialize_with = "ser_naive_dt_micros::serialize")]
    #[schema(value_type = String)]
    pub created_at: NaiveDateTime,
}

/// A pending tool-execution action waiting for human approval or rejection.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiPendingAction {
    /// Stable string identifier (UUID or custom slug stored in the DB).
    pub id: String,
    /// Tool name (e.g. `"bash"`, `"edit"`).
    pub tool: String,
    /// Polymorphic tool arguments — shape varies by tool.
    pub args: serde_json::Value,
    /// Human-readable reason the runtime requested this action.
    pub reason: String,
    /// Lifecycle status: `"pending"`, `"executed"`, or `"rejected"`.
    pub status: String,
    /// Subject that requested the action.
    pub requested_by: String,
    /// Subject that approved the action (empty string if not yet approved).
    pub approved_by: String,
    /// Subject that rejected the action (empty string if not yet rejected).
    pub rejected_by: String,
    /// Rejection note (empty string if action was not rejected).
    pub rejection_note: String,
    /// Action creation timestamp, microsecond precision, no timezone suffix.
    #[serde(serialize_with = "ser_naive_dt_micros::serialize")]
    #[schema(value_type = String)]
    pub created_at: NaiveDateTime,
    /// Action last-updated timestamp, microsecond precision, no timezone suffix.
    #[serde(serialize_with = "ser_naive_dt_micros::serialize")]
    #[schema(value_type = String)]
    pub updated_at: NaiveDateTime,
}

/// A capability descriptor for a supported AI runtime kind.
///
/// Returned as an array by `GET /api/ai/runtime-capabilities`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CapabilitySet {
    /// Runtime kind identifier (e.g. `"claude_code"`, `"opencode"`).
    pub runtime_kind: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Icon slug used by the frontend asset resolver.
    pub icon_slug: String,
    /// Whether this runtime supports SSE event streaming.
    pub supports_streaming: bool,
    /// Whether this runtime can operate on a file-system workspace.
    pub supports_file_workspace: bool,
    /// Whether tool calls can be intercepted for approval gating.
    pub supports_tool_interception: bool,
    /// Whether edgeplaned skill packs are supported.
    pub supports_skill_packs: bool,
    /// Whether a suspended session can be resumed.
    pub supports_session_resume: bool,
    /// Maximum usable context window in tokens.
    pub max_context_tokens: u32,
}

/// A full AI session record, including nested turns, events, and pending actions.
///
/// Returned by `POST /api/ai/sessions`, `GET /api/ai/sessions/{id}`,
/// `POST /api/ai/sessions/{id}/turns`, and the action approve/reject endpoints.
///
/// `GET /api/ai/sessions` (list) returns a lightweight variant: `turns`,
/// `events`, and `pending_actions` are empty arrays, and `capability_snapshot`
/// and `policy` are `null` (those columns are not fetched in the list query for
/// performance reasons — the wire shape is identical; only the values differ).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiSession {
    /// Stable session identifier (e.g. `"ais_a1b2c3d4e5f6g7h8"`).
    pub id: String,
    /// OIDC subject of the session owner.
    pub owner_subject: String,
    /// Human-readable session title.
    pub title: String,
    /// Lifecycle status: `"active"`, `"completed"`, or `"error"`.
    pub status: String,
    /// Runtime kind used for this session (e.g. `"claude_code"`, `"opencode"`).
    pub runtime_kind: String,
    /// External runtime session identifier (set by the runtime after launch).
    pub runtime_session_id: Option<String>,
    /// Filesystem workspace path used by the runtime, if any.
    pub workspace_path: Option<String>,
    /// Snapshot of the `CapabilitySet` at the time the session was created.
    /// Polymorphic — stays as `Value` (→ `unknown` in TypeScript).
    /// `null` in list responses (not fetched in the list query).
    pub capability_snapshot: Option<serde_json::Value>,
    /// Governance policy document applied to this session.
    /// Polymorphic — stays as `Value` (→ `unknown` in TypeScript).
    /// `null` in list responses (not fetched in the list query).
    pub policy: Option<serde_json::Value>,
    /// Ordered list of turns in this session (empty in list responses).
    pub turns: Vec<AiTurn>,
    /// All events associated with this session (empty in list responses).
    pub events: Vec<AiEvent>,
    /// Pending tool-execution actions awaiting approval (empty in list responses).
    pub pending_actions: Vec<AiPendingAction>,
    /// Session creation timestamp, microsecond precision, no timezone suffix.
    #[serde(serialize_with = "ser_naive_dt_micros::serialize")]
    #[schema(value_type = String)]
    pub created_at: NaiveDateTime,
    /// Session last-updated timestamp, microsecond precision, no timezone suffix.
    #[serde(serialize_with = "ser_naive_dt_micros::serialize")]
    #[schema(value_type = String)]
    pub updated_at: NaiveDateTime,
}

// ── Request bodies ────────────────────────────────────────────────────────────

/// Request body for `POST /api/ai/sessions`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// Optional human-readable session title (defaults to empty string).
    pub title: Option<String>,
    /// Runtime kind to use for this session (defaults to `"opencode"`).
    pub runtime_kind: Option<String>,
    /// Governance policy document to apply to the session.
    /// Polymorphic — stays as `Value`.
    pub policy: Option<serde_json::Value>,
}

/// Request body for `POST /api/ai/sessions/{id}/turns`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PostTurnRequest {
    /// The user message to add as a new turn.
    pub message: String,
}
