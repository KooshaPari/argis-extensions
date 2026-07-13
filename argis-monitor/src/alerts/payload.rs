//! `AlertPayload` — the wire-compatible JSON envelope posted to webhooks.
//!
//! Three constructors cover the three fire paths:
//!   * `firing`     — a per-rule alert just transitioned into Firing
//!   * `resolved`   — a per-rule alert just transitioned back to Ok
//!   * `meta_alert` — a meta-alert fired on a webhook-failure pattern

use serde::{Deserialize, Serialize};

use super::types::Severity;

/// The payload POSTed to webhooks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlertPayload {
    pub rule: String,
    pub target: String,
    pub slo: String,
    pub burn_rate: f64,
    pub threshold: f64,
    pub severity: Severity,
    pub fired_at_unix: u64,
    pub message: String,
}

impl AlertPayload {
    pub fn firing(rule: &str, target: &str, slo: &str, burn: f64, threshold: f64, ts: u64) -> Self {
        Self {
            rule: rule.into(),
            target: target.into(),
            slo: slo.into(),
            burn_rate: burn,
            threshold,
            severity: Severity::from_burn(burn, threshold),
            fired_at_unix: ts,
            message: format!(
                "argis-monitor: target={target} slo={slo} burn={burn:.2}x threshold={threshold:.2}x ({} severity)",
                match Severity::from_burn(burn, threshold) {
                    Severity::Critical => "CRITICAL",
                    Severity::Warning  => "WARNING",
                    Severity::Ok       => "ok",
                }
            ),
        }
    }

    pub fn resolved(rule: &str, target: &str, slo: &str, burn: f64, resolve_threshold: f64, ts: u64) -> Self {
        Self {
            rule: rule.into(),
            target: target.into(),
            slo: slo.into(),
            burn_rate: burn,
            threshold: resolve_threshold,
            severity: Severity::Ok,
            fired_at_unix: ts,
            message: format!("argis-monitor: RESOLVED target={target} slo={slo} burn={burn:.2}x <= {resolve_threshold:.2}x"),
        }
    }

    /// Build a payload for a meta-alert fire. The meta-alert name goes in
    /// `rule`, the target in `target`, the optional reason in `slo` (which
    /// is just a free-form string in the JSON envelope), the observed
    /// failure count in `burn_rate`, the configured threshold in
    /// `threshold`, and the meta-rule's severity in `severity`.
    pub fn meta_alert(
        name: String,
        target: String,
        reason: Option<String>,
        count: f64,
        threshold: f64,
        severity: Severity,
        ts: u64,
    ) -> Self {
        let reason_str = reason.as_deref().unwrap_or("");
        Self {
            rule: name.clone(),
            target,
            slo: reason_str.to_string(),
            burn_rate: count,
            threshold,
            severity,
            fired_at_unix: ts,
            message: format!(
                "argis-monitor: META-ALERT {name} target-count={count:.0} threshold={threshold:.0}{} ({})",
                if reason_str.is_empty() { String::new() } else { format!(" reason={reason_str}") },
                match severity {
                    Severity::Critical => "CRITICAL",
                    Severity::Warning  => "WARNING",
                    Severity::Ok       => "ok",
                }
            ),
        }
    }
}
