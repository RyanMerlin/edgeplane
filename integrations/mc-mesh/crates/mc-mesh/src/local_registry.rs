//! Local SQLite agent registry — `~/.mc/mc-mesh.db` (mode 0600).
//!
//! In standalone mode (no active controlplane profile) this is the source of
//! truth for which agents run on this node. In federated mode the controlplane
//! sync loop upserts rows here; the reconciler always reads from here — the
//! registry is the universal substrate under both modes.
//!
//! Schema v1. New columns go through `migrate()` additions.

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

use crate::config::SessionMode;
use crate::daemon::AgentSpec;

/// Source tag for locally-enrolled agents (standalone mode).
pub const SOURCE_LOCAL: &str = "local";

/// Source tag for agents synced from a named controlplane profile.
pub fn source_cp(profile: &str) -> String {
    format!("controlplane:{profile}")
}

// ---------- Registry ----------

pub struct LocalRegistry {
    conn: Connection,
}

impl LocalRegistry {
    /// Default DB path: `~/.mc/mc-mesh.db`.
    pub fn default_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot resolve $HOME"))?;
        Ok(home.join(".mc").join("mc-mesh.db"))
    }

    /// Open (or create) the registry. WAL mode; file chmod'd to 0600.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating db dir {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening registry {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", 1)?;
        Self::migrate(&conn)?;
        set_mode_0600(path)?;
        Ok(Self { conn })
    }

    fn migrate(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY
            );
            CREATE TABLE IF NOT EXISTS agent (
                id                TEXT NOT NULL,
                source            TEXT NOT NULL,
                mission_id        TEXT NOT NULL,
                runtime_kind      TEXT NOT NULL,
                supervision_mode  TEXT NOT NULL,
                capabilities_json TEXT NOT NULL DEFAULT '[]',
                profile_path      TEXT,
                enrolled_at       TEXT NOT NULL,
                last_synced_at    TEXT,
                PRIMARY KEY (source, id)
            );
            CREATE INDEX IF NOT EXISTS agent_by_source ON agent (source);",
        )?;
        conn.execute("INSERT OR IGNORE INTO schema_version VALUES (1)", [])?;
        Ok(())
    }

    /// Insert or update an agent record (upsert on (source, id)).
    pub fn upsert(&self, rec: &AgentRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agent
                (id, source, mission_id, runtime_kind, supervision_mode,
                 capabilities_json, profile_path, enrolled_at, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(source, id) DO UPDATE SET
                mission_id        = excluded.mission_id,
                runtime_kind      = excluded.runtime_kind,
                supervision_mode  = excluded.supervision_mode,
                capabilities_json = excluded.capabilities_json,
                profile_path      = excluded.profile_path,
                last_synced_at    = excluded.last_synced_at",
            params![
                rec.id,
                rec.source,
                rec.mission_id,
                rec.runtime_kind,
                rec.supervision_mode,
                rec.capabilities_json,
                rec.profile_path,
                rec.enrolled_at,
                rec.last_synced_at,
            ],
        )?;
        Ok(())
    }

    /// Reassign an agent to a different mission (updates mission_id in place).
    pub fn reassign(&self, source: &str, agent_id: &str, new_mission_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE agent SET mission_id = ?1 WHERE source = ?2 AND id = ?3",
            params![new_mission_id, source, agent_id],
        )?;
        Ok(n > 0)
    }

    /// Remove an agent from the registry. Returns true if a row was deleted.
    pub fn delete(&self, source: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM agent WHERE source = ?1 AND id = ?2",
            params![source, agent_id],
        )?;
        Ok(n > 0)
    }

    /// List all agent rows for a given source tag.
    pub fn list_by_source(&self, source: &str) -> Result<Vec<AgentRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source, mission_id, runtime_kind, supervision_mode,
                    capabilities_json, profile_path, enrolled_at, last_synced_at
             FROM agent WHERE source = ?1 ORDER BY enrolled_at ASC",
        )?;
        let rows = stmt.query_map(params![source], |row| {
            Ok(AgentRecord {
                id: row.get(0)?,
                source: row.get(1)?,
                mission_id: row.get(2)?,
                runtime_kind: row.get(3)?,
                supervision_mode: row.get(4)?,
                capabilities_json: row.get(5)?,
                profile_path: row.get(6)?,
                enrolled_at: row.get(7)?,
                last_synced_at: row.get(8)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading agent rows from registry")
    }

    /// Convenience: list all records for a source and convert to `AgentSpec`.
    pub fn list_specs_by_source(&self, source: &str) -> Result<Vec<AgentSpec>> {
        let rows = self.list_by_source(source)?;
        Ok(rows.into_iter().map(AgentRecord::into_spec).collect())
    }

    /// Atomically replace all agent records for `source` with `specs`.
    ///
    /// Opens a fresh connection and uses a transaction, making this safe to
    /// call from `tokio::task::spawn_blocking`. WAL mode allows concurrent
    /// reads from the daemon's long-lived connection while this write runs.
    pub fn replace_source(path: &Path, source: &str, specs: &[AgentSpec]) -> Result<()> {
        let mut conn = Connection::open(path)
            .with_context(|| format!("opening registry for replace_source: {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::migrate(&conn)?;
        let now = chrono::Utc::now().to_rfc3339();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM agent WHERE source = ?1", params![source])?;
        for spec in specs {
            let caps = serde_json::to_string(&spec.capabilities).unwrap_or_else(|_| "[]".into());
            let profile = spec
                .profile_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned());
            let supervision = match spec.session_mode {
                SessionMode::Task => "task",
                SessionMode::Persistent => "persistent",
            };
            tx.execute(
                "INSERT INTO agent
                    (id, source, mission_id, runtime_kind, supervision_mode,
                     capabilities_json, profile_path, enrolled_at, last_synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    spec.agent_id,
                    source,
                    spec.mission_id,
                    spec.runtime_kind,
                    supervision,
                    caps,
                    profile,
                    now,
                    now,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

// ---------- AgentRecord ----------

pub struct AgentRecord {
    pub id: String,
    pub source: String,
    pub mission_id: String,
    pub runtime_kind: String,
    /// "task" | "persistent"
    pub supervision_mode: String,
    pub capabilities_json: String,
    pub profile_path: Option<String>,
    pub enrolled_at: String,
    pub last_synced_at: Option<String>,
}

impl AgentRecord {
    /// Build an `AgentRecord` from an in-memory `AgentSpec`. `spec.agent_id`
    /// is now the controlplane-provided `public_id` (post `agent_public_id`
    /// migration; falls back to the legacy mesh UUID if a pre-migration
    /// controlplane is on the other end). This row therefore stores the
    /// stable wire identifier — what mc-mesh uses to poll
    /// `/agents/{id}/messages` and what `mc` CLI passes via `--to-agent-id`.
    pub fn from_spec(spec: &AgentSpec, source: &str) -> Self {
        Self {
            id: spec.agent_id.clone(),
            source: source.to_string(),
            mission_id: spec.mission_id.clone(),
            runtime_kind: spec.runtime_kind.clone(),
            supervision_mode: match spec.session_mode {
                SessionMode::Task => "task".into(),
                SessionMode::Persistent => "persistent".into(),
            },
            capabilities_json: serde_json::to_string(&spec.capabilities)
                .unwrap_or_else(|_| "[]".into()),
            profile_path: spec
                .profile_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            enrolled_at: chrono::Utc::now().to_rfc3339(),
            last_synced_at: None,
        }
    }

    pub fn into_spec(self) -> AgentSpec {
        let session_mode = match self.supervision_mode.as_str() {
            "persistent" => SessionMode::Persistent,
            _ => SessionMode::Task,
        };
        let capabilities: Vec<String> =
            serde_json::from_str(&self.capabilities_json).unwrap_or_default();
        AgentSpec {
            agent_id: self.id,
            mission_id: self.mission_id,
            runtime_kind: self.runtime_kind,
            session_mode,
            capabilities,
            profile_path: self.profile_path.map(PathBuf::from),
        }
    }
}

// ---------- Helpers ----------

#[cfg(unix)]
fn set_mode_0600(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}
#[cfg(not(unix))]
fn set_mode_0600(_path: &Path) -> Result<()> {
    Ok(())
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_reg() -> (TempDir, LocalRegistry) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mc-mesh.db");
        let reg = LocalRegistry::open(&path).unwrap();
        (dir, reg)
    }

    fn spec(id: &str, mission: &str, mode: SessionMode) -> AgentSpec {
        AgentSpec {
            agent_id: id.into(),
            mission_id: mission.into(),
            runtime_kind: "claude_agent_acp".into(),
            session_mode: mode,
            capabilities: vec![],
            profile_path: None,
        }
    }

    #[test]
    fn upsert_and_list() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        let specs = reg.list_specs_by_source(SOURCE_LOCAL).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].agent_id, "a-1");
    }

    #[test]
    fn upsert_is_idempotent() {
        let (_dir, reg) = tmp_reg();
        let rec = AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL);
        reg.upsert(&rec).unwrap();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        assert_eq!(reg.list_by_source(SOURCE_LOCAL).unwrap().len(), 1);
    }

    #[test]
    fn upsert_updates_mission() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-2", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        let specs = reg.list_specs_by_source(SOURCE_LOCAL).unwrap();
        assert_eq!(specs[0].mission_id, "m-2");
    }

    #[test]
    fn reassign_changes_mission() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        let changed = reg.reassign(SOURCE_LOCAL, "a-1", "m-2").unwrap();
        assert!(changed);
        let specs = reg.list_specs_by_source(SOURCE_LOCAL).unwrap();
        assert_eq!(specs[0].mission_id, "m-2");
    }

    #[test]
    fn reassign_returns_false_for_missing_agent() {
        let (_dir, reg) = tmp_reg();
        let changed = reg.reassign(SOURCE_LOCAL, "no-such-agent", "m-2").unwrap();
        assert!(!changed);
    }

    #[test]
    fn delete_removes_agent() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        let removed = reg.delete(SOURCE_LOCAL, "a-1").unwrap();
        assert!(removed);
        assert!(reg.list_specs_by_source(SOURCE_LOCAL).unwrap().is_empty());
    }

    #[test]
    fn delete_returns_false_for_missing_agent() {
        let (_dir, reg) = tmp_reg();
        let removed = reg.delete(SOURCE_LOCAL, "a-1").unwrap();
        assert!(!removed);
    }

    #[test]
    fn source_isolation() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        reg.upsert(&AgentRecord::from_spec(
            &spec("a-1", "m-1", SessionMode::Task),
            &source_cp("work"),
        ))
        .unwrap();
        assert_eq!(reg.list_by_source(SOURCE_LOCAL).unwrap().len(), 1);
        assert_eq!(reg.list_by_source(&source_cp("work")).unwrap().len(), 1);
    }

    #[test]
    fn open_twice_wal_safe() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mc-mesh.db");
        let r1 = LocalRegistry::open(&path).unwrap();
        let r2 = LocalRegistry::open(&path).unwrap();
        r1.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        let specs = r2.list_specs_by_source(SOURCE_LOCAL).unwrap();
        assert_eq!(specs.len(), 1);
    }

    #[test]
    fn persistent_mode_round_trips() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(
            &spec("a-1", "m-1", SessionMode::Persistent),
            SOURCE_LOCAL,
        ))
        .unwrap();
        let specs = reg.list_specs_by_source(SOURCE_LOCAL).unwrap();
        assert!(matches!(specs[0].session_mode, SessionMode::Persistent));
    }

    #[test]
    fn replace_source_atomic_swap() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mc-mesh.db");
        let src = source_cp("homelab");

        // Seed two agents.
        LocalRegistry::replace_source(&path, &src, &[
            spec("a-1", "m-1", SessionMode::Task),
            spec("a-2", "m-1", SessionMode::Task),
        ])
        .unwrap();

        let reg = LocalRegistry::open(&path).unwrap();
        assert_eq!(reg.list_specs_by_source(&src).unwrap().len(), 2);

        // Replace with a single new spec — old records for source are gone.
        LocalRegistry::replace_source(&path, &src, &[spec("a-3", "m-2", SessionMode::Task)])
            .unwrap();
        let after = reg.list_specs_by_source(&src).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].agent_id, "a-3");
    }

    #[test]
    fn replace_source_does_not_touch_other_sources() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mc-mesh.db");
        let src_a = source_cp("profile-a");
        let src_b = source_cp("profile-b");

        LocalRegistry::replace_source(&path, &src_a, &[spec("a-1", "m-1", SessionMode::Task)])
            .unwrap();
        LocalRegistry::replace_source(&path, &src_b, &[spec("b-1", "m-1", SessionMode::Task)])
            .unwrap();

        // Replacing src_a should not remove src_b rows.
        LocalRegistry::replace_source(&path, &src_a, &[]).unwrap();
        let reg = LocalRegistry::open(&path).unwrap();
        assert!(reg.list_specs_by_source(&src_a).unwrap().is_empty());
        assert_eq!(reg.list_specs_by_source(&src_b).unwrap().len(), 1);
    }
}
