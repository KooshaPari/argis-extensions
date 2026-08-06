//! Per-rule alert evaluation: state machine + webhook delivery + persistence.

use tracing::warn;

use crate::alerts::{self, AlertPayload, AlertStateTracker, Decision};
use crate::state_store::TrackerSnapshot;
use crate::suppression;
use crate::webhook;

use super::monitor::Monitor;

/// Evaluate every alert rule against the latest burn rates. Returns the
/// list of payloads that fired (already delivered via webhooks).
///
/// Free function so it can be invoked from `poll_loop` without going through
/// the public `Monitor::evaluate_alerts` API.
pub(crate) async fn evaluate_alerts_impl(
    me: &Monitor,
    target_name: &str,
    burn_short: f64,
    burn_long: f64,
    ts: u64,
) -> Vec<AlertPayload> {
    let inner = me.inner.load();
    let mut fired = Vec::new();
    let mut store = inner.state_store.lock().await;
    // Slice 34: snapshot the disabled set up front (avoids holding the
    // Mutex across the await-heavy evaluate_alerts_impl body and prevents
    // re-entry deadlock when the breaker branch re-locks the same Mutex).
    let disabled: std::collections::HashSet<String> = inner
        .auto_disabled_rules
        .lock()
        .await
        .iter()
        .cloned()
        .collect();
    for rule in &inner.config.alert_rules {
        let key = format!("{}::{}", target_name, rule.name);
        if disabled.contains(&key) { continue; }
        let burn = match rule.window {
            Some(w) if w >= crate::slo::BurnWindow::SLOW_BURN.long => burn_long,
            _ => burn_short,
        };
        let key = format!("{}::{}", target_name, rule.name);
        let snap;
        {
            let mut trackers = inner.alert_trackers.lock().await;
            let tracker = trackers.entry(key.clone()).or_insert_with(AlertStateTracker::default);
            match alerts::evaluate(rule, target_name, burn, ts, tracker) {
                Decision::Fire(payload) => {
                    // Suppression check. A matching window swallows the
                    // webhook delivery but the state machine still
                    // transitions (so the alert would have fired is
                    // visible in metrics + the alert_history table).
                    let window_name = suppression::is_suppressed(
                        &inner.config.alert_windows,
                        target_name,
                        &rule.name,
                        ts,
                    );
                    if let Some(wname) = &window_name {
                        tracing::info!(
                            target = %target_name,
                            rule = %rule.name,
                            window = %wname,
                            burn = burn,
                            "alert suppressed by window"
                        );
                    } else {
                        let reports = webhook::deliver_all(&inner.http, &rule.webhooks, &payload).await;
                        let mut last = inner.last_delivery.lock().await;
                        for r in reports {
                            if !r.success {
                                // Seed the meta-alert pipeline. The key
                                // is "{target}::{rule.name}" so the meta-
                                // alert layer can scope by rule when it
                                // wants to (or use a "{target}::*" key
                                // for the unscoped variant). We rely on
                                // the persisted row rather than an in-
                                // memory counter so failures survive a
                                // monitor restart.
                                if let Some(s) = store.as_mut() {
                                    let msg = r.error.as_deref().unwrap_or("webhook delivery failed");
                                    if let Err(e) = s.record_alert_failure(&key, ts, msg) {
                                        warn!(target = %target_name, rule = %rule.name, error = %e, "alert failure record failed");
                                    }
                                }
                            }
                            last.insert(r.url.clone(), r);
                        }
                    }
                    fired.push(payload);

                    // Slice 34: per-rule auto-disable (circuit breaker).
                    // Append this fire's timestamp to the rolling deque, prune
                    // anything outside `auto_disable_window`, then if the
                    // count crosses `auto_disable_after`, auto-disable the rule
                    // and emit metrics + a structured log line.
                    if let Some(after) = rule.auto_disable_after {
                        let window = rule.auto_disable_window.as_secs();
                        let inner = me.inner.load();
                        let mut history = inner.rule_fire_history.lock().await;
                        let entry = history.entry(key.clone()).or_default();
                        let cutoff = ts.saturating_sub(window);
                        while entry.front().map_or(false, |t| *t <= cutoff) {
                            entry.pop_front();
                        }
                        entry.push_back(ts);
                        let count = entry.len() as u32;
                        if count >= after {
                            let mut disabled = inner.auto_disabled_rules.lock().await;
                            if disabled.insert(key.clone()) {
                                let m = inner.metrics.lock().await;
                                m.record_rule_auto_disabled(&rule.name, target_name);
                                m.set_rule_active(&rule.name, target_name, false);
                                drop(m);
                                tracing::warn!(
                                    target = %target_name,
                                    rule = %rule.name,
                                    fires = count,
                                    window_secs = window,
                                    threshold = after,
                                    "alert rule auto-disabled by circuit breaker"
                                );
                            }
                        }
                    }
                }
                Decision::None => {}
            }
            snap = TrackerSnapshot {
                state: tracker.state.clone(),
                sustained_secs: tracker.sustained_for.as_secs(),
            };
        }
        // Capture fired-meta for the alert_history insert below.
        let fired_meta = fired.last().map(|p| (p.rule.clone(), p.severity, p.burn_rate, p.threshold, p.fired_at_unix));
        // Persist outside the trackers lock to avoid contention.
        if let Some(s) = store.as_mut() {
            if let Err(e) = s.save(&key, &snap) {
                warn!(target = %target_name, rule = %rule.name, error = %e, "state store save failed");
            }
            if let Some((rule_name, severity, burn, threshold, ts)) = &fired_meta {
                let payload_json = serde_json::to_string(&serde_json::json!({
                    "rule": rule_name,
                    "target": target_name,
                    "slo": &rule.slo,
                    "burn_rate": burn,
                    "threshold": threshold,
                    "severity": format!("{:?}", severity).to_lowercase(),
                    "fired_at_unix": ts,
                })).unwrap_or_default();
                let event = match snap.state {
                    crate::alerts::AlertState::Ok => "resolved",
                    _ => "fired",
                };
                let severity_str = format!("{:?}", severity).to_lowercase();
                if let Err(e) = s.record_event(
                    &key, event, &severity_str,
                    *burn, *threshold, &payload_json, *ts,
                ) {
                    warn!(target = %target_name, rule = %rule_name, error = %e, "alert history record failed");
                }
            }
        }
    }
    fired
}
