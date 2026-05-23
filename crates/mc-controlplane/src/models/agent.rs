use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: i32,
    /// Stable, human-readable identifier — `{name}-{8-char-suffix}`. Used
    /// as the external/wire identity (mcd, CLI, TUI, dashboard). The
    /// numeric `id` stays internal to the database for foreign keys.
    pub public_id: String,
    pub name: String,
    pub capabilities: String,
    pub status: String,
    pub metadata: String,
    /// Permanent home domain — created automatically on first registration.
    pub home_domain_id: Option<String>,
    /// Active domain context. Follows the agent when attached elsewhere;
    /// reset to `home_domain_id` on detach. Joined to `domain_name` in API responses.
    pub current_domain_id: Option<String>,
    /// Human name of `current_domain_id` (joined server-side, not a DB column).
    #[serde(skip_deserializing, default)]
    pub domain_name: Option<String>,
    /// Extracted from `metadata.runtime` (joined server-side, not a DB column).
    #[serde(skip_deserializing, default)]
    pub runtime: Option<String>,
    /// Extracted from `metadata.node_id` (joined server-side, not a DB column).
    #[serde(skip_deserializing, default)]
    pub node_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AgentSession {
    pub id: i32,
    pub agent_id: i32,
    pub context: String,
    pub started_at: NaiveDateTime,
    pub ended_at: Option<NaiveDateTime>,
    pub claude_session_id: Option<String>,
    pub end_reason: Option<String>,
    pub audit_log: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TaskAssignment {
    pub id: i32,
    pub task_id: i32,
    pub agent_id: i32,
    pub status: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AgentMessage {
    pub id: i32,
    pub from_agent_id: i32,
    pub to_agent_id: i32,
    pub content: String,
    pub message_type: String,
    pub task_id: Option<i32>,
    pub read: bool,
    pub created_at: NaiveDateTime,
}

// ── Request shapes ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AgentCreate {
    pub name: String,
    #[serde(default)]
    pub capabilities: String,
    #[serde(default = "default_offline")]
    pub status: String,
    #[serde(default)]
    pub metadata: String,
}

#[derive(Debug, Deserialize)]
pub struct AgentUpdate {
    pub name: Option<String>,
    pub capabilities: Option<String>,
    pub status: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionCreate {
    #[serde(default)]
    pub context: String,
}

#[derive(Debug, Deserialize)]
pub struct AssignmentCreate {
    pub task_id: i32,
    pub agent_id: i32,
    #[serde(default = "default_available")]
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct AssignmentUpdate {
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageSend {
    /// Accepts either the internal numeric id (legacy) or the public_id
    /// string (new mcd path). Resolved to a database row via
    /// `AgentIdent::resolve_id` before persistence.
    pub to_agent_id: AgentIdent,
    pub content: String,
    #[serde(default = "default_info")]
    pub message_type: String,
    pub task_id: Option<i32>,
}

fn default_offline() -> String { "offline".into() }
fn default_available() -> String { "available".into() }
fn default_info() -> String { "info".into() }

// ── AgentIdent ────────────────────────────────────────────────────────────────

/// Wire identifier for an agent. Accepts either the internal numeric id
/// (legacy) or the new `public_id` string in path segments and JSON bodies,
/// so callers can transition incrementally.
///
/// The DB schema still uses `i32` for foreign keys; routes resolve an
/// `AgentIdent` to the underlying numeric id via [`AgentIdent::resolve_id`]
/// before persisting anything.
#[derive(Debug, Clone)]
pub enum AgentIdent {
    Id(i32),
    PublicId(String),
}

impl AgentIdent {
    /// Look up the numeric agent id this identifier resolves to. Returns
    /// `Ok(None)` if no agent matches (the caller renders a 404). Returns
    /// the numeric id directly for the `Id` variant without a DB round-trip.
    pub async fn resolve_id(&self, db: &PgPool) -> Result<Option<i32>, sqlx::Error> {
        match self {
            AgentIdent::Id(id) => Ok(Some(*id)),
            AgentIdent::PublicId(pid) => {
                sqlx::query_scalar::<_, i32>(
                    "SELECT id FROM agent WHERE public_id = $1 AND archived_at IS NULL",
                )
                .bind(pid)
                .fetch_optional(db)
                .await
            }
        }
    }

    /// String form used for error messages and tracing — preserves the
    /// caller's spelling so logs show what the client sent.
    pub fn as_display(&self) -> String {
        match self {
            AgentIdent::Id(id) => id.to_string(),
            AgentIdent::PublicId(pid) => pid.clone(),
        }
    }
}

impl<'de> serde::Deserialize<'de> for AgentIdent {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = AgentIdent;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "an integer agent id or a public_id string")
            }

            // JSON numeric body
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                i32::try_from(v).map(AgentIdent::Id).map_err(|_| E::custom("agent id out of range for i32"))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                i32::try_from(v).map(AgentIdent::Id).map_err(|_| E::custom("agent id out of range for i32"))
            }

            // Path segments arrive as strings; JSON string bodies hit here too.
            // Numeric strings still resolve to `Id` so /agents/7 keeps working.
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(v.parse::<i32>()
                    .map(AgentIdent::Id)
                    .unwrap_or_else(|_| AgentIdent::PublicId(v.to_string())))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(v.parse::<i32>()
                    .map(AgentIdent::Id)
                    .unwrap_or(AgentIdent::PublicId(v)))
            }
        }
        d.deserialize_any(V)
    }
}

impl serde::Serialize for AgentIdent {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            AgentIdent::Id(id) => s.serialize_i32(*id),
            AgentIdent::PublicId(pid) => s.serialize_str(pid),
        }
    }
}

#[cfg(test)]
mod agent_ident_tests {
    use super::AgentIdent;

    #[test]
    fn from_numeric_string() {
        let v: AgentIdent = serde_json::from_value(serde_json::json!("7")).unwrap();
        assert!(matches!(v, AgentIdent::Id(7)));
    }
    #[test]
    fn from_numeric_json() {
        let v: AgentIdent = serde_json::from_value(serde_json::json!(7)).unwrap();
        assert!(matches!(v, AgentIdent::Id(7)));
    }
    #[test]
    fn from_public_id_string() {
        let v: AgentIdent = serde_json::from_value(serde_json::json!("aria-work-qwn5eb33")).unwrap();
        match v {
            AgentIdent::PublicId(s) => assert_eq!(s, "aria-work-qwn5eb33"),
            other => panic!("expected PublicId, got {other:?}"),
        }
    }
    #[test]
    fn name_only_falls_to_public_id() {
        // A bare name without a suffix is still a valid PublicId from the
        // type's perspective — DB lookup decides whether it matches.
        let v: AgentIdent = serde_json::from_value(serde_json::json!("aria-work")).unwrap();
        assert!(matches!(v, AgentIdent::PublicId(_)));
    }
}
