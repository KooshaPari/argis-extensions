//! Alert rules: `AlertRule` (per-burn-rate) and `MetaAlertRule` (webhook-failure-driven).
//!
//! Both kinds of rule carry the same webhook-target list, so the delivery
//! path can be shared (see `poller::evaluate_alerts` / `evaluate_meta_alerts`).

use std::time::Duration;
use serde::{Deserialize, Serialize};

use super::types::Severity;
use super::webhook_target::WebhookTarget;

/// One alert rule. Evaluated independently per (target x SLO).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertRule {
    pub name: String,
    /// SLO name to track (must match a name in `Config.slos`).
    pub slo: String,
    /// Burn-rate threshold (multiplier of error budget). Common values:
    ///   1.0  = "on track" / informational
    ///   2.0  = "burning 2x faster than sustainable"
    ///   14.4 = Google SRE "fast burn" page-out threshold
    pub threshold: f64,
    /// Burn-rate at or below which the alert is considered resolved.
    /// Default: half of `threshold`.
    #[serde(default)]
    pub resolve_threshold: Option<f64>,
    /// Window over which the burn rate is computed. Defaults to FAST_BURN.long.
    #[serde(default, deserialize_with = "super::serde_mods::opt_seconds_as_duration::deserialize", serialize_with = "super::serde_mods::opt_seconds_as_duration::serialize")]
    pub window: Option<Duration>,
    /// Sustained-burn duration before the rule fires. Defaults to 0s (fire
    /// immediately on threshold crossing).
    #[serde(default, with = "super::serde_mods::seconds_as_duration")]
    pub for_secs: Duration,
    /// Minimum seconds between consecutive fires. Default 300s (5 min).
    #[serde(default = "default_cooldown_secs", with = "super::serde_mods::seconds_as_duration")]
    pub cooldown: Duration,
    /// Webhooks to notify when the rule fires.
    #[serde(default)]
    pub webhooks: Vec<WebhookTarget>,
}

fn default_cooldown_secs() -> Duration { Duration::from_secs(300) }

impl Default for AlertRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            slo: String::new(),
            threshold: 1.0,
            resolve_threshold: None,
            window: None,
            for_secs: Duration::from_secs(0),
            cooldown: default_cooldown_secs(),
            webhooks: Vec::new(),
        }
    }
}

/// A meta-alert rule: fires when `consecutive_failures` webhook delivery
/// failures occur within `window` seconds for a given (target, rule) pair.
///
/// Meta-alerts are a separate layer above per-rule `AlertRule`s: they catch
/// *patterns of failure* (e.g. webhook target down for 5+ minutes) even when
/// no burn-rate threshold has crossed. The alert_failures table in the state
/// store is the source of truth (slice 18).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetaAlertRule {
    /// Stable identifier. Used as the meta-alert name + payload.rule.
    pub name: String,
    /// Target name to monitor (matches `Target.name`).
    pub target: String,
    /// Optional: only count failures for this specific rule on the target.
    /// When `None`, every failed webhook for `target` counts.
    #[serde(default)]
    pub rule: Option<String>,
    /// Fire when this many failures occur within `window`. Default: 3.
    #[serde(default = "default_meta_consecutive")]
    pub consecutive_failures: u32,
    /// Sliding window over which failures are counted. Default: 300s.
    #[serde(default = "default_meta_window", with = "super::serde_mods::seconds_as_duration")]
    pub window: Duration,
    /// Severity emitted when the meta-alert fires. Defaults to Critical
    /// because the failure pattern itself is the signal.
    #[serde(default = "default_meta_severity")]
    pub severity: Severity,
    /// Optional human-readable reason shown in the alert payload.
    #[serde(default)]
    pub reason: Option<String>,
    /// Webhooks to notify when the meta-alert fires. Falls back to the
    /// owning `AlertRule`'s webhooks when empty (caller decides).
    #[serde(default)]
    pub webhooks: Vec<WebhookTarget>,
}

fn default_meta_consecutive() -> u32 { 3 }
fn default_meta_window() -> Duration { Duration::from_secs(300) }
fn default_meta_severity() -> Severity { Severity::Critical }

impl Default for MetaAlertRule {
    fn default() -> Self {
        Self {
            name: String::new(),
            target: String::new(),
            rule: None,
            consecutive_failures: default_meta_consecutive(),
            window: default_meta_window(),
            severity: default_meta_severity(),
            reason: None,
            webhooks: Vec::new(),
        }
    }
}
