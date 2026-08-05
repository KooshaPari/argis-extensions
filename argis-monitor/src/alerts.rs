//! Alert rules + evaluator.
//!
//! A rule fires when the per-target burn rate (from the ring buffer) crosses
//! `threshold` for `for_secs` consecutive seconds. The first firing posts the
//! alert payload to the configured webhook URL(s); subsequent fires within
//! `cooldown_secs` are dropped (rate-limiting). When `burn_rate` returns below
//! `resolve_threshold`, the alert transitions back to OK and the next firing
//! is allowed immediately (resolve-on-recovery is the standard SRE pattern).

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where to send an alert payload when a rule fires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebhookTarget {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

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
    #[serde(default, deserialize_with = "opt_seconds_as_duration::deserialize", serialize_with = "opt_seconds_as_duration::serialize")]
    pub window: Option<Duration>,
    /// Sustained-burn duration before the rule fires. Defaults to 0s (fire
    /// immediately on threshold crossing).
    #[serde(default, with = "seconds_as_duration")]
    pub for_secs: Duration,
    /// Minimum seconds between consecutive fires. Default 300s (5 min).
    #[serde(default = "default_cooldown_secs", with = "seconds_as_duration")]
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
}

/// State machine for one (rule, target, slo) combination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertState {
    /// Below the threshold. No alerts in flight.
    Ok,
    /// Above the threshold but hasn't been sustained long enough.
    Pending { since: u64 },
    /// Currently firing. `last_fired_at` gates cooldown.
    Firing { since: u64, last_fired_at: u64 },
}

#[derive(Debug, Clone)]
pub struct AlertStateTracker {
    pub state: AlertState,
    pub sustained_for: Duration,
}

impl Default for AlertStateTracker {
    fn default() -> Self {
        Self { state: AlertState::Ok, sustained_for: Duration::from_secs(0) }
    }
}

/// Decision returned by the evaluator.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Stay silent.
    None,
    /// Fire an alert payload (or its resolve counterpart).
    Fire(AlertPayload),
}

/// Evaluate one (rule, target, slo) tick. `burn` is the current burn rate.
/// `ts` is the wall-clock seconds; `last_fired_at` is the cooldown anchor
/// (or `None` if no prior fire).
pub fn evaluate(
    rule: &AlertRule,
    target: &str,
    burn: f64,
    ts: u64,
    tracker: &mut AlertStateTracker,
) -> Decision {
    let resolve = rule.resolve_threshold.unwrap_or(rule.threshold / 2.0);
    match tracker.state {
        AlertState::Ok => {
            if burn >= rule.threshold {
                if rule.for_secs.is_zero() {
                    tracker.state = AlertState::Firing { since: ts, last_fired_at: ts };
                    Decision::Fire(AlertPayload::firing(
                        &rule.name, target, &rule.slo, burn, rule.threshold, ts,
                    ))
                } else {
                    tracker.state = AlertState::Pending { since: ts };
                    tracker.sustained_for = Duration::from_secs(0);
                    Decision::None
                }
            } else {
                Decision::None
            }
        }
        AlertState::Pending { since } => {
            if burn < rule.threshold {
                tracker.state = AlertState::Ok;
                tracker.sustained_for = Duration::from_secs(0);
                Decision::None
            } else {
                // Use wall-clock elapsed time so delayed polls do not distort
                // the configured sustained-burn duration.
                tracker.sustained_for = Duration::from_secs(ts.saturating_sub(since));
                if tracker.sustained_for >= rule.for_secs {
                    tracker.state = AlertState::Firing { since, last_fired_at: ts };
                    let payload = AlertPayload::firing(&rule.name, target, &rule.slo, burn, rule.threshold, ts);
                    Decision::Fire(payload)
                } else {
                    Decision::None
                }
            }
        }
        AlertState::Firing { since, last_fired_at } => {
            if burn <= resolve {
                let payload = AlertPayload::resolved(&rule.name, target, &rule.slo, burn, resolve, ts);
                tracker.state = AlertState::Ok;
                tracker.sustained_for = Duration::from_secs(0);
                return Decision::Fire(payload);
            }
            let since_fire = ts.saturating_sub(last_fired_at);
            if since_fire >= rule.cooldown.as_secs() {
                tracker.state = AlertState::Firing { since, last_fired_at: ts };
                let payload = AlertPayload::firing(&rule.name, target, &rule.slo, burn, rule.threshold, ts);
                Decision::Fire(payload)
            } else {
                Decision::None
            }
        }
    }
}

mod opt_seconds_as_duration {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d { Some(d) => s.serialize_u64(d.as_secs()), None => s.serialize_none() }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum R { S(u64), T(String), N }
        let opt: Option<R> = Option::deserialize(d)?;
        Ok(match opt {
            None | Some(R::N) => None,
            Some(R::S(n)) => Some(Duration::from_secs(n)),
            Some(R::T(t)) => Some(parse_human(&t).map_err(serde::de::Error::custom)?),
        })
    }
    fn parse_human(s: &str) -> Result<Duration, String> {
        let s = s.trim();
        let (num, unit) = s.split_at(s.len().saturating_sub(1));
        let n: u64 = num.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
        let mul = match unit {
            "s" => 1, "m" => 60, "h" => 3600, "d" => 86_400,
            _ => return Err(format!("unknown duration unit: {unit}")),
        };
        Ok(Duration::from_secs(n * mul))
    }
}

mod seconds_as_duration {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> { s.serialize_u64(d.as_secs()) }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum R { S(u64), T(String) }
        match R::deserialize(d)? {
            R::S(n) => Ok(Duration::from_secs(n)),
            R::T(t) => parse_human(&t).map_err(serde::de::Error::custom),
        }
    }
    fn parse_human(s: &str) -> Result<Duration, String> {
        let s = s.trim();
        let (num, unit) = s.split_at(s.len().saturating_sub(1));
        let n: u64 = num.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
        let mul = match unit {
            "s" => 1, "m" => 60, "h" => 3600, "d" => 86_400,
            _ => return Err(format!("unknown duration unit: {unit}")),
        };
        Ok(Duration::from_secs(n * mul))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_below_threshold_does_not_fire() {
        let rule = AlertRule { name: "r".into(), slo: "s".into(), threshold: 2.0, ..Default::default() };
        let mut t = AlertStateTracker::default();
        assert_eq!(evaluate(&rule, "gateway", 0.5, 100, &mut t), Decision::None);
        assert_eq!(t.state, AlertState::Ok);
    }

    #[test]
    fn crossing_threshold_enters_pending_not_firing() {
        let rule = AlertRule { name: "r".into(), slo: "s".into(), threshold: 2.0, for_secs: Duration::from_secs(30), ..Default::default() };
        let mut t = AlertStateTracker::default();
        let d = evaluate(&rule, "gateway", 3.0, 100, &mut t);
        assert_eq!(d, Decision::None);
        assert!(matches!(t.state, AlertState::Pending { .. }));
    }

    #[test]
    fn sustained_burn_promotes_to_firing() {
        let rule = AlertRule { name: "r".into(), slo: "s".into(), threshold: 2.0, for_secs: Duration::from_secs(5), cooldown: Duration::from_secs(60), ..Default::default() };
        let mut t = AlertStateTracker { state: AlertState::Pending { since: 100 }, sustained_for: Duration::from_secs(5) };
        let d = evaluate(&rule, "gateway", 3.0, 106, &mut t);
        assert!(matches!(d, Decision::Fire(_)));
        assert!(matches!(t.state, AlertState::Firing { .. }));
    }

    #[test]
    fn cooldown_suppresses_repeat_fires() {
        let rule = AlertRule { name: "r".into(), slo: "s".into(), threshold: 2.0, for_secs: Duration::from_secs(0), cooldown: Duration::from_secs(300), ..Default::default() };
        let mut t = AlertStateTracker { state: AlertState::Firing { since: 100, last_fired_at: 100 }, sustained_for: Duration::from_secs(60) };
        // 60s after last fire, still in cooldown
        assert_eq!(evaluate(&rule, "gateway", 3.0, 160, &mut t), Decision::None);
        // 301s after last fire, cooldown elapsed, re-fires
        let d = evaluate(&rule, "gateway", 3.0, 401, &mut t);
        assert!(matches!(d, Decision::Fire(_)));
    }

    #[test]
    fn resolve_emits_resolve_payload() {
        let rule = AlertRule { name: "r".into(), slo: "s".into(), threshold: 2.0, resolve_threshold: Some(1.0), for_secs: Duration::from_secs(0), ..Default::default() };
        let mut t = AlertStateTracker { state: AlertState::Firing { since: 100, last_fired_at: 100 }, sustained_for: Duration::from_secs(60) };
        let d = evaluate(&rule, "gateway", 0.5, 200, &mut t);
        match d {
            Decision::Fire(p) => {
                assert_eq!(p.severity, Severity::Ok);
                assert!(p.message.contains("RESOLVED"));
            }
            _ => panic!("expected resolve payload"),
        }
        assert_eq!(t.state, AlertState::Ok);
    }

    #[test]
    fn severity_escalates_at_2x_threshold() {
        assert_eq!(Severity::from_burn(1.5, 1.0), Severity::Warning);
        assert_eq!(Severity::from_burn(2.0, 1.0), Severity::Warning);
        assert_eq!(Severity::from_burn(2.5, 1.0), Severity::Critical);
        assert_eq!(Severity::from_burn(0.5, 1.0), Severity::Ok);
    }
}
