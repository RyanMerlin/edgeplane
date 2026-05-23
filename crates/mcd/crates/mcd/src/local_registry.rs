//! Local SQLite agent registry — `~/.mc/mcd/registry.db` (mode 0600).
//!
//! In standalone mode (no active controlplane profile) this is the source of
//! truth for which agents run on this node. In federated mode the controlplane
//! sync loop upserts rows here; the reconciler always reads from here — the
//! registry is the universal substrate under both modes.
//!
//! Schema v1. New columns go through `migrate()` additions.

use anyhow::{Context, Result};
use mcd_core::types::StateDirSpec;
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

// ---------- Schema migrations ----------
//
// Versioned forward-only migrations. The current state of an installation
// is recorded as a single row in `schema_version`. On `LocalRegistry::open`
// we walk from whatever version is stamped up to `CURRENT_SCHEMA_VERSION`,
// applying each step in turn. Adding a new migration:
//   1. Increment `CURRENT_SCHEMA_VERSION`.
//   2. Add a new `migrate_to_vN(conn)` function with the DDL.
//   3. Wire it into `apply_migrations` after the prior step.
// Each step runs in a transaction; a panic or error inside leaves the DB
// at the previous version, never half-applied.
//
// Version map:
//   v1 → Phase 1+4 baseline: agent, agent_launch_context, agent_cron_state,
//        agent_cron_fire_log + their indexes.
//   v2 → Phase 5: agent_launch_context gains systemd_service +
//        supervise_paused columns; new unit_restart_log table.

const CURRENT_SCHEMA_VERSION: u32 = 2;

fn apply_migrations(conn: &Connection) -> Result<()> {
    // Bootstrap the version table itself, idempotent.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        );",
    )?;
    let current = read_schema_version(conn)?;
    if current >= CURRENT_SCHEMA_VERSION {
        return Ok(());
    }
    if current < 1 {
        migrate_to_v1(conn).context("schema migration to v1")?;
    }
    if current < 2 {
        migrate_to_v2(conn).context("schema migration to v2")?;
    }
    // Stamp the resulting version. Use DELETE+INSERT so multiple stamps
    // (e.g. from earlier `INSERT OR IGNORE VALUES (1)` callers) collapse
    // to one row.
    conn.execute("DELETE FROM schema_version", [])?;
    conn.execute("INSERT INTO schema_version VALUES (?1)", params![CURRENT_SCHEMA_VERSION])?;
    Ok(())
}

fn read_schema_version(conn: &Connection) -> Result<u32> {
    let mut stmt = match conn.prepare("SELECT MAX(version) FROM schema_version") {
        Ok(s) => s,
        Err(rusqlite::Error::SqliteFailure(_, Some(ref m))) if m.contains("no such table") => {
            return Ok(0);
        }
        Err(e) => return Err(e.into()),
    };
    let v: Option<u32> = stmt
        .query_row([], |row| row.get::<_, Option<u32>>(0))
        .unwrap_or(None);
    Ok(v.unwrap_or(0))
}

fn migrate_to_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent (
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
            PRIMARY KEY (source, agent_id),
            FOREIGN KEY (source, agent_id)
                REFERENCES agent (source, id)
                ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS launch_context_by_source
            ON agent_launch_context (source);
        CREATE TABLE IF NOT EXISTS agent_cron_state (
            job_name        TEXT PRIMARY KEY,
            last_fired_at   TEXT,
            last_status     TEXT,
            last_error      TEXT,
            updated_at      TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS agent_cron_fire_log (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            job_name        TEXT NOT NULL,
            fired_at        TEXT NOT NULL,
            status          TEXT NOT NULL,
            duration_ms     INTEGER,
            error_message   TEXT
        );
        CREATE INDEX IF NOT EXISTS fire_log_by_job_time
            ON agent_cron_fire_log (job_name, fired_at DESC);",
    )?;
    Ok(())
}

fn migrate_to_v2(conn: &Connection) -> Result<()> {
    // ALTER TABLE ADD COLUMN is not idempotent in SQLite — running twice
    // returns "duplicate column" error. Guard each ADD by checking
    // PRAGMA table_info first.
    add_column_if_missing(
        conn,
        "agent_launch_context",
        "systemd_service",
        "TEXT",
    )?;
    add_column_if_missing(
        conn,
        "agent_launch_context",
        "supervise_paused",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS unit_restart_log (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            agent_id        TEXT NOT NULL,
            source          TEXT NOT NULL,
            triggered_at    TEXT NOT NULL,
            reason          TEXT NOT NULL,
            result          TEXT NOT NULL,
            systemctl_exit  INTEGER,
            notes           TEXT
        );
        CREATE INDEX IF NOT EXISTS unit_restart_log_by_agent_time
            ON unit_restart_log (source, agent_id, triggered_at DESC);",
    )?;
    Ok(())
}

/// Check `PRAGMA table_info(table)` for a column; add it if missing.
/// `column_def` is the full column definition minus the name, e.g.
/// `"TEXT"` or `"INTEGER NOT NULL DEFAULT 0"`.
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    column_def: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let existing: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))? // column 1 is name
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if existing.iter().any(|c| c == column) {
        return Ok(());
    }
    let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {column_def}");
    conn.execute(&sql, [])?;
    Ok(())
}

// ---------- Registry ----------

pub struct LocalRegistry {
    conn: Connection,
}

impl LocalRegistry {
    /// Default DB path: `~/.mc/mcd/registry.db`.
    pub fn default_path() -> Result<PathBuf> {
        Ok(mcd_core::paths::registry_db_path())
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
        apply_migrations(conn)?;
        Ok(())
    }

    /// Report the schema version currently stamped on this database.
    /// Used by `mcd doctor` for diagnostics. Returns 0 if no
    /// `schema_version` row exists (a fresh DB before `migrate` runs).
    pub fn schema_version(&self) -> Result<u32> {
        read_schema_version(&self.conn)
    }

    /// Insert or update an agent record (upsert on (source, id)).
    pub fn upsert(&self, rec: &AgentRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agent
                (id, source, domain_id, runtime_kind, supervision_mode,
                 capabilities_json, profile_path, enrolled_at, last_synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(source, id) DO UPDATE SET
                domain_id        = excluded.domain_id,
                runtime_kind      = excluded.runtime_kind,
                supervision_mode  = excluded.supervision_mode,
                capabilities_json = excluded.capabilities_json,
                profile_path      = excluded.profile_path,
                last_synced_at    = excluded.last_synced_at",
            params![
                rec.id,
                rec.source,
                rec.domain_id,
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

    /// Reassign an agent to a different domain (updates domain_id in place).
    pub fn reassign(&self, source: &str, agent_id: &str, new_domain_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE agent SET domain_id = ?1 WHERE source = ?2 AND id = ?3",
            params![new_domain_id, source, agent_id],
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
            "SELECT id, source, domain_id, runtime_kind, supervision_mode,
                    capabilities_json, profile_path, enrolled_at, last_synced_at
             FROM agent WHERE source = ?1 ORDER BY enrolled_at ASC",
        )?;
        let rows = stmt.query_map(params![source], |row| {
            Ok(AgentRecord {
                id: row.get(0)?,
                source: row.get(1)?,
                domain_id: row.get(2)?,
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

    /// Upsert the launch context for an agent. Foreign-key enforced — the
    /// `(source, agent_id)` row must already exist in `agent`. Phase 1 of
    /// the daemon-absorption plan; consumed by the ZellijHosted runtime
    /// (Phase 2) and the cron registry (Phase 4).
    pub fn upsert_launch_context(&self, ctx: &AgentLaunchContext) -> Result<()> {
        let state_dir_json = ctx
            .state_dir_spec
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("serialising state_dir_spec")?;
        self.conn.execute(
            "INSERT INTO agent_launch_context
                (source, agent_id, vault_folder, state_dir_spec, zellij_session,
                 systemd_service, supervise_paused)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(source, agent_id) DO UPDATE SET
                vault_folder    = excluded.vault_folder,
                state_dir_spec  = excluded.state_dir_spec,
                zellij_session  = excluded.zellij_session,
                systemd_service = excluded.systemd_service",
            // Note: we deliberately don't overwrite supervise_paused on
            // re-import — operator may have paused an agent; an importer
            // refresh shouldn't clobber that.
            params![
                ctx.source,
                ctx.agent_id,
                ctx.vault_folder,
                state_dir_json,
                ctx.zellij_session,
                ctx.systemd_service,
                if ctx.supervise_paused { 1i64 } else { 0i64 },
            ],
        )?;
        Ok(())
    }

    /// Fetch the launch context for a single agent. Returns `None` if no row
    /// exists — most agents (legacy task-mode, controlplane-synced) won't
    /// have one until they're explicitly registered with one.
    pub fn get_launch_context(
        &self,
        source: &str,
        agent_id: &str,
    ) -> Result<Option<AgentLaunchContext>> {
        let mut stmt = self.conn.prepare(
            "SELECT source, agent_id, vault_folder, state_dir_spec, zellij_session,
                    systemd_service, supervise_paused
             FROM agent_launch_context
             WHERE source = ?1 AND agent_id = ?2",
        )?;
        let mut rows = stmt.query(params![source, agent_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_launch_context(row)?))
        } else {
            Ok(None)
        }
    }

    /// List all launch contexts for a given source. Used by the importer
    /// (to skip already-imported rows) and by diagnostic CLIs.
    pub fn list_launch_contexts_by_source(
        &self,
        source: &str,
    ) -> Result<Vec<AgentLaunchContext>> {
        let mut stmt = self.conn.prepare(
            "SELECT source, agent_id, vault_folder, state_dir_spec, zellij_session,
                    systemd_service, supervise_paused
             FROM agent_launch_context
             WHERE source = ?1
             ORDER BY agent_id ASC",
        )?;
        let rows = stmt.query_map(params![source], row_to_launch_context)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading launch contexts from registry")
    }

    /// Set the `supervise_paused` flag for one agent. Used by
    /// `mc agent supervise pause/resume`. Returns true if a row was
    /// updated.
    pub fn set_supervise_paused(
        &self,
        source: &str,
        agent_id: &str,
        paused: bool,
    ) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE agent_launch_context SET supervise_paused = ?1
             WHERE source = ?2 AND agent_id = ?3",
            params![if paused { 1i64 } else { 0i64 }, source, agent_id],
        )?;
        Ok(n > 0)
    }

    /// List all known launch contexts across every source. Used by the
    /// unit-health loop to enumerate agents to supervise.
    pub fn list_all_launch_contexts(&self) -> Result<Vec<AgentLaunchContext>> {
        let mut stmt = self.conn.prepare(
            "SELECT source, agent_id, vault_folder, state_dir_spec, zellij_session,
                    systemd_service, supervise_paused
             FROM agent_launch_context
             ORDER BY source, agent_id ASC",
        )?;
        let rows = stmt.query_map([], row_to_launch_context)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading all launch contexts from registry")
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
                    (id, source, domain_id, runtime_kind, supervision_mode,
                     capabilities_json, profile_path, enrolled_at, last_synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    spec.agent_id,
                    source,
                    spec.domain_id,
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
    pub domain_id: String,
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
    /// stable wire identifier — what mcd uses to poll
    /// `/agents/{id}/messages` and what `mc` CLI passes via `--to-agent-id`.
    pub fn from_spec(spec: &AgentSpec, source: &str) -> Self {
        Self {
            id: spec.agent_id.clone(),
            source: source.to_string(),
            domain_id: spec.domain_id.clone(),
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
            domain_id: self.domain_id,
            runtime_kind: self.runtime_kind,
            session_mode,
            capabilities,
            profile_path: self.profile_path.map(PathBuf::from),
            webhook_url: None,
            // launch_overrides is populated by `resolve_agent_specs` after
            // listing — into_spec only sees the agent row, not the joined
            // launch context.
            launch_overrides: crate::supervisor::SpawnOverrides::default(),
        }
    }
}

// ---------- AgentLaunchContext ----------

/// One row of `agent_launch_context` — the declarative launch parameters
/// for a given agent (vault folder, state dir lifecycle, zellij session
/// name). 1:1 with `agent` by `(source, agent_id)`; most agents won't have
/// a row here until the fleet importer or an explicit registration writes
/// one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunchContext {
    pub source: String,
    pub agent_id: String,
    pub vault_folder: Option<String>,
    pub state_dir_spec: Option<StateDirSpec>,
    pub zellij_session: Option<String>,
    /// systemd `--user` unit name that owns this agent's session (e.g.
    /// `aria-work.service`). Populated by the fleet importer from
    /// fleet-profiles.toml's `service` field. `None` for agents not
    /// managed by systemd.
    pub systemd_service: Option<String>,
    /// Pause flag for the Phase 5 unit-health auto-restart loop. When
    /// true, mcd notices the unit is down but does NOT issue a restart.
    /// Operator-controlled via `mc agent supervise pause/resume`.
    pub supervise_paused: bool,
}

fn row_to_launch_context(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentLaunchContext> {
    let state_dir_json: Option<String> = row.get(3)?;
    let state_dir_spec = match state_dir_json {
        Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })?),
        None => None,
    };
    let supervise_paused_i: i64 = row.get(6).unwrap_or(0);
    Ok(AgentLaunchContext {
        source: row.get(0)?,
        agent_id: row.get(1)?,
        vault_folder: row.get(2)?,
        state_dir_spec,
        zellij_session: row.get(4)?,
        systemd_service: row.get(5)?,
        supervise_paused: supervise_paused_i != 0,
    })
}

// ---------- Cron telemetry (Phase 4) ----------
//
// `cron.toml` is the source of truth for which jobs exist. SQLite only
// stores runtime state — when each job last fired, the outcome, and a
// bounded history log for diagnostics.

/// Latest state per known cron job. One row per job_name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCronState {
    pub job_name: String,
    pub last_fired_at: Option<String>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

/// One row in the append-only fire log. Rows are GC'd per retention
/// policy in the cron.toml file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronFireLogEntry {
    pub id: i64,
    pub job_name: String,
    pub fired_at: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub error_message: Option<String>,
}

impl LocalRegistry {
    /// Record a fire: upsert `agent_cron_state` (latest state) + append
    /// to `agent_cron_fire_log` (history). Both writes happen in one
    /// transaction so a crash between them can't leave a fire half-logged.
    pub fn cron_record_fire(
        &self,
        job_name: &str,
        fired_at: &str,
        status: &str,
        duration_ms: Option<i64>,
        error_message: Option<&str>,
    ) -> Result<()> {
        let tx_conn = &self.conn;
        // SQLite immediate transaction — atomic across the two tables.
        tx_conn.execute("BEGIN IMMEDIATE", [])?;
        let result = (|| -> Result<()> {
            tx_conn.execute(
                "INSERT INTO agent_cron_state
                    (job_name, last_fired_at, last_status, last_error, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?2)
                 ON CONFLICT(job_name) DO UPDATE SET
                    last_fired_at = excluded.last_fired_at,
                    last_status   = excluded.last_status,
                    last_error    = excluded.last_error,
                    updated_at    = excluded.updated_at",
                params![job_name, fired_at, status, error_message],
            )?;
            tx_conn.execute(
                "INSERT INTO agent_cron_fire_log
                    (job_name, fired_at, status, duration_ms, error_message)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![job_name, fired_at, status, duration_ms, error_message],
            )?;
            Ok(())
        })();
        if result.is_ok() {
            tx_conn.execute("COMMIT", [])?;
        } else {
            tx_conn.execute("ROLLBACK", [])?;
        }
        result
    }

    /// Fetch the latest state for one job. Returns `None` if the job
    /// has never fired (no row yet).
    pub fn cron_get_state(&self, job_name: &str) -> Result<Option<AgentCronState>> {
        let mut stmt = self.conn.prepare(
            "SELECT job_name, last_fired_at, last_status, last_error, updated_at
             FROM agent_cron_state WHERE job_name = ?1",
        )?;
        let mut rows = stmt.query(params![job_name])?;
        if let Some(row) = rows.next()? {
            Ok(Some(AgentCronState {
                job_name: row.get(0)?,
                last_fired_at: row.get(1)?,
                last_status: row.get(2)?,
                last_error: row.get(3)?,
                updated_at: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// List all rows in `agent_cron_state`. Used by `mc agent cron list`
    /// to join file-defined jobs against their last-fire status.
    pub fn cron_list_state(&self) -> Result<Vec<AgentCronState>> {
        let mut stmt = self.conn.prepare(
            "SELECT job_name, last_fired_at, last_status, last_error, updated_at
             FROM agent_cron_state",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AgentCronState {
                job_name: row.get(0)?,
                last_fired_at: row.get(1)?,
                last_status: row.get(2)?,
                last_error: row.get(3)?,
                updated_at: row.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading agent_cron_state rows")
    }

    /// Fetch the most recent N fire-log entries for one job, newest first.
    /// `limit` of 0 returns all rows for that job.
    pub fn cron_history_for_job(
        &self,
        job_name: &str,
        limit: u32,
    ) -> Result<Vec<CronFireLogEntry>> {
        let sql = if limit == 0 {
            "SELECT id, job_name, fired_at, status, duration_ms, error_message
             FROM agent_cron_fire_log
             WHERE job_name = ?1
             ORDER BY fired_at DESC".to_string()
        } else {
            format!(
                "SELECT id, job_name, fired_at, status, duration_ms, error_message
                 FROM agent_cron_fire_log
                 WHERE job_name = ?1
                 ORDER BY fired_at DESC LIMIT {limit}"
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![job_name], cron_log_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading agent_cron_fire_log rows")
    }

    /// Recent fires across all jobs, newest first.
    pub fn cron_history_all(&self, limit: u32) -> Result<Vec<CronFireLogEntry>> {
        let sql = if limit == 0 {
            "SELECT id, job_name, fired_at, status, duration_ms, error_message
             FROM agent_cron_fire_log
             ORDER BY fired_at DESC".to_string()
        } else {
            format!(
                "SELECT id, job_name, fired_at, status, duration_ms, error_message
                 FROM agent_cron_fire_log
                 ORDER BY fired_at DESC LIMIT {limit}"
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], cron_log_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading agent_cron_fire_log rows")
    }

    /// GC sweep. Drops fire-log rows older than `history_days` OR beyond
    /// `max_rows_per_job` (whichever fires first per row). Returns the
    /// number of rows deleted across both passes.
    pub fn cron_gc(&self, history_days: u32, max_rows_per_job: u32) -> Result<u64> {
        let mut deleted: u64 = 0;

        // Pass 1: age-based.
        if history_days > 0 {
            let n = self.conn.execute(
                "DELETE FROM agent_cron_fire_log
                 WHERE fired_at < datetime('now', '-' || ?1 || ' days')",
                params![history_days],
            )?;
            deleted += n as u64;
        }

        // Pass 2: per-job cap. For each job_name with > max_rows_per_job
        // remaining, delete the oldest rows.
        if max_rows_per_job > 0 {
            let mut stmt = self.conn.prepare(
                "SELECT job_name FROM agent_cron_fire_log GROUP BY job_name",
            )?;
            let jobs: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for job in jobs {
                let n = self.conn.execute(
                    "DELETE FROM agent_cron_fire_log
                     WHERE job_name = ?1
                       AND id NOT IN (
                         SELECT id FROM agent_cron_fire_log
                         WHERE job_name = ?1
                         ORDER BY fired_at DESC
                         LIMIT ?2
                       )",
                    params![job, max_rows_per_job],
                )?;
                deleted += n as u64;
            }
        }

        Ok(deleted)
    }
}

fn cron_log_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CronFireLogEntry> {
    Ok(CronFireLogEntry {
        id: row.get(0)?,
        job_name: row.get(1)?,
        fired_at: row.get(2)?,
        status: row.get(3)?,
        duration_ms: row.get(4)?,
        error_message: row.get(5)?,
    })
}

// ---------- Unit restart log (Phase 5) ----------
//
// Append-only history of systemd unit restart attempts. Populated by
// the Phase 5 unit_health loop. GC'd per retention config (defaults
// match the cron fire log).

/// One row of `unit_restart_log`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitRestartEntry {
    pub id: i64,
    pub agent_id: String,
    pub source: String,
    pub triggered_at: String,
    /// `'dead'` (unit was down) | `'nightly'` (hygiene restart) | `'manual'` (`mc agent supervise restart`)
    pub reason: String,
    /// `'started'` (success) | `'failed'` (systemctl non-zero) | `'throttled'` (within retry window)
    pub result: String,
    pub systemctl_exit: Option<i64>,
    pub notes: Option<String>,
}

impl LocalRegistry {
    /// Append a row to `unit_restart_log`. Returns the new row id.
    pub fn log_unit_restart(
        &self,
        agent_id: &str,
        source: &str,
        triggered_at: &str,
        reason: &str,
        result: &str,
        systemctl_exit: Option<i64>,
        notes: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO unit_restart_log
                (agent_id, source, triggered_at, reason, result, systemctl_exit, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![agent_id, source, triggered_at, reason, result, systemctl_exit, notes],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Fetch the most recent `limit` rows for one agent, newest first.
    /// `limit = 0` returns all rows for that agent.
    pub fn unit_restart_history(
        &self,
        source: &str,
        agent_id: &str,
        limit: u32,
    ) -> Result<Vec<UnitRestartEntry>> {
        let sql = if limit == 0 {
            "SELECT id, agent_id, source, triggered_at, reason, result, systemctl_exit, notes
             FROM unit_restart_log
             WHERE source = ?1 AND agent_id = ?2
             ORDER BY triggered_at DESC".to_string()
        } else {
            format!(
                "SELECT id, agent_id, source, triggered_at, reason, result, systemctl_exit, notes
                 FROM unit_restart_log
                 WHERE source = ?1 AND agent_id = ?2
                 ORDER BY triggered_at DESC LIMIT {limit}"
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![source, agent_id], unit_restart_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading unit_restart_log rows")
    }

    /// Recent restart events across all agents, newest first.
    pub fn unit_restart_history_all(&self, limit: u32) -> Result<Vec<UnitRestartEntry>> {
        let sql = if limit == 0 {
            "SELECT id, agent_id, source, triggered_at, reason, result, systemctl_exit, notes
             FROM unit_restart_log
             ORDER BY triggered_at DESC".to_string()
        } else {
            format!(
                "SELECT id, agent_id, source, triggered_at, reason, result, systemctl_exit, notes
                 FROM unit_restart_log
                 ORDER BY triggered_at DESC LIMIT {limit}"
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], unit_restart_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("reading unit_restart_log rows")
    }

    /// GC sweep for `unit_restart_log`. Same shape as `cron_gc`: drop
    /// rows older than `history_days`, then per-(source, agent) cap at
    /// `max_rows_per_agent`. Returns total rows deleted.
    pub fn unit_restart_gc(
        &self,
        history_days: u32,
        max_rows_per_agent: u32,
    ) -> Result<u64> {
        let mut deleted: u64 = 0;
        if history_days > 0 {
            let n = self.conn.execute(
                "DELETE FROM unit_restart_log
                 WHERE triggered_at < datetime('now', '-' || ?1 || ' days')",
                params![history_days],
            )?;
            deleted += n as u64;
        }
        if max_rows_per_agent > 0 {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT source, agent_id FROM unit_restart_log",
            )?;
            let pairs: Vec<(String, String)> = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (source, agent_id) in pairs {
                let n = self.conn.execute(
                    "DELETE FROM unit_restart_log
                     WHERE source = ?1 AND agent_id = ?2
                       AND id NOT IN (
                         SELECT id FROM unit_restart_log
                         WHERE source = ?1 AND agent_id = ?2
                         ORDER BY triggered_at DESC
                         LIMIT ?3
                       )",
                    params![source, agent_id, max_rows_per_agent],
                )?;
                deleted += n as u64;
            }
        }
        Ok(deleted)
    }
}

fn unit_restart_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UnitRestartEntry> {
    Ok(UnitRestartEntry {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        source: row.get(2)?,
        triggered_at: row.get(3)?,
        reason: row.get(4)?,
        result: row.get(5)?,
        systemctl_exit: row.get(6)?,
        notes: row.get(7)?,
    })
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
        let path = dir.path().join("registry.db");
        let reg = LocalRegistry::open(&path).unwrap();
        (dir, reg)
    }

    fn spec(id: &str, domain: &str, mode: SessionMode) -> AgentSpec {
        AgentSpec {
            agent_id: id.into(),
            domain_id: domain.into(),
            runtime_kind: "claude_agent_acp".into(),
            session_mode: mode,
            capabilities: vec![],
            profile_path: None,
            webhook_url: None,
            launch_overrides: Default::default(),
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
    fn upsert_updates_domain() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-2", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        let specs = reg.list_specs_by_source(SOURCE_LOCAL).unwrap();
        assert_eq!(specs[0].domain_id, "m-2");
    }

    #[test]
    fn reassign_changes_domain() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        let changed = reg.reassign(SOURCE_LOCAL, "a-1", "m-2").unwrap();
        assert!(changed);
        let specs = reg.list_specs_by_source(SOURCE_LOCAL).unwrap();
        assert_eq!(specs[0].domain_id, "m-2");
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
        let path = dir.path().join("registry.db");
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
        let path = dir.path().join("registry.db");
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
    fn launch_context_round_trips_persistent() {
        let (_dir, reg) = tmp_reg();
        // FK requires the agent row to exist first.
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Persistent), SOURCE_LOCAL))
            .unwrap();
        let ctx = AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "a-1".into(),
            vault_folder: Some("work".into()),
            state_dir_spec: Some(StateDirSpec::Persistent {
                path: PathBuf::from("/home/merlin/.claude/profiles/work"),
            }),
            zellij_session: Some("work".into()),
            systemd_service: None,
            supervise_paused: false,
        };
        reg.upsert_launch_context(&ctx).unwrap();
        let got = reg.get_launch_context(SOURCE_LOCAL, "a-1").unwrap().unwrap();
        assert_eq!(got, ctx);
    }

    #[test]
    fn launch_context_round_trips_ephemeral() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        let ctx = AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "a-1".into(),
            vault_folder: None,
            state_dir_spec: Some(StateDirSpec::Ephemeral { ttl_minutes: Some(30) }),
            zellij_session: None,
            systemd_service: None,
            supervise_paused: false,
        };
        reg.upsert_launch_context(&ctx).unwrap();
        let got = reg.get_launch_context(SOURCE_LOCAL, "a-1").unwrap().unwrap();
        assert_eq!(got, ctx);
    }

    #[test]
    fn launch_context_missing_returns_none() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Task), SOURCE_LOCAL))
            .unwrap();
        assert!(reg.get_launch_context(SOURCE_LOCAL, "a-1").unwrap().is_none());
    }

    #[test]
    fn launch_context_fk_requires_agent_row() {
        let (_dir, reg) = tmp_reg();
        // No agent row → FK violation.
        let ctx = AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "no-such-agent".into(),
            vault_folder: None,
            state_dir_spec: None,
            zellij_session: None,
            systemd_service: None,
            supervise_paused: false,
        };
        assert!(reg.upsert_launch_context(&ctx).is_err());
    }

    #[test]
    fn launch_context_cascades_on_agent_delete() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Persistent), SOURCE_LOCAL))
            .unwrap();
        reg.upsert_launch_context(&AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "a-1".into(),
            vault_folder: Some("operator".into()),
            state_dir_spec: None,
            zellij_session: Some("operator".into()),
            systemd_service: None,
            supervise_paused: false,
        })
        .unwrap();
        assert!(reg.delete(SOURCE_LOCAL, "a-1").unwrap());
        // Deleting the agent cascades — the launch_context row is gone too.
        assert!(reg.get_launch_context(SOURCE_LOCAL, "a-1").unwrap().is_none());
    }

    #[test]
    fn launch_context_upsert_overwrites() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a-1", "m-1", SessionMode::Persistent), SOURCE_LOCAL))
            .unwrap();
        reg.upsert_launch_context(&AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "a-1".into(),
            vault_folder: Some("work".into()),
            state_dir_spec: None,
            zellij_session: Some("work".into()),
            systemd_service: None,
            supervise_paused: false,
        })
        .unwrap();
        reg.upsert_launch_context(&AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "a-1".into(),
            vault_folder: Some("research".into()),
            state_dir_spec: None,
            zellij_session: Some("research".into()),
            systemd_service: None,
            supervise_paused: false,
        })
        .unwrap();
        let got = reg.get_launch_context(SOURCE_LOCAL, "a-1").unwrap().unwrap();
        assert_eq!(got.vault_folder.as_deref(), Some("research"));
        assert_eq!(got.zellij_session.as_deref(), Some("research"));
    }

    #[test]
    fn replace_source_does_not_touch_other_sources() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("registry.db");
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

    // ---- Phase 4: cron telemetry ----

    #[test]
    fn cron_record_fire_writes_state_and_log() {
        let (_dir, reg) = tmp_reg();
        reg.cron_record_fire(
            "briefing",
            "2026-05-20T05:30:00Z",
            "ok",
            Some(120),
            None,
        )
        .unwrap();

        let state = reg.cron_get_state("briefing").unwrap().unwrap();
        assert_eq!(state.job_name, "briefing");
        assert_eq!(state.last_fired_at.as_deref(), Some("2026-05-20T05:30:00Z"));
        assert_eq!(state.last_status.as_deref(), Some("ok"));
        assert!(state.last_error.is_none());

        let history = reg.cron_history_for_job("briefing", 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].duration_ms, Some(120));
    }

    #[test]
    fn cron_record_fire_with_error() {
        let (_dir, reg) = tmp_reg();
        reg.cron_record_fire(
            "briefing",
            "2026-05-20T05:30:00Z",
            "error",
            Some(50),
            Some("agent operator not supervised"),
        )
        .unwrap();

        let state = reg.cron_get_state("briefing").unwrap().unwrap();
        assert_eq!(state.last_status.as_deref(), Some("error"));
        assert_eq!(
            state.last_error.as_deref(),
            Some("agent operator not supervised")
        );
    }

    #[test]
    fn cron_record_fire_upserts_state() {
        let (_dir, reg) = tmp_reg();
        // First fire
        reg.cron_record_fire("briefing", "2026-05-19T05:30:00Z", "ok", Some(100), None)
            .unwrap();
        // Second fire: state overwrites, history appends
        reg.cron_record_fire("briefing", "2026-05-20T05:30:00Z", "error", Some(200), Some("x"))
            .unwrap();

        let state = reg.cron_get_state("briefing").unwrap().unwrap();
        assert_eq!(state.last_fired_at.as_deref(), Some("2026-05-20T05:30:00Z"));
        assert_eq!(state.last_status.as_deref(), Some("error"));

        let history = reg.cron_history_for_job("briefing", 10).unwrap();
        assert_eq!(history.len(), 2);
        // Newest first
        assert_eq!(history[0].fired_at, "2026-05-20T05:30:00Z");
        assert_eq!(history[1].fired_at, "2026-05-19T05:30:00Z");
    }

    #[test]
    fn cron_get_state_returns_none_for_missing_job() {
        let (_dir, reg) = tmp_reg();
        assert!(reg.cron_get_state("never-fired").unwrap().is_none());
    }

    #[test]
    fn cron_history_all_across_jobs() {
        let (_dir, reg) = tmp_reg();
        reg.cron_record_fire("a", "2026-05-20T01:00:00Z", "ok", None, None).unwrap();
        reg.cron_record_fire("b", "2026-05-20T02:00:00Z", "ok", None, None).unwrap();
        reg.cron_record_fire("a", "2026-05-20T03:00:00Z", "ok", None, None).unwrap();

        let all = reg.cron_history_all(10).unwrap();
        assert_eq!(all.len(), 3);
        // Newest first
        assert_eq!(all[0].fired_at, "2026-05-20T03:00:00Z");
    }

    #[test]
    fn cron_list_state_returns_all() {
        let (_dir, reg) = tmp_reg();
        reg.cron_record_fire("a", "2026-05-20T01:00:00Z", "ok", None, None).unwrap();
        reg.cron_record_fire("b", "2026-05-20T02:00:00Z", "ok", None, None).unwrap();

        let all = reg.cron_list_state().unwrap();
        assert_eq!(all.len(), 2);
        let names: std::collections::HashSet<String> =
            all.into_iter().map(|s| s.job_name).collect();
        assert!(names.contains("a"));
        assert!(names.contains("b"));
    }

    #[test]
    fn cron_gc_caps_per_job() {
        let (_dir, reg) = tmp_reg();
        // Insert 20 rows for job-a, 5 for job-b.
        for i in 0..20 {
            reg.cron_record_fire(
                "job-a",
                &format!("2026-05-{:02}T00:00:00Z", i + 1),
                "ok",
                None,
                None,
            )
            .unwrap();
        }
        for i in 0..5 {
            reg.cron_record_fire(
                "job-b",
                &format!("2026-05-{:02}T00:00:00Z", i + 1),
                "ok",
                None,
                None,
            )
            .unwrap();
        }

        // GC: keep all ages, cap at 10/job.
        let deleted = reg.cron_gc(0, 10).unwrap();
        assert_eq!(deleted, 10, "expected to drop 10 from job-a, 0 from job-b");

        let a_rows = reg.cron_history_for_job("job-a", 0).unwrap();
        let b_rows = reg.cron_history_for_job("job-b", 0).unwrap();
        assert_eq!(a_rows.len(), 10);
        assert_eq!(b_rows.len(), 5);
    }

    #[test]
    fn cron_gc_drops_old_rows() {
        let (_dir, reg) = tmp_reg();
        // Insert a row that's clearly outside the retention window (older
        // than 30 days from "now"). SQLite's datetime('now', '-30 days')
        // is wall-clock time, so we need a row dated long in the past.
        reg.cron_record_fire(
            "job-a",
            "2020-01-01T00:00:00",
            "ok",
            None,
            None,
        )
        .unwrap();
        reg.cron_record_fire(
            "job-a",
            &chrono::Utc::now().to_rfc3339(),
            "ok",
            None,
            None,
        )
        .unwrap();

        // GC with 30 days retention should drop the 2020 row, keep the recent.
        let deleted = reg.cron_gc(30, 0).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(reg.cron_history_for_job("job-a", 0).unwrap().len(), 1);
    }

    #[test]
    fn cron_gc_zero_history_days_keeps_all() {
        let (_dir, reg) = tmp_reg();
        reg.cron_record_fire("a", "2020-01-01T00:00:00", "ok", None, None).unwrap();
        reg.cron_record_fire("a", "2026-05-20T00:00:00Z", "ok", None, None).unwrap();

        // 0 history_days = keep forever (no age sweep). 0 max_rows_per_job = no cap.
        let deleted = reg.cron_gc(0, 0).unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(reg.cron_history_for_job("a", 0).unwrap().len(), 2);
    }

    #[test]
    fn cron_history_limit_respected() {
        let (_dir, reg) = tmp_reg();
        for i in 0..5 {
            reg.cron_record_fire(
                "job-x",
                &format!("2026-05-{:02}T00:00:00Z", i + 1),
                "ok",
                None,
                None,
            )
            .unwrap();
        }
        let limit_2 = reg.cron_history_for_job("job-x", 2).unwrap();
        assert_eq!(limit_2.len(), 2);
        // Newest first
        assert_eq!(limit_2[0].fired_at, "2026-05-05T00:00:00Z");
        assert_eq!(limit_2[1].fired_at, "2026-05-04T00:00:00Z");
    }

    // ---- Phase 5: schema versioning + supervise + unit_restart_log ----

    #[test]
    fn schema_version_stamped_on_open() {
        let (_dir, reg) = tmp_reg();
        assert_eq!(reg.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn migrate_from_empty_db_is_idempotent() {
        // Open twice: second open shouldn't re-apply migrations or error.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("registry.db");
        let _r1 = LocalRegistry::open(&path).unwrap();
        let r2 = LocalRegistry::open(&path).unwrap();
        assert_eq!(r2.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn migrate_from_v1_baseline_adds_v2_columns() {
        // Simulate a pre-Phase-5 DB: open with the v1 schema, manually
        // stamp version=1, then re-open and verify v2 ALTER ran.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("registry.db");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "foreign_keys", 1).unwrap();
        migrate_to_v1(&conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY);
             INSERT OR IGNORE INTO schema_version VALUES (1);",
        )
        .unwrap();
        drop(conn);

        // Re-open through the normal path; should apply v2.
        let reg = LocalRegistry::open(&path).unwrap();
        assert_eq!(reg.schema_version().unwrap(), 2);

        // Confirm the new columns exist on agent_launch_context.
        let conn = Connection::open(&path).unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(agent_launch_context)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(cols.contains(&"systemd_service".to_string()), "cols: {cols:?}");
        assert!(cols.contains(&"supervise_paused".to_string()), "cols: {cols:?}");

        // unit_restart_log must exist.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM unit_restart_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let _ = reg; // keep reg alive past the manual queries
    }

    #[test]
    fn launch_context_with_systemd_service_round_trips() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("work", "", SessionMode::Persistent), SOURCE_LOCAL))
            .unwrap();
        let ctx = AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "work".into(),
            vault_folder: Some("work".into()),
            state_dir_spec: None,
            zellij_session: Some("work".into()),
            systemd_service: Some("aria-work.service".into()),
            supervise_paused: false,
        };
        reg.upsert_launch_context(&ctx).unwrap();
        let got = reg.get_launch_context(SOURCE_LOCAL, "work").unwrap().unwrap();
        assert_eq!(got.systemd_service.as_deref(), Some("aria-work.service"));
        assert!(!got.supervise_paused);
    }

    #[test]
    fn set_supervise_paused_toggles() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("work", "", SessionMode::Persistent), SOURCE_LOCAL))
            .unwrap();
        reg.upsert_launch_context(&AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "work".into(),
            vault_folder: None,
            state_dir_spec: None,
            zellij_session: None,
            systemd_service: Some("aria-work.service".into()),
            supervise_paused: false,
        })
        .unwrap();

        let changed = reg.set_supervise_paused(SOURCE_LOCAL, "work", true).unwrap();
        assert!(changed);
        let got = reg.get_launch_context(SOURCE_LOCAL, "work").unwrap().unwrap();
        assert!(got.supervise_paused);

        let changed = reg.set_supervise_paused(SOURCE_LOCAL, "work", false).unwrap();
        assert!(changed);
        let got = reg.get_launch_context(SOURCE_LOCAL, "work").unwrap().unwrap();
        assert!(!got.supervise_paused);
    }

    #[test]
    fn upsert_launch_context_preserves_supervise_paused() {
        // Operator's pause must not be clobbered by a re-import.
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("work", "", SessionMode::Persistent), SOURCE_LOCAL))
            .unwrap();
        reg.upsert_launch_context(&AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "work".into(),
            vault_folder: None,
            state_dir_spec: None,
            zellij_session: None,
            systemd_service: Some("aria-work.service".into()),
            supervise_paused: false,
        })
        .unwrap();
        reg.set_supervise_paused(SOURCE_LOCAL, "work", true).unwrap();

        // Re-import: paused must stay true.
        reg.upsert_launch_context(&AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "work".into(),
            vault_folder: Some("work-vault".into()),
            state_dir_spec: None,
            zellij_session: Some("work".into()),
            systemd_service: Some("aria-work.service".into()),
            supervise_paused: false, // importer doesn't know about pause
        })
        .unwrap();
        let got = reg.get_launch_context(SOURCE_LOCAL, "work").unwrap().unwrap();
        assert!(got.supervise_paused, "operator pause must survive re-import");
        assert_eq!(got.vault_folder.as_deref(), Some("work-vault")); // other fields update
    }

    #[test]
    fn list_all_launch_contexts_across_sources() {
        let (_dir, reg) = tmp_reg();
        reg.upsert(&AgentRecord::from_spec(&spec("a", "", SessionMode::Persistent), SOURCE_LOCAL))
            .unwrap();
        reg.upsert(&AgentRecord::from_spec(&spec("b", "", SessionMode::Persistent), &source_cp("work")))
            .unwrap();
        reg.upsert_launch_context(&AgentLaunchContext {
            source: SOURCE_LOCAL.into(),
            agent_id: "a".into(),
            vault_folder: None,
            state_dir_spec: None,
            zellij_session: None,
            systemd_service: None,
            supervise_paused: false,
        })
        .unwrap();
        reg.upsert_launch_context(&AgentLaunchContext {
            source: source_cp("work"),
            agent_id: "b".into(),
            vault_folder: None,
            state_dir_spec: None,
            zellij_session: None,
            systemd_service: None,
            supervise_paused: false,
        })
        .unwrap();
        let all = reg.list_all_launch_contexts().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn log_unit_restart_round_trips() {
        let (_dir, reg) = tmp_reg();
        let id = reg
            .log_unit_restart(
                "work",
                SOURCE_LOCAL,
                "2026-05-20T12:00:00Z",
                "dead",
                "started",
                Some(0),
                None,
            )
            .unwrap();
        assert!(id > 0);
        let history = reg.unit_restart_history(SOURCE_LOCAL, "work", 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].reason, "dead");
        assert_eq!(history[0].result, "started");
        assert_eq!(history[0].systemctl_exit, Some(0));
    }

    #[test]
    fn unit_restart_history_all_across_agents() {
        let (_dir, reg) = tmp_reg();
        reg.log_unit_restart("a", SOURCE_LOCAL, "2026-05-20T01:00:00Z", "dead", "started", Some(0), None)
            .unwrap();
        reg.log_unit_restart("b", SOURCE_LOCAL, "2026-05-20T02:00:00Z", "nightly", "started", Some(0), None)
            .unwrap();
        reg.log_unit_restart("a", SOURCE_LOCAL, "2026-05-20T03:00:00Z", "manual", "started", Some(0), None)
            .unwrap();
        let all = reg.unit_restart_history_all(10).unwrap();
        assert_eq!(all.len(), 3);
        // Newest first
        assert_eq!(all[0].triggered_at, "2026-05-20T03:00:00Z");
    }

    #[test]
    fn unit_restart_gc_caps_per_agent() {
        let (_dir, reg) = tmp_reg();
        for i in 0..15 {
            reg.log_unit_restart(
                "work",
                SOURCE_LOCAL,
                &format!("2026-05-{:02}T00:00:00Z", i + 1),
                "dead",
                "started",
                Some(0),
                None,
            )
            .unwrap();
        }
        let deleted = reg.unit_restart_gc(0, 5).unwrap();
        assert_eq!(deleted, 10);
        assert_eq!(reg.unit_restart_history(SOURCE_LOCAL, "work", 0).unwrap().len(), 5);
    }
}
