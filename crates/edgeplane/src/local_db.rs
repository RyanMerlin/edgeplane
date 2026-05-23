//! Thin SQLite access for `edgeplane daemon agent` commands in standalone mode.
//!
//! Mirrors the schema in `edgeplaned/src/local_registry.rs`. Both processes open
//! the same `~/.ep/edgeplaned/registry.db` in WAL mode so concurrent access is safe.
//!
//! In standalone mode (no active controlplane profile) the `edgeplane` CLI reads and
//! writes this file directly — no daemon round-trip required. The daemon picks
//! up changes on its next reconcile tick (60s), or immediately if you send a
//! SIGHUP (future: mgmt socket hint from `edgeplane`).

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::config::mc_home_dir;

pub fn db_path() -> std::path::PathBuf {
    mc_home_dir().join("edgeplaned").join("registry.db")
}

/// Returns true if the node has a registered controlplane identity
/// (state file has a `node_id`). Used to switch between standalone and
/// federated CLI paths.
pub fn is_federated() -> bool {
    let state_path = mc_home_dir().join("edgeplaned").join("state.json");
    let Ok(raw) = std::fs::read_to_string(&state_path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    // v1 state: has node_id. v2 (Phase 5b): has active_profile.
    v.get("node_id").and_then(|s| s.as_str()).is_some_and(|s| !s.is_empty())
        || v.get("active_profile").and_then(|s| s.as_str()).is_some_and(|s| !s.is_empty())
}

// ---------- DB access ----------

fn open() -> Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating db dir {}", parent.display()))?;
    }
    let conn = Connection::open(&path)
        .with_context(|| format!("opening local registry {}", path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    ensure_schema(&conn)?;
    Ok(conn)
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY);
         CREATE TABLE IF NOT EXISTS agent (
             id                TEXT NOT NULL,
             source            TEXT NOT NULL,
             domain_id        TEXT NOT NULL,
             runtime_kind      TEXT NOT NULL,
             supervision_mode  TEXT NOT NULL,
             capabilities_json TEXT NOT NULL DEFAULT '[]',
             profile_path      TEXT,
             enrolled_at       TEXT NOT NULL,
             last_synced_at    TEXT,
             PRIMARY KEY (source, id)
         );
         CREATE INDEX IF NOT EXISTS agent_by_source ON agent (source);
         INSERT OR IGNORE INTO schema_version VALUES (1);",
    )?;
    Ok(())
}

// ---------- Operations ----------

pub struct LocalAgent {
    pub id: String,
    pub domain_id: String,
    pub runtime_kind: String,
    pub supervision_mode: String,
    pub capabilities: Vec<String>,
    pub profile_path: Option<String>,
    pub enrolled_at: String,
}

/// Enroll a new agent in standalone mode. Returns the assigned agent ID.
pub fn enroll(
    domain_id: &str,
    runtime_kind: &str,
    supervision_mode: &str,
    capabilities: &[String],
    profile_path: Option<&str>,
) -> Result<String> {
    let conn = open()?;
    let agent_id = uuid::Uuid::new_v4().to_string();
    let enrolled_at = chrono::Utc::now().to_rfc3339();
    let caps_json = serde_json::to_string(capabilities).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO agent
            (id, source, domain_id, runtime_kind, supervision_mode,
             capabilities_json, profile_path, enrolled_at)
         VALUES (?1, 'local', ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            agent_id,
            domain_id,
            runtime_kind,
            supervision_mode,
            caps_json,
            profile_path,
            enrolled_at,
        ],
    )?;
    Ok(agent_id)
}

/// Reassign a local agent to a different domain. Returns false if not found.
pub fn reassign(agent_id: &str, new_domain_id: &str) -> Result<bool> {
    let conn = open()?;
    let n = conn.execute(
        "UPDATE agent SET domain_id = ?1 WHERE source = 'local' AND id = ?2",
        params![new_domain_id, agent_id],
    )?;
    Ok(n > 0)
}

/// Remove a local agent. Returns false if not found.
pub fn unenroll(agent_id: &str) -> Result<bool> {
    let conn = open()?;
    let n = conn.execute(
        "DELETE FROM agent WHERE source = 'local' AND id = ?1",
        params![agent_id],
    )?;
    Ok(n > 0)
}

/// List all locally-enrolled agents, optionally filtered by domain.
pub fn list(domain_filter: Option<&str>) -> Result<Vec<LocalAgent>> {
    let conn = open()?;
    let (sql, param): (&str, Option<&str>) = if let Some(m) = domain_filter {
        (
            "SELECT id, domain_id, runtime_kind, supervision_mode, \
                     capabilities_json, profile_path, enrolled_at \
              FROM agent WHERE source = 'local' AND domain_id = ?1 \
              ORDER BY enrolled_at ASC",
            Some(m),
        )
    } else {
        (
            "SELECT id, domain_id, runtime_kind, supervision_mode, \
                     capabilities_json, profile_path, enrolled_at \
              FROM agent WHERE source = 'local' \
              ORDER BY enrolled_at ASC",
            None,
        )
    };

    let mut stmt = conn.prepare(sql)?;
    let rows: rusqlite::Result<Vec<LocalAgent>> = if let Some(p) = param {
        stmt.query_map(params![p], row_to_agent)?
            .collect()
    } else {
        stmt.query_map([], row_to_agent)?
            .collect()
    };
    rows.context("reading local agents")
}

fn row_to_agent(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalAgent> {
    let caps_json: String = row.get(4)?;
    let capabilities: Vec<String> = serde_json::from_str(&caps_json).unwrap_or_default();
    Ok(LocalAgent {
        id: row.get(0)?,
        domain_id: row.get(1)?,
        runtime_kind: row.get(2)?,
        supervision_mode: row.get(3)?,
        capabilities,
        profile_path: row.get(5)?,
        enrolled_at: row.get(6)?,
    })
}
