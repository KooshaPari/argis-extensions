//! argis-monitor: Observable Integration substrate for bifrost-extensions.
//!
//! Implements Tenet 4 of the bifrost-extensions charter: "Extension behavior
//! is fully observable. Metrics, logs, and traces flow through the same
//! pipeline as core components."
//!
//! Architecture (see docs/ARCHITECTURE.md):
//!
//! ```text
//!   Bifrost gateway
//!        |  HTTP /health, /v1/chat/completions
//!        v
//!   argis-monitor poller  --(every poll_interval)-->  Bifrost gateway
//!        |
//!        v
//!   SLO burn-rate calculator  (slos.rs)
//!        |
//!        v
//!   Prometheus exposition  -->  /metrics on :9090  (axum)
//! ```
//!
//! Library usage:
//!
//! ```no_run
//! use argis_monitor::{Monitor, Config, SLO};
//! # async fn run() -> anyhow::Result<()> {
//! let monitor = Monitor::new(Config::default()
//!     .with_target_url("http://127.0.0.1:8080")
//!     .with_poll_interval_secs(15)
//!     .with_slo(SLO {
//!         name: "chat_completions_p99".into(),
//!         window_secs: 30 * 24 * 3600,
//!         target: 0.999,
//!     }))?;
//! monitor.run().await?;
//! # Ok(()) }
//! ```
//!
//! CLI usage: see `argis-monitor --help`.


pub mod alerts;
pub mod config;
pub mod dashboard;
pub mod exporter;
pub mod metrics;
pub mod poller;
pub mod push;
pub mod ring_buffer;
pub mod slo;
pub mod state_store;
pub mod target;
pub mod webhook;

pub use alerts::{AlertPayload, AlertRule, AlertState, AlertStateTracker, Decision, Severity, WebhookTarget};
pub use dashboard::{load_and_summarize, DashboardSummary};
pub use push::{push_to, run_pusher, PushError};
pub use state_store::{StateStore, TrackerSnapshot, StateStoreError};
pub use config::{Config, SLO};
pub use ring_buffer::{Bucket, RingBuffer};
pub use target::Target;
pub use webhook::{deliver_all, DeliveryReport};
pub use metrics::{Outcome, Sample};
pub use poller::{Monitor, PollError, PollOutcome};
pub use slo::{burn_rate, BurnWindow};

/// Re-export of the crate version (matches `Cargo.toml`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
