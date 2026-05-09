use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Kluster {
    pub id: String,
    pub mission_id: Option<String>,
    pub name: String,
    pub description: String,
    pub owners: String,
    pub contributors: String,
    pub tags: String,
    pub status: String,
    pub workstream_md: String,
    pub workstream_version: i32,
    pub workstream_created_by: String,
    pub workstream_modified_by: String,
    pub workstream_created_at: Option<NaiveDateTime>,
    pub workstream_modified_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct KlusterCreate {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub owners: String,
    #[serde(default)]
    pub contributors: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default = "default_active")]
    pub status: String,
    pub mission_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct KlusterUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub owners: Option<String>,
    pub contributors: Option<String>,
    pub tags: Option<String>,
    pub status: Option<String>,
}

fn default_active() -> String { "active".into() }
