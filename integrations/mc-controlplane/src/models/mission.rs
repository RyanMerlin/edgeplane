use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Mission {
    pub id: String,
    pub name: String,
    pub description: String,
    pub owners: String,
    pub contributors: String,
    pub tags: String,
    pub visibility: String,
    pub status: String,
    pub northstar_md: String,
    pub northstar_version: i32,
    pub northstar_created_by: String,
    pub northstar_modified_by: String,
    pub northstar_created_at: Option<NaiveDateTime>,
    pub northstar_modified_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MissionRoleMembership {
    pub id: i32,
    pub mission_id: String,
    pub subject: String,
    pub role: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

// ── Request/response shapes ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MissionCreate {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub owners: String,
    #[serde(default)]
    pub contributors: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default = "default_public")]
    pub visibility: String,
    #[serde(default = "default_active")]
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct MissionUpdate {
    pub description: Option<String>,
    pub owners: Option<String>,
    pub contributors: Option<String>,
    pub tags: Option<String>,
    pub visibility: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MissionRoleUpsert {
    pub subject: String,
    pub role: String,
}

fn default_public() -> String { "public".into() }
fn default_active() -> String { "active".into() }
