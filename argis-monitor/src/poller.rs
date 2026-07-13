//! Async poller: drives one tokio task per target, accumulates samples into
//! the shared `Metrics` registry, and feeds the `RingBuffer` per target for
//! proper SLO multi-window burn-rate computation.
//!
//! Decomposed into focused submodules (per the file-size mandate):
//!   - `types`                — `PollOutcome`, `PollError`, `TargetCounters`, `MonitorInner`
//!   - `monitor`              — `Monitor` struct + `new()` + `run()` + `run_target()`
//!   - `poll_loop`            — the HTTP-driven per-target poll cycle
//!   - `evaluate_alerts`      — per-rule alert state machine + webhook delivery
//!   - `evaluate_meta_alerts` — meta-alert evaluation + webhook delivery
//!
//! No logic in this file; just module declarations + re-exports +
//! thin public-API wrappers around the free functions in submodules.

mod evaluate_alerts;
mod evaluate_meta_alerts;
mod monitor;
mod poll_loop;
mod types;

use std::time::Duration;

use tracing::instrument;

pub use monitor::Monitor;
pub use types::{MonitorInner, PollError, PollOutcome, TargetCounters};

use crate::config::SLO;
use crate::slo::BurnWindow;
use crate::target::Target;

// =====================================================================
// Public API wrappers (preserve the original Monitor API surface).
// =====================================================================

impl Monitor {
    /// Poll one specific target once. Public wrapper around
    /// `poll_loop::poll_once_target_impl`.
    pub async fn poll_once_target(&self, target: &Target, timeout: Duration)
        -> Result<PollOutcome, PollError>
    {
        poll_loop::poll_once_target_impl(self, target, timeout).await
    }

    /// Backward-compat helper: poll the first target once.
    pub async fn poll_once(&self) -> Result<PollOutcome, PollError> {
        let target = self.inner.load().config.targets.first()
            .ok_or(PollError::NoTargets)?
            .clone();
        self.poll_once_target(&target, self.inner.load().config.poll_timeout).await
    }

    pub fn windows(&self) -> &'static [BurnWindow] {
        &[BurnWindow::FAST_BURN, BurnWindow::SLOW_BURN]
    }

    /// Evaluate every meta-alert rule and deliver the resulting payloads.
    /// Returns the names of the meta-alerts that fired in this tick.
    ///
    /// A meta-alert fires when the alert_failures table holds at least
    /// `consecutive_failures` rows for the target (and optional specific
    /// rule) within the trailing `window` seconds. This is the "Bifrost-
    /// backed" piece: persistent failure history that survives restarts.
    ///
    /// Delivery: each fired meta-alert is wrapped in an `AlertPayload` and
    /// POSTed via `webhook::deliver_all`. Webhook targets are taken from
    /// `MetaAlertRule.webhooks` when non-empty; otherwise the matching
    /// `AlertRule.webhooks` for the same target is used as the fallback so
    /// operators don't have to configure meta-alert webhooks twice.
    #[instrument(skip(self))]
    pub async fn evaluate_meta_alerts(&self, ts: u64) -> Vec<String> {
        evaluate_meta_alerts::evaluate_meta_alerts_impl(self, ts).await
    }
}

impl SLO {
    pub fn with_window_secs(mut self, secs: u64) -> Self { self.window_secs = secs; self }
    pub fn with_target(mut self, target: f64) -> Self { self.target = target; self }
}

