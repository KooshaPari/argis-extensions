//! Lightweight data types: severity, alert state, state tracker, decision.

use serde::{Deserialize, Serialize};

/// Severity of an alert firing. `Ok` is emitted on resolve.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity { Ok, Warning, Critical }

impl Severity {
    pub fn from_burn(burn: f64, threshold: f64) -> Self {
        if burn > threshold * 2.0 { Severity::Critical }
        else if burn >= threshold { Severity::Warning }
        else { Severity::Ok }
    }
}

/// State machine for one (rule, target, slo) combination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertState {
    /// Below the threshold. No alerts in flight.
    Ok,
    /// Above the threshold but hasn't been sustained long enough.
    Pending { since: u64 },
    /// Sustained long enough; firing. `last_fired_at` is the unix-seconds of
    /// the last emit (for cooldown accounting).
    Firing { since: u64, last_fired_at: u64 },
}

/// Tracks state + sustained-burn time for one (target, rule) pair.
#[derive(Debug, Clone)]
pub struct AlertStateTracker {
    pub state: AlertState,
    pub sustained_for: std::time::Duration,
}

impl Default for AlertStateTracker {
    fn default() -> Self {
        Self {
            state: AlertState::Ok,
            sustained_for: std::time::Duration::from_secs(0),
        }
    }
}

/// Result of evaluating one alert rule against the latest burn rate.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// No state change worth firing on.
    None,
    /// Fire this payload (resolved or alert; the payload itself tells you).
    Fire(super::payload::AlertPayload),
}
