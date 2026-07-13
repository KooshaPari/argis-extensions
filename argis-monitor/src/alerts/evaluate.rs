//! The state-machine evaluator: takes a rule + latest burn rate + tracker
//! and returns either `Decision::None` or `Decision::Fire(payload)`.

use std::time::Duration;

use crate::alerts::payload::AlertPayload;

use super::rules::AlertRule;
use super::types::{AlertState, AlertStateTracker, Decision, Severity};

/// Evaluate one alert rule against the latest burn rate. Returns either
/// `Decision::None` (no state change worth firing on) or `Decision::Fire(payload)`.
pub fn evaluate(
    rule: &AlertRule,
    target_name: &str,
    burn: f64,
    ts: u64,
    tracker: &mut AlertStateTracker,
) -> Decision {
    let threshold = rule.threshold;
    let resolve = rule.resolve_threshold.unwrap_or(threshold / 2.0);

    match tracker.state {
        AlertState::Ok | AlertState::Pending { .. } => {
            if burn >= threshold {
                if rule.for_secs.is_zero() {
                    // Fire immediately
                    let payload = AlertPayload::firing(
                        &rule.name, target_name, &rule.slo, burn, threshold, ts,
                    );
                    tracker.state = AlertState::Firing { since: ts, last_fired_at: ts };
                    tracker.sustained_for = Duration::from_secs(0);
                    Decision::Fire(payload)
                } else if let AlertState::Pending { since } = tracker.state {
                    if ts.saturating_sub(since) >= rule.for_secs.as_secs() {
                        let payload = AlertPayload::firing(
                            &rule.name, target_name, &rule.slo, burn, threshold, ts,
                        );
                        tracker.state = AlertState::Firing { since, last_fired_at: ts };
                        tracker.sustained_for = Duration::from_secs(0);
                        Decision::Fire(payload)
                    } else {
                        Decision::None
                    }
                } else {
                    // Just crossed threshold, enter Pending
                    tracker.state = AlertState::Pending { since: ts };
                    tracker.sustained_for = Duration::from_secs(0);
                    Decision::None
                }
            } else if burn < resolve && matches!(tracker.state, AlertState::Pending { .. }) {
                // Below threshold and was pending; drop back to Ok.
                tracker.state = AlertState::Ok;
                tracker.sustained_for = Duration::from_secs(0);
                Decision::None
            } else {
                Decision::None
            }
        }
        AlertState::Firing { since, last_fired_at } => {
            if burn < resolve {
                // Resolve: fire a resolved payload and drop back to Ok.
                let payload = AlertPayload::resolved(
                    &rule.name, target_name, &rule.slo, burn, resolve, ts,
                );
                tracker.state = AlertState::Ok;
                tracker.sustained_for = Duration::from_secs(0);
                Decision::Fire(payload)
            } else if ts.saturating_sub(last_fired_at) >= rule.cooldown.as_secs() {
                // Re-fire after cooldown.
                let payload = AlertPayload::firing(
                    &rule.name, target_name, &rule.slo, burn, threshold, ts,
                );
                tracker.state = AlertState::Firing { since, last_fired_at: ts };
                tracker.sustained_for = Duration::from_secs(0);
                Decision::Fire(payload)
            } else {
                // Still in cooldown; don't re-fire.
                Decision::None
            }
        }
    }
}
