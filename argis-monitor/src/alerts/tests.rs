//! Unit tests for the alert domain. Lives in its own submodule so the
//! other submodule files stay focused on production code.

use std::time::Duration;

use super::evaluate::evaluate;
use super::payload::AlertPayload;
use super::rules::AlertRule;
use super::types::{AlertState, AlertStateTracker, Decision, Severity};

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

#[test]
fn meta_alert_payload_preserves_count_threshold_severity() {
    let p = AlertPayload::meta_alert(
        "chat_burn_outage".into(),
        "gateway".into(),
        Some("webhook delivery down".into()),
        3.0,
        3.0,
        Severity::Critical,
        1_750_000_000,
    );
    assert_eq!(p.rule, "chat_burn_outage");
    assert_eq!(p.target, "gateway");
    assert_eq!(p.slo, "webhook delivery down");
    assert_eq!(p.burn_rate, 3.0);
    assert_eq!(p.threshold, 3.0);
    assert_eq!(p.severity, Severity::Critical);
    assert_eq!(p.fired_at_unix, 1_750_000_000);
    assert!(p.message.contains("META-ALERT chat_burn_outage"));
}
