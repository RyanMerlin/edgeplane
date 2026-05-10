//! Hand-rolled wire types for ACP discriminated unions.
//!
//! The vendored ACP schema uses OpenAPI-style `discriminator` keywords on
//! [`ContentBlock`] and [`SessionUpdate`]. typify 0.6 doesn't fully translate
//! that into named serde-tagged enums — its output names variants
//! `Variant0`, `Variant1`, ... and inlines the discriminator field. We
//! provide cleaner enums here that serialize/deserialize the same wire
//! format.
//!
//! Inner data types (e.g. [`crate::schema::TextContent`]) are still
//! typify-generated, so this overlay stays thin.
//!
//! When upstream adds new variants:
//! - [`ContentBlock`] will fail to deserialize unknown `type` values; the
//!   actor logs and drops the malformed message.
//! - [`SessionUpdate`] catches every known kind today; new kinds added by
//!   upstream will fail to deserialize, again surfaced via the actor's
//!   tracing path. Fix is mechanical: add the variant here.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::schema;

/// `session/prompt` content blocks. Discriminated by `type` on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text(schema::TextContent),
    Image(schema::ImageContent),
    Audio(schema::AudioContent),
    ResourceLink(schema::ResourceLink),
    Resource(schema::EmbeddedResource),
}

impl ContentBlock {
    /// Convenience: build a plain text block.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(schema::TextContent {
            annotations: None,
            meta: None,
            text: text.into(),
        })
    }

    /// If this is a text block, return its text. Useful for tests and
    /// supervisors that just want streamed assistant output.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(t) => Some(&t.text),
            _ => None,
        }
    }
}

/// `session/update` notification payload. Discriminated by `sessionUpdate`
/// on the wire.
///
/// The three "chunk" variants (assistant message, thought, user echo) get
/// typed inner shapes since they're the hot path. Tool-call, plan, and
/// metadata variants land as raw JSON until a caller needs them; promote
/// to typed shapes lazily when consumers ask for them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    UserMessageChunk {
        content: ContentBlock,
    },
    AgentMessageChunk {
        content: ContentBlock,
    },
    AgentThoughtChunk {
        content: ContentBlock,
    },
    /// A tool call started or progressed. Raw JSON for now.
    ToolCall(Value),
    /// An update to a previously-emitted tool call. Raw JSON.
    ToolCallUpdate(Value),
    /// Plan entries. Raw JSON.
    Plan(Value),
    /// Available slash-commands listing (sent unsolicited at session start
    /// by claude-code-acp).
    AvailableCommandsUpdate(Value),
    /// Mode changes.
    CurrentModeUpdate(Value),
    /// Per-session config options surfaced by the agent.
    ConfigOptionUpdate(Value),
    /// Session metadata updates (e.g. title).
    SessionInfoUpdate(Value),
    /// Token / cost usage telemetry.
    UsageUpdate(Value),
}

/// `session/update` envelope sent by the agent during a prompt turn.
///
/// Wire format:
/// ```json
/// {
///   "sessionId": "uuid",
///   "update": { "sessionUpdate": "agent_message_chunk", "content": {...} }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionNotification {
    #[serde(rename = "sessionId")]
    pub session_id: schema::SessionId,
    pub update: SessionUpdate,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Map<String, Value>>,
}
