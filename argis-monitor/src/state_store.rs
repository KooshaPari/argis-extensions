//! SQLite-backed persistent alert state.
//!
//! Decomposed into focused submodules (per the file-size mandate):
//!   - `types`        — `StateStoreError` + `TrackerSnapshot`
//!   - `connection`   — `StateStore` struct + `open` + `load_all` + SCHEMA
//!   - `alert_state`  — CRUD for the per-(target, rule) state machine
//!   - `alert_history` — append-only event log + `AlertHistoryRow`
//!   - `alert_failures` — webhook delivery failure log (slice 18)
//!   - `tests`        — unit tests
//!
//! No logic in this file; re-exports preserve the original API.

mod alert_failures;
mod alert_history;
mod alert_state;
mod connection;
mod tests;
mod types;

pub use alert_history::AlertHistoryRow;
pub use connection::StateStore;
pub use types::{StateStoreError, TrackerSnapshot};
