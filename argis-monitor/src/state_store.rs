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
    #[error("invalid state string in DB: {0}")]
    InvalidState(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
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
        let rows = stmt.query_and_then([], |row| {
            let key: String = row.get(0)?;
            let state_str: String = row.get(1)?;
            let since_unix: u64 = row.get(2)?;
            let last_fired_unix: u64 = row.get(3)?;
            let sustained_secs: u64 = row.get(4)?;
            let state = parse_state_err(&state_str, since_unix, last_fired_unix)?;
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
"#;

/// Parse a persisted state string and surface corrupt rows as
/// `StateStoreError::InvalidState`.
pub fn parse_state_err(s: &str, since: u64, last_fired: u64) -> Result<AlertState, StateStoreError> {
    match s {
        "ok" => Ok(AlertState::Ok),
        "pending" => Ok(AlertState::Pending { since }),
        "firing" => Ok(AlertState::Firing { since, last_fired_at: last_fired }),
        other => Err(StateStoreError::InvalidState(other.to_string())),
    }
}

fn flatten(snap: &TrackerSnapshot) -> (&'static str, u64, u64) {
    match &snap.state {
        AlertState::Ok => ("ok", 0, 0),
        AlertState::Pending { since } => ("pending", *since, 0),
        AlertState::Firing { since, last_fired_at } => ("firing", *since, *last_fired_at),
    }
}

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
}
