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
use serde::Deserialize;
use std::path::Path;

pub fn db_path() -> std::path::PathBuf {
    edgeplaned_paths::registry_db_path()
}

/// Returns true if the node has a registered controlplane identity
/// (state file has a `node_id`). Used to switch between standalone and
/// federated CLI paths.
pub fn is_federated() -> bool {
    let state_path = edgeplaned_paths::state_file_path();
    let Ok(raw) = std::fs::read_to_string(&state_path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    // v1 state: has node_id. v2 (Phase 5b): has active_profile.
    v.get("node_id")
        .and_then(|s| s.as_str())
        .is_some_and(|s| !s.is_empty())
        || v.get("active_profile")
            .and_then(|s| s.as_str())
            .is_some_and(|s| !s.is_empty())
}

// ---------- DB access ----------

fn open() -> Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating db dir {}", parent.display()))?;
    }
    let conn = edgeplaned_paths::open_tuned(&path)
        .with_context(|| format!("opening local registry {}", path.display()))?;
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
         CREATE TABLE IF NOT EXISTS agent_launch_context (
             source          TEXT NOT NULL,
             agent_id        TEXT NOT NULL,
             vault_folder    TEXT,
             state_dir_spec  TEXT,
             zellij_session  TEXT,
             herdr_session   TEXT,
             systemd_service TEXT,
             supervise_paused INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (source, agent_id),
             FOREIGN KEY (source, agent_id) REFERENCES agent (source, id) ON DELETE CASCADE
         );
         INSERT OR IGNORE INTO schema_version VALUES (1);",
    )?;
    Ok(())
}

// ---------- Operations ----------

pub struct LocalAgent {
    pub source: String,
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
            "SELECT source, id, domain_id, runtime_kind, supervision_mode, \
                     capabilities_json, profile_path, enrolled_at \
              FROM agent WHERE domain_id = ?1 \
              ORDER BY source, enrolled_at ASC",
            Some(m),
        )
    } else {
        (
            "SELECT source, id, domain_id, runtime_kind, supervision_mode, \
                     capabilities_json, profile_path, enrolled_at \
              FROM agent \
              ORDER BY source, enrolled_at ASC",
            None,
        )
    };

    let mut stmt = conn.prepare(sql)?;
    let rows: rusqlite::Result<Vec<LocalAgent>> = if let Some(p) = param {
        stmt.query_map(params![p], row_to_agent)?.collect()
    } else {
        stmt.query_map([], row_to_agent)?.collect()
    };
    rows.context("reading local agents")
}

fn row_to_agent(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalAgent> {
    let caps_json: String = row.get(5)?;
    let capabilities: Vec<String> = serde_json::from_str(&caps_json).unwrap_or_default();
    Ok(LocalAgent {
        source: row.get(0)?,
        id: row.get(1)?,
        domain_id: row.get(2)?,
        runtime_kind: row.get(3)?,
        supervision_mode: row.get(4)?,
        capabilities,
        profile_path: row.get(6)?,
        enrolled_at: row.get(7)?,
    })
}

/// Remove agents by source tag. Returns the number of rows deleted.
pub fn unenroll_by_source(source: &str) -> Result<usize> {
    let conn = open()?;
    let n = conn.execute("DELETE FROM agent WHERE source = ?1", params![source])?;
    let _ = conn.execute(
        "DELETE FROM agent_launch_context WHERE source = ?1",
        params![source],
    );
    Ok(n)
}

// ---------- Manifest import ----------

/// One `[[profile]]` block from a TOML agent manifest.
#[derive(Debug, Clone, Deserialize)]
struct ManifestProfile {
    pub name: String,
    /// Only required for `zellij_hosted` runtime. Optional so ACP profiles
    /// can omit it without breaking existing TOML files that still include it.
    pub zellij_session: Option<String>,
    /// Only required for `herdr_hosted` runtime. Optional so profiles on
    /// other runtimes can omit it.
    pub herdr_session: Option<String>,
    /// systemd `--user` unit name (e.g. `my-agent.service`).
    pub service: String,
    pub state_dir: String,
    /// Runtime kind. Defaults to `"zellij_hosted"`.
    /// Set to `"claude_agent_acp"` for supervisor-spawned ACP sessions.
    #[serde(default)]
    pub runtime: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    profile: Vec<ManifestProfile>,
}

/// Summary returned by `import_manifest`.
#[derive(Debug, Default)]
pub struct ImportSummary {
    pub created: usize,
    pub updated: usize,
    pub total: usize,
}

/// Parse a TOML manifest at `path` and upsert each `[[profile]]` into the
/// local registry as a `zellij_hosted` / persistent agent with a matching
/// launch context. Idempotent: keyed on `(source, name)`.
pub fn import_manifest(path: &Path, source: &str) -> Result<ImportSummary> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading manifest: {}", path.display()))?;
    let parsed: ManifestFile =
        toml::from_str(&raw).with_context(|| format!("parsing manifest: {}", path.display()))?;

    let conn = open()?;
    let enrolled_at = chrono::Utc::now().to_rfc3339();

    // Snapshot existing agents under this source tag to distinguish create vs update.
    let existing: std::collections::HashSet<String> = {
        let mut stmt = conn.prepare("SELECT id FROM agent WHERE source = ?1")?;
        let rows: rusqlite::Result<Vec<String>> =
            stmt.query_map(params![source], |row| row.get(0))?.collect();
        rows.context("reading existing agents")?
            .into_iter()
            .collect()
    };

    let mut summary = ImportSummary {
        total: parsed.profile.len(),
        ..Default::default()
    };

    for profile in &parsed.profile {
        let runtime_kind = profile.runtime.as_deref().unwrap_or("zellij_hosted");
        let is_acp = runtime_kind == "claude_agent_acp";
        let is_herdr = runtime_kind == "herdr_hosted";

        // ACP supervisor uses profile_path as cwd so claude loads the right CLAUDE.md.
        let profile_path: Option<&str> = if is_acp {
            Some(&profile.state_dir)
        } else {
            None
        };

        // Upsert agent row.
        conn.execute(
            "INSERT INTO agent
                (id, source, domain_id, runtime_kind, supervision_mode,
                 capabilities_json, profile_path, enrolled_at)
             VALUES (?1, ?2, '', ?3, 'persistent', '[]', ?4, ?5)
             ON CONFLICT(source, id) DO UPDATE SET
                runtime_kind     = excluded.runtime_kind,
                supervision_mode = excluded.supervision_mode,
                profile_path     = excluded.profile_path",
            params![
                profile.name,
                source,
                runtime_kind,
                profile_path,
                enrolled_at
            ],
        )?;

        // Upsert launch context row.
        // state_dir_spec uses internally-tagged format to match edgeplaned-core's
        // StateDirSpec enum: #[serde(tag = "kind", rename_all = "snake_case")]
        let state_dir_spec = serde_json::json!({
            "kind": "persistent", "path": profile.state_dir
        })
        .to_string();

        // ACP and Herdr profiles don't use a zellij_session in their launch
        // context; a profile shouldn't carry both zellij_session and
        // herdr_session.
        let zellij_session: Option<&str> = if is_acp || is_herdr {
            None
        } else {
            profile.zellij_session.as_deref()
        };
        let herdr_session: Option<&str> = if is_herdr {
            profile.herdr_session.as_deref()
        } else {
            None
        };

        conn.execute(
            "INSERT INTO agent_launch_context
                (source, agent_id, vault_folder, state_dir_spec,
                 zellij_session, systemd_service, supervise_paused, herdr_session)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)
             ON CONFLICT(source, agent_id) DO UPDATE SET
                vault_folder    = excluded.vault_folder,
                state_dir_spec  = excluded.state_dir_spec,
                zellij_session  = excluded.zellij_session,
                systemd_service = excluded.systemd_service,
                herdr_session   = excluded.herdr_session",
            params![
                source,
                profile.name,
                profile.name,
                state_dir_spec,
                zellij_session,
                profile.service,
                herdr_session,
            ],
        )?;

        if existing.contains(&profile.name) {
            summary.updated += 1;
        } else {
            summary.created += 1;
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn setup_ep_home(tmp: &TempDir) {
        // SAFETY: tests run single-threaded (--test-threads 1).
        unsafe { std::env::set_var("EP_HOME", tmp.path()) };
    }

    fn teardown() {
        unsafe { std::env::remove_var("EP_HOME") };
    }

    fn sample_manifest(dir: &Path) -> PathBuf {
        let path = dir.join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profile]]
name           = "operator"
zellij_session = "operator"
service        = "test-operator.service"
state_dir      = "/tmp/test-profiles/operator"

[[profile]]
name           = "work"
zellij_session = "work"
service        = "test-work.service"
state_dir      = "/tmp/test-profiles/work"
"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn fresh_db_has_herdr_session_column() {
        let dir = TempDir::new().unwrap();
        let conn = edgeplaned_paths::open_tuned(&dir.path().join("registry.db")).unwrap();
        ensure_schema(&conn).unwrap();
        let mut stmt = conn
            .prepare("PRAGMA table_info(agent_launch_context)")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(
            cols.contains(&"herdr_session".to_string()),
            "columns: {cols:?}"
        );
    }

    #[test]
    fn import_manifest_creates_agents_and_contexts() {
        let tmp = TempDir::new().unwrap();
        setup_ep_home(&tmp);
        let manifest = sample_manifest(tmp.path());

        let summary = import_manifest(&manifest, "test_src").unwrap();
        assert_eq!(summary.created, 2);
        assert_eq!(summary.updated, 0);
        assert_eq!(summary.total, 2);

        let conn = open().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent WHERE source = 'test_src'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);

        let ctx_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_launch_context WHERE source = 'test_src'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ctx_count, 2);

        let spec: String = conn
            .query_row(
                "SELECT state_dir_spec FROM agent_launch_context WHERE agent_id = 'operator'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            spec.contains("\"kind\":\"persistent\""),
            "state_dir_spec should use internally-tagged format, got: {spec}"
        );

        teardown();
    }

    #[test]
    fn import_manifest_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        setup_ep_home(&tmp);
        let manifest = sample_manifest(tmp.path());

        let first = import_manifest(&manifest, "test_src").unwrap();
        assert_eq!(first.created, 2);

        let second = import_manifest(&manifest, "test_src").unwrap();
        assert_eq!(second.created, 0);
        assert_eq!(second.updated, 2);

        let conn = open().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent WHERE source = 'test_src'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 2,
            "idempotent re-import should not create duplicates"
        );

        teardown();
    }

    #[test]
    fn import_manifest_acp_profile_sets_runtime_kind_and_null_session() {
        let tmp = TempDir::new().unwrap();
        setup_ep_home(&tmp);

        let state_dir = tmp.path().join("profiles").join("work");
        let path = tmp.path().join("acp.toml");
        std::fs::write(
            &path,
            format!(
                r#"
[[profile]]
name      = "work"
runtime   = "claude_agent_acp"
service   = "my-agent-work.service"
state_dir = "{}"
"#,
                state_dir.display()
            ),
        )
        .unwrap();

        let summary = import_manifest(&path, "test_acp").unwrap();
        assert_eq!(summary.created, 1);

        let conn = open().unwrap();
        let (runtime_kind, profile_path): (String, Option<String>) = conn
            .query_row(
                "SELECT runtime_kind, profile_path FROM agent WHERE id = 'work'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(runtime_kind, "claude_agent_acp");
        assert_eq!(
            profile_path.as_deref(),
            Some(state_dir.to_string_lossy().as_ref())
        );

        let zellij_session: Option<String> = conn
            .query_row(
                "SELECT zellij_session FROM agent_launch_context WHERE agent_id = 'work'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            zellij_session.is_none(),
            "ACP profile should have no zellij_session"
        );

        teardown();
    }

    #[test]
    fn import_herdr_profile_sets_herdr_session_and_null_zellij() {
        let tmp = TempDir::new().unwrap();
        setup_ep_home(&tmp);

        let manifest = r#"
[[profile]]
name           = "vega"
herdr_session  = "vega"
service        = "aria-vega.service"
state_dir      = "/tmp/test-profiles/vega"
runtime        = "herdr_hosted"
"#;
        let path = tmp.path().join("fleet.toml");
        std::fs::write(&path, manifest).unwrap();

        let summary = import_manifest(&path, "test_herdr").unwrap();
        assert_eq!(summary.created, 1);

        let conn = open().unwrap();
        let runtime_kind: String = conn
            .query_row(
                "SELECT runtime_kind FROM agent WHERE id = 'vega'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(runtime_kind, "herdr_hosted");

        let (herdr_session, zellij_session): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT herdr_session, zellij_session FROM agent_launch_context WHERE agent_id = 'vega'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(herdr_session.as_deref(), Some("vega"));
        assert!(
            zellij_session.is_none(),
            "herdr_hosted profile should have no zellij_session"
        );

        teardown();
    }

    #[test]
    fn unenroll_by_source_removes_all_matching() {
        let tmp = TempDir::new().unwrap();
        setup_ep_home(&tmp);
        let manifest = sample_manifest(tmp.path());

        import_manifest(&manifest, "removable").unwrap();
        let removed = unenroll_by_source("removable").unwrap();
        assert_eq!(removed, 2);

        let conn = open().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent WHERE source = 'removable'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

        teardown();
    }
}
