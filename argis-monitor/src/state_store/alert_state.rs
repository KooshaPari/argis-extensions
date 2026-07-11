//! alert_state CRUD: the per-(target, rule) state machine row.

use rusqlite::params;

use super::connection::flatten;
use super::types::{StateStoreError, TrackerSnapshot};
use super::StateStore;

impl StateStore {
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
    /// the config; we don't want stale rows hanging around forever.
    pub fn delete(&mut self, key: &str) -> Result<(), StateStoreError> {
        self.conn.execute("DELETE FROM alert_state WHERE key = ?1", params![key])?;
        Ok(())
    }
}
