use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, Row};
use tracing::debug;

use crate::{
    error::ReceiptsError,
    types::{Receipt, ReceiptFilter},
};

pub struct ReceiptStore {
    conn: std::sync::Mutex<Connection>,
}

impl ReceiptStore {
    /// Open or create the SQLite database at `path`, enabling WAL mode and creating schema.
    pub fn open(path: &Path) -> Result<Self, ReceiptsError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| rusqlite::Error::InvalidPath(format!("{e}").into()))?;
            }
        }

        let conn = Connection::open(path)?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS receipts (
                 id                TEXT PRIMARY KEY,
                 capability        TEXT NOT NULL,
                 args_json         TEXT NOT NULL,
                 result_json       TEXT NOT NULL,
                 exit_code         INTEGER NOT NULL,
                 execution_time_ms INTEGER NOT NULL,
                 mission_id        TEXT,
                 agent_id          TEXT,
                 created_at        TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS receipts_created_at ON receipts(created_at DESC);
             CREATE INDEX IF NOT EXISTS receipts_mission_id ON receipts(mission_id);",
        )?;

        debug!(path = %path.display(), "ReceiptStore opened");
        Ok(Self { conn: std::sync::Mutex::new(conn) })
    }

    /// Insert a receipt record. Duplicate IDs are silently ignored (audit integrity).
    pub fn insert(&self, r: &Receipt) -> Result<(), ReceiptsError> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO receipts
             (id, capability, args_json, result_json, exit_code, execution_time_ms, mission_id, agent_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO NOTHING",
            rusqlite::params![
                r.id,
                r.capability,
                r.args_json,
                r.result_json,
                r.exit_code,
                r.execution_time_ms as i64,
                r.mission_id,
                r.agent_id,
                r.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Return the most recent `limit` receipts, newest first.
    pub fn last(&self, limit: usize) -> Result<Vec<Receipt>, ReceiptsError> {
        let limit = if limit == 0 { 50 } else { limit };
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, capability, args_json, result_json, exit_code, execution_time_ms,
                    mission_id, agent_id, created_at
             FROM receipts
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], row_to_receipt)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(ReceiptsError::Db)
    }

    /// Fetch a single receipt by id; returns `None` if not found.
    pub fn get(&self, id: &str) -> Result<Option<Receipt>, ReceiptsError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, capability, args_json, result_json, exit_code, execution_time_ms,
                    mission_id, agent_id, created_at
             FROM receipts
             WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], row_to_receipt)?;
        match rows.next() {
            Some(r) => Ok(Some(r.map_err(ReceiptsError::Db)?)),
            None => Ok(None),
        }
    }

    /// List receipts with optional filters, newest first.
    pub fn list(&self, filter: ReceiptFilter) -> Result<Vec<Receipt>, ReceiptsError> {
        let f = filter;
        let limit = if f.limit == 0 { 50 } else { f.limit };

        let mut conditions: Vec<&str> = Vec::new();
        let mut mission_id_val: Option<String> = None;
        let mut agent_id_val: Option<String> = None;
        let mut capability_val: Option<String> = None;
        let mut since_val: Option<String> = None;

        if f.mission_id.is_some() { conditions.push("mission_id = ?1"); mission_id_val = f.mission_id; }
        if f.agent_id.is_some() { conditions.push("agent_id = ?2"); agent_id_val = f.agent_id; }
        if f.capability.is_some() { conditions.push("capability = ?3"); capability_val = f.capability; }
        if f.since.is_some() { conditions.push("created_at >= ?4"); since_val = f.since.map(|dt| dt.to_rfc3339()); }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT id, capability, args_json, result_json, exit_code, execution_time_ms, \
             mission_id, agent_id, created_at FROM receipts{} ORDER BY created_at DESC LIMIT ?5",
            where_clause
        );

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql)?;

        let rows = stmt.query_map(
            rusqlite::params![
                mission_id_val.as_deref(),
                agent_id_val.as_deref(),
                capability_val.as_deref(),
                since_val.as_deref(),
                limit as i64,
            ],
            row_to_receipt,
        )?;

        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }
}

fn row_to_receipt(row: &Row<'_>) -> rusqlite::Result<Receipt> {
    let s: String = row.get(8)?;
    let created_at = DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            Box::new(e),
        ))?;
    let execution_time_ms: i64 = row.get(5)?;
    Ok(Receipt {
        id: row.get(0)?,
        capability: row.get(1)?,
        args_json: row.get(2)?,
        result_json: row.get(3)?,
        exit_code: row.get(4)?,
        execution_time_ms: execution_time_ms as u64,
        mission_id: row.get(6)?,
        agent_id: row.get(7)?,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    fn sample(id: &str) -> Receipt {
        Receipt {
            id: id.to_string(),
            capability: "kubectl-observe.kubectl-get-pods".to_string(),
            args_json: r#"{"namespace":"default"}"#.to_string(),
            result_json: r#"{"ok":true}"#.to_string(),
            exit_code: 0,
            execution_time_ms: 42,
            mission_id: Some("m1".to_string()),
            agent_id: Some("a1".to_string()),
            created_at: Utc::now(),
        }
    }

    fn sample_at(id: &str, ts: chrono::DateTime<Utc>) -> Receipt {
        Receipt {
            id: id.to_string(),
            capability: "kubectl-observe.kubectl-get-pods".to_string(),
            args_json: r#"{"namespace":"default"}"#.to_string(),
            result_json: r#"{"ok":true}"#.to_string(),
            exit_code: 0,
            execution_time_ms: 42,
            mission_id: Some("m1".to_string()),
            agent_id: Some("a1".to_string()),
            created_at: ts,
        }
    }

    #[test]
    fn open_creates_schema() {
        let dir = tempdir().unwrap();
        let store = ReceiptStore::open(&dir.path().join("receipts.db")).unwrap();
        assert!(store.last(10).unwrap().is_empty());
    }

    #[test]
    fn insert_and_last() {
        let dir = tempdir().unwrap();
        let store = ReceiptStore::open(&dir.path().join("receipts.db")).unwrap();
        // Use explicit timestamps 1 second apart to ensure deterministic ordering.
        let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 1).unwrap();
        store.insert(&sample_at("r1", t1)).unwrap();
        store.insert(&sample_at("r2", t2)).unwrap();
        let last = store.last(1).unwrap();
        assert_eq!(last.len(), 1);
        // r2 has the later timestamp so it should be returned first.
        assert_eq!(last[0].id, "r2");
    }

    #[test]
    fn get_returns_none_for_unknown() {
        let dir = tempdir().unwrap();
        let store = ReceiptStore::open(&dir.path().join("receipts.db")).unwrap();
        assert!(store.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn list_filters_by_mission_id() {
        let dir = tempdir().unwrap();
        let store = ReceiptStore::open(&dir.path().join("receipts.db")).unwrap();
        let mut r1 = sample("r1");
        r1.mission_id = Some("mission-a".to_string());
        let mut r2 = sample("r2");
        r2.mission_id = Some("mission-b".to_string());
        store.insert(&r1).unwrap();
        store.insert(&r2).unwrap();
        let results = store
            .list(ReceiptFilter {
                mission_id: Some("mission-a".to_string()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "r1");
    }
}
