//! alert_failures CRUD: per-(target, rule) webhook delivery failure log.
//!
//! Seeded by the poller whenever a webhook delivery comes back non-success.
//! Read by `evaluate_meta_alerts` to decide if a meta-alert should fire.

use super::types::StateStoreError;
use super::StateStore;

impl StateStore {
    /// Record one alert webhook delivery failure. Called by the poller
    /// after `webhook::deliver_all` returns a non-success report.
    pub fn record_alert_failure(
        &mut self,
        key: &str,
        fired_at_unix: u64,
        error: &str,
    ) -> Result<(), StateStoreError> {
        let ts: i64 = if fired_at_unix > i64::MAX as u64 {
            i64::MAX
        } else {
            fired_at_unix as i64
        };
        self.conn.execute(
            "INSERT INTO alert_failures (key, fired_at_unix, error) VALUES (?1, ?2, ?3)",
            rusqlite::params![key, ts, error],
        )?;
        Ok(())
    }

    /// Count alert_failures rows for `key` that occurred within the trailing
    /// `window_secs` ending at `now_unix`.
    pub fn count_failures_in_window(
        &self,
        key: &str,
        window_secs: u64,
        now_unix: u64,
    ) -> Result<u64, StateStoreError> {
        // Saturate each u64 -> i64 cast independently to avoid overflow.
        let now_i: i64 = if now_unix > i64::MAX as u64 {
            i64::MAX
        } else {
            now_unix as i64
        };
        let win_i: i64 = if window_secs > i64::MAX as u64 {
            i64::MAX
        } else {
            window_secs as i64
        };
        let since_i = now_i.saturating_sub(win_i);
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM alert_failures
             WHERE key = ?1 AND fired_at_unix >= ?2",
            rusqlite::params![key, since_i],
            |row| row.get(0),
        )?;
        Ok(n.max(0) as u64)
    }

    /// Delete alert_failures rows older than `older_than_unix` (Unix seconds).
    /// Returns the number of rows deleted.
    pub fn prune_alert_failures(
        &mut self,
        older_than_unix: u64,
    ) -> Result<u64, StateStoreError> {
        let threshold: i64 = if older_than_unix > i64::MAX as u64 {
            i64::MAX
        } else {
            older_than_unix as i64
        };
        let deleted = self.conn.execute(
            "DELETE FROM alert_failures WHERE fired_at_unix < ?1",
            rusqlite::params![threshold],
        )?;
        Ok(deleted as u64)
    }
}
