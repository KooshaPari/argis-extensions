//! SQLite-backed persistent alert state.
//!
//! Why SQLite and not Postgres or a flat file:
//!   - Postgres requires a server (overkill for the substrate's scale;
//!     ~tens of rows of alert state per monitor instance).
//!   - A flat file works but is racy under concurrent ticks.
//!   - SQLite (via rusqlite with the `bundled` feature) gives us ACID
//!     transactions without external dependencies.
//!
//! Schema:
//!
//! ```sql
//! CREATE TABLE alert_state (
//!     key             TEXT PRIMARY KEY,   -- "{target}::{rule_name}"
//!     state           TEXT NOT NULL,      -- "ok" | "pending" | "firing"
//!     since_unix      INTEGER NOT NULL,   -- when the current state started
//!     last_fired_unix INTEGER NOT NULL DEFAULT 0,
//!     sustained_secs  INTEGER NOT NULL DEFAULT 0
//! );
//! ```
//!
//! The state store is synchronous; it\'s only called once per poll tick so
//! the latency cost is negligible (~50us per save).

use std::path::Path;

use rusqlite::{params, Connection};
use thiserror::Error;

use crate::alerts::AlertState;

/// Errors from the state store.
#[derive(Debug, Error)]
pub enum StateStoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid state string in DB: {0}")]
    InvalidState(String),
}

/// A snapshot of one (target, rule) state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerSnapshot {
    pub state: AlertState,
    pub sustained_secs: u64,
}

impl TrackerSnapshot {
    pub fn ok() -> Self {
        Self { state: AlertState::Ok, sustained_secs: 0 }
    }
}

/// The state store. Cheap to clone (wraps `Arc<Connection>` internally via
/// `Connection::open` once per monitor).
pub struct StateStore {
    conn: Connection,
}

impl StateStore {
    /// Open or create the SQLite file at `path`. Runs the schema migration.
    pub fn open(path: &Path) -> Result<Self, StateStoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Load every persisted (key -> snapshot). Used on startup to
    /// rehydrate `Monitor::alert_trackers` from disk.
    pub fn load_all(&self) -> Result<Vec<(String, TrackerSnapshot)>, StateStoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT key, state, since_unix, last_fired_unix, sustained_secs FROM alert_state",
        )?;
        let rows = stmt.query_map([], |row| {
            let key: String = row.get(0)?;
            let state_str: String = row.get(1)?;
            let since_unix: u64 = row.get(2)?;
            let last_fired_unix: u64 = row.get(3)?;
            let sustained_secs: u64 = row.get(4)?;
            let state = parse_state(&state_str, since_unix, last_fired_unix)
                .map_err(|e| rusqlite::Error::InvalidQuery)?;
            Ok((key, TrackerSnapshot { state, sustained_secs }))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Upsert one (key -> state) snapshot.
    pub fn save(&mut self, key: &str, snap: &TrackerSnapshot) -> Result<(), StateStoreError> {
        let (state_str, since, last_fired) = flatten(snap);
        self.conn.execute(
            "INSERT INTO alert_state (key, state, since_unix, last_fired_unix, sustained_secs)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(key) DO UPDATE SET
                state=excluded.state,
                since_unix=excluded.since_unix,
                last_fired_unix=excluded.last_fired_unix,
                sustained_secs=excluded.sustained_secs",
            params![key, state_str, since as i64, last_fired as i64, snap.sustained_secs as i64],
        )?;
        Ok(())
    }

    /// Delete the row for `key`. Used when an alert rule is removed from
    /// the config; we don\'t want stale rows hanging around forever.
    pub fn delete(&mut self, key: &str) -> Result<(), StateStoreError> {
        self.conn.execute("DELETE FROM alert_state WHERE key = ?1", params![key])?;
        Ok(())
    }

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

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS alert_state (
    key             TEXT PRIMARY KEY,
    state           TEXT NOT NULL,
    since_unix      INTEGER NOT NULL,
    last_fired_unix INTEGER NOT NULL DEFAULT 0,
    sustained_secs  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_alert_state_key ON alert_state(key);

CREATE TABLE IF NOT EXISTS alert_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    key             TEXT NOT NULL,
    event           TEXT NOT NULL,
    severity        TEXT NOT NULL,
    burn_rate       REAL NOT NULL,
    threshold       REAL NOT NULL,
    payload_json    TEXT NOT NULL,
    fired_at_unix   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_alert_history_key ON alert_history(key);
CREATE INDEX IF NOT EXISTS idx_alert_history_fired_at ON alert_history(fired_at_unix);
"#;

/// One row of alert history.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
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

/// Parse a persisted alert state, surfacing corrupt rows as
/// `StateStoreError::InvalidState`.
pub fn parse_state(s: &str, since: u64, last_fired: u64) -> Result<AlertState, StateStoreError> {
    match s {
        "ok" => Ok(AlertState::Ok),
        "pending" => Ok(AlertState::Pending { since }),
        "firing" => Ok(AlertState::Firing { since, last_fired_at: last_fired }),
        other => Err(StateStoreError::InvalidState(other.to_string())),
    }
}

/// Backward-compatible alias for callers using the previous name.
pub use parse_state as parse_state_err;

fn flatten(snap: &TrackerSnapshot) -> (&'static str, u64, u64) {
    match &snap.state {
        AlertState::Ok => ("ok", 0, 0),
        AlertState::Pending { since } => ("pending", *since, 0),
        AlertState::Firing { since, last_fired_at } => ("firing", *since, *last_fired_at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::AlertStateTracker;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn tmpfile(label: &str) -> std::path::PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let tid = std::thread::current().id();
        // Format the thread id as its Debug repr (a portable integer-ish form).
        let dir = std::env::temp_dir().join(format!(
            "argis-monitor-test-pid{}-{:?}-{}-{}",
            std::process::id(), tid, label, n,
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("state.sqlite")
    }

    #[test]
    fn round_trip_ok_state() {
        let path = tmpfile("ok");
        let _ = std::fs::remove_file(&path);
        let mut store = StateStore::open(&path).unwrap();
        let snap = TrackerSnapshot::ok();
        store.save("gw::rule_a", &snap).unwrap();
        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "gw::rule_a");
        assert_eq!(all[0].1, snap);
    }

    #[test]
    fn round_trip_pending_state() {
        let path = tmpfile("pending");
        let _ = std::fs::remove_file(&path);
        let mut store = StateStore::open(&path).unwrap();
        let snap = TrackerSnapshot {
            state: AlertState::Pending { since: 1234567890 },
            sustained_secs: 12,
        };
        store.save("gw::rule_b", &snap).unwrap();
        let all = store.load_all().unwrap();
        assert_eq!(all[0].1.state, AlertState::Pending { since: 1234567890 });
        assert_eq!(all[0].1.sustained_secs, 12);
    }

    #[test]
    fn upsert_overwrites_previous_state() {
        let path = tmpfile("upsert");
        let _ = std::fs::remove_file(&path);
        let mut store = StateStore::open(&path).unwrap();
        store.save("gw::rule_c", &TrackerSnapshot::ok()).unwrap();
        store.save("gw::rule_c", &TrackerSnapshot {
            state: AlertState::Firing { since: 100, last_fired_at: 100 },
            sustained_secs: 0,
        }).unwrap();
        let all = store.load_all().unwrap();
        assert_eq!(all.len(), 1);
        assert!(matches!(all[0].1.state, AlertState::Firing { .. }));
    }

    #[test]
    fn restart_rehydration_matches_in_memory() {
        let path = tmpfile("restart");
        let snaps = vec![
            ("gw::r1", TrackerSnapshot { state: AlertState::Pending { since: 1000 }, sustained_secs: 5 }),
            ("gw::r2", TrackerSnapshot::ok()),
            ("openai::r1", TrackerSnapshot { state: AlertState::Firing { since: 2000, last_fired_at: 2000 }, sustained_secs: 30 }),
        ];
        {
            let mut store = StateStore::open(&path).unwrap();
            for (k, s) in &snaps {
                store.save(k, s).unwrap();
            }
        }
        let store = StateStore::open(&path).unwrap();
        let restored = store.load_all().unwrap();
        assert_eq!(restored.len(), snaps.len());
        for (k, s) in &snaps {
            let found = restored.iter().find(|(rk, _)| rk == k).expect("missing key");
            assert_eq!(&found.1, s);
        }
    }

    #[test]
    fn delete_removes_row() {
        let path = tmpfile("delete");
        let mut store = StateStore::open(&path).unwrap();
        store.save("gw::r1", &TrackerSnapshot::ok()).unwrap();
        store.delete("gw::r1").unwrap();
        assert_eq!(store.load_all().unwrap().len(), 0);
    }

    #[test]
    fn parse_state_err_handles_unknown_string() {
        let ok = parse_state_err("ok", 0, 0).unwrap();
        assert_eq!(ok, AlertState::Ok);
        let pending = parse_state_err("pending", 100, 0).unwrap();
        assert_eq!(pending, AlertState::Pending { since: 100 });
        let firing = parse_state_err("firing", 200, 250).unwrap();
        assert_eq!(firing, AlertState::Firing { since: 200, last_fired_at: 250 });
        let err = parse_state_err("wat", 0, 0).unwrap_err();
        assert!(matches!(err, StateStoreError::InvalidState(s) if s == "wat"));
    }

    #[test]
    fn alert_state_tracker_conversion() {
        let path = tmpfile("tracker");
        let mut store = StateStore::open(&path).unwrap();
        let mut tracker = AlertStateTracker::default();
        tracker.state = AlertState::Pending { since: 42 };
        tracker.sustained_for = std::time::Duration::from_secs(7);
        let snap = TrackerSnapshot { state: tracker.state.clone(), sustained_secs: tracker.sustained_for.as_secs() };
        store.save("gw::r1", &snap).unwrap();
        let restored = store.load_all().unwrap();
        assert_eq!(restored[0].1.state, AlertState::Pending { since: 42 });
        assert_eq!(restored[0].1.sustained_secs, 7);
    }

    // ============================================================
    // alert_history (slice 7)
    // ============================================================

    #[test]
    fn record_event_appends_history_row() {
        let path = tmpfile("event");
        let mut store = StateStore::open(&path).unwrap();
        store.record_event(
            "gw::r1", "fired", "critical", 5.0, 2.0,
            r#"{"rule":"r1","target":"gw","slo":"s","burn_rate":5.0,"threshold":2.0,"severity":"Critical","fired_at_unix":1000,"message":"m"}"#,
            1000,
        ).unwrap();
        store.record_event(
            "gw::r1", "resolved", "ok", 0.5, 2.0,
            r#"{"rule":"r1","target":"gw","slo":"s","burn_rate":0.5,"threshold":2.0,"severity":"Ok","fired_at_unix":2000,"message":"m"}"#,
            2000,
        ).unwrap();
        let all = store.list_history(None, 100).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].event, "resolved");
        assert_eq!(all[0].severity, "ok");
        assert_eq!(all[1].event, "fired");
    }

    #[test]
    fn list_history_filters_by_key_prefix() {
        let path = tmpfile("prefix");
        let mut store = StateStore::open(&path).unwrap();
        for k in ["gw::r1", "gw::r2", "openai::r1"] {
            store.record_event(
                k, "fired", "warning", 3.0, 2.0,
                r#"{"rule":"r","target":"t","slo":"s","burn_rate":3.0,"threshold":2.0,"severity":"Warning","fired_at_unix":1,"message":"m"}"#,
                1,
            ).unwrap();
        }
        let gw_only = store.list_history(Some("gw::"), 100).unwrap();
        assert_eq!(gw_only.len(), 2);
        for r in &gw_only { assert!(r.key.starts_with("gw::")); }
    }

    #[test]
    fn list_history_respects_limit() {
        let path = tmpfile("limit");
        let mut store = StateStore::open(&path).unwrap();
        for i in 0..50u64 {
            store.record_event(
                "k", "fired", "warning", 1.0, 1.0,
                r#"{"rule":"r","target":"t","slo":"s","burn_rate":1.0,"threshold":1.0,"severity":"Warning","fired_at_unix":0,"message":"m"}"#,
                i,
            ).unwrap();
        }
        let ten = store.list_history(None, 10).unwrap();
        assert_eq!(ten.len(), 10);
    }

    #[test]
    fn history_persists_across_reopen() {
        let path = tmpfile("persist");
        {
            let mut store = StateStore::open(&path).unwrap();
            store.record_event(
                "gw::r1", "fired", "critical", 4.0, 2.0,
                r#"{"rule":"r1","target":"gw","slo":"s","burn_rate":4.0,"threshold":2.0,"severity":"Critical","fired_at_unix":42,"message":"m"}"#,
                42,
            ).unwrap();
        }
        let store = StateStore::open(&path).unwrap();
        let history = store.list_history(None, 100).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].burn_rate, 4.0);
        assert_eq!(history[0].fired_at_unix, 42);
    }

}
