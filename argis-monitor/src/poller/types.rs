//! Shared types for the poller: PollOutcome + PollError + per-target counters
//! + the `MonitorInner` struct that all the impl-Monitor submodules read.

use std::collections::HashMap;
use std::sync::Arc;

use prometheus_client::registry::Registry;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::alerts::AlertStateTracker;
use crate::config::Config;
use crate::metrics::Metrics;
use crate::ring_buffer::RingBuffer;
use crate::state_store::{StateStore, TrackerSnapshot};
use crate::webhook;

/// One poll's outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PollOutcome {
    pub sample: crate::metrics::Sample,
    pub burn_short: f64,
    pub burn_long: f64,
    #[serde(default)]
    pub alert_payloads: Vec<crate::alerts::AlertPayload>,
}

/// Errors the poller can encounter.
#[derive(Debug, Error)]
#[allow(clippy::large_enum_variant)]
pub enum PollError {
    #[error("HTTP transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("no targets configured")]
    NoTargets,
}

/// Per-target ring buffer state.
pub struct TargetCounters {
    pub short: RingBuffer,
    pub long: RingBuffer,
}

/// Shared state behind the `Monitor` clone.
pub struct MonitorInner {
    pub config: Config,
    pub http: reqwest::Client,
    pub registry: Arc<Registry>,
    pub metrics: Arc<Mutex<Metrics>>,
    pub counters: Mutex<HashMap<String, TargetCounters>>,
    /// Per-(target, rule) state machine. Keyed by "{target}::{rule.name}".
    pub alert_trackers: Mutex<HashMap<String, AlertStateTracker>>,
    /// Last delivery report per webhook URL (for tests + ops introspection).
    pub last_delivery: Mutex<HashMap<String, webhook::DeliveryReport>>,
    /// Optional SQLite state store. When present, every alert state transition
    /// is persisted so the monitor can rehydrate after a restart.
    pub state_store: Mutex<Option<StateStore>>,
}
