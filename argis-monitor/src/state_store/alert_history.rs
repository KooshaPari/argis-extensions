//! alert_history CRUD: append-only log of every alert state transition.

use serde::{Deserialize, Serialize};

use super::types::StateStoreError;
use super::StateStore;

/// One row of alert history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertHistoryRow {
    pub id: i64,
    pub key: String,
    pub event: String,
    pub severity: String,
    pub burn_rate: f64,
    pub threshold: f64,
    pub payload_json: String,
    pub fired_at_unix: u64,
}

impl StateStore {
    pub fn record_event(
        &mut self,
        key: &str,
        event: &str,
        severity: &str,
        burn_rate: f64,
        threshold: f64,
        payload_json: &str,
        fired_at_unix: u64,
    ) -> Result<(), StateStoreError> {
        self.conn.execute(
            "INSERT INTO alert_history (key, event, severity, burn_rate, threshold, payload_json, fired_at_unix)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![key, event, severity, burn_rate, threshold, payload_json, fired_at_unix as i64],
        )?;
        Ok(())
    }

    pub fn list_history(
        &self,
        key_prefix: Option<&str>,
        limit: u32,
    ) -> Result<Vec<AlertHistoryRow>, StateStoreError> {
        let limit = limit.min(10_000) as i64;
        let mut stmt = match key_prefix {
            Some(p) => self.conn.prepare(
                "SELECT id, key, event, severity, burn_rate, threshold, payload_json, fired_at_unix
                 FROM alert_history WHERE key LIKE ?1 ORDER BY fired_at_unix DESC, id DESC LIMIT ?2"
            )?,
            None => self.conn.prepare(
                "SELECT id, key, event, severity, burn_rate, threshold, payload_json, fired_at_unix
                 FROM alert_history ORDER BY fired_at_unix DESC, id DESC LIMIT ?1"
            )?,
        };
        let row_map = |row: &rusqlite::Row| -> rusqlite::Result<AlertHistoryRow> {
            Ok(AlertHistoryRow {
                id: row.get(0)?,
                key: row.get(1)?,
                event: row.get(2)?,
                severity: row.get(3)?,
                burn_rate: row.get(4)?,
                threshold: row.get(5)?,
                payload_json: row.get(6)?,
                fired_at_unix: row.get::<_, i64>(7)? as u64,
            })
        };
        let rows = match key_prefix {
            Some(p) => stmt.query_map(rusqlite::params![format!("{p}%"), limit], row_map)?,
            None => stmt.query_map(rusqlite::params![limit], row_map)?,
        };
        let mut out = Vec::new();
        for row in rows { out.push(row?); }
        Ok(out)
    }
}
