//! Shared types for the state store: error enum + tracker snapshot.
//!
//! Kept in its own submodule so each table-specific file can `use super::types::*`
//! without pulling in connection / schema internals.

use thiserror::Error;

use crate::alerts::AlertState;

/// Errors from the state store.
#[derive(Debug, Error)]
pub enum StateStoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
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
