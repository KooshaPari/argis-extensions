//! SQLite-backed persistent alert state.
//!
//! Why SQLite and not Postgres or a flat file:
//!   - Postgres requires a server (overkill for the substrate's scale;
//!     ~tens of rows of alert state per monitor instance).
//!   - A flat file works but is racy under concurrent ticks.
//!   - SQLite (via rusqlite with the `bundled` feature) gives us ACID
//!     transactions without external dependencies.
//!
//! The connection + SCHEMA + initial open live here. Table-specific CRUD is
//! in `alert_state`, `alert_history`, and `alert_failures` (all defined as
//! `impl StateStore` blocks in their respective submodules).

use std::path::Path;

use rusqlite::Connection;

use super::types::{StateStoreError, TrackerSnapshot};

/// The state store. Cheap to clone (wraps `Arc<Connection>` internally via
/// `Connection::open` once per monitor).
pub struct StateStore {
    /// Exposed to sibling submodules (`alert_state`, `alert_history`,
    /// `alert_failures`) so each table can keep its `impl StateStore` block.
    /// Not part of the public API.
    pub(super) conn: Connection,
}

impl StateStore {
    /// Open or create the SQLite file at `path`. Runs the schema migration.
    pub fn open(path: &Path) -> Result<Self, StateStoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                StateStoreError::Sqlite(rusqlite::Error::InvalidParameterName(
                    format!("create_dir_all({parent:?}): {e}").into(),
                ))
            })?;
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
}

/// Full schema: all three tables + their indexes. Kept in one place so
/// `StateStore::open` runs a single idempotent batch.
pub(super) const SCHEMA: &str = r#"
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

CREATE TABLE IF NOT EXISTS alert_failures (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    key             TEXT NOT NULL,        -- "{target}::{rule_name}"
    fired_at_unix   INTEGER NOT NULL,
    error           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_alert_failures_key ON alert_failures(key);
CREATE INDEX IF NOT EXISTS idx_alert_failures_fired_at ON alert_failures(fired_at_unix);
"#;

/// Internal parser used by `load_all`. Kept here next to SCHEMA so schema
/// and parser evolve together.
pub(super) fn parse_state(s: &str, since: u64, last_fired: u64) -> Result<crate::alerts::AlertState, StateStoreError> {
    match s {
        "ok" => Ok(crate::alerts::AlertState::Ok),
        "pending" => Ok(crate::alerts::AlertState::Pending { since }),
        "firing" => Ok(crate::alerts::AlertState::Firing { since, last_fired_at: last_fired }),
        other => Err(StateStoreError::InvalidState(other.to_string())),
    }
}

/// Flatten a `TrackerSnapshot` into the (state_str, since_unix, last_fired_unix)
/// tuple used by `INSERT INTO alert_state`.
pub(super) fn flatten(snap: &TrackerSnapshot) -> (&'static str, u64, u64) {
    use crate::alerts::AlertState;
    match &snap.state {
        AlertState::Ok => ("ok", 0, 0),
        AlertState::Pending { since } => ("pending", *since, 0),
        AlertState::Firing { since, last_fired_at } => ("firing", *since, *last_fired_at),
    }
}
