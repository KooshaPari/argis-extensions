//! Meta-alert evaluation: read alert_failures counts, deliver payloads via
//! webhook::deliver_all. Public API surface (evaluate_meta_alerts on Monitor)
//! is preserved by the thin wrapper in this file's caller below.

use tracing::{info, warn};

use crate::alerts;
use crate::webhook;

use super::monitor::Monitor;

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
pub(crate) async fn evaluate_meta_alerts_impl(me: &Monitor, ts: u64) -> Vec<String> {
    let rules = me.inner.config.meta_alerts.clone();
    if rules.is_empty() { return Vec::new(); }
    let mut store_guard = me.inner.state_store.lock().await;
    let store = match store_guard.as_mut() {
        Some(s) => s,
        None => {
            tracing::debug!("meta-alert evaluation skipped: no state store configured");
            return Vec::new();
        }
    };
    let mut fired = Vec::new();
    for rule in &rules {
        // The state_store key is "{target}::{rule_name}" for per-rule
        // counts. When the meta-alert specifies a `rule`, scope to it.
        let key = match &rule.rule {
            Some(r) => format!("{}::{}", rule.target, r),
            None => format!("{}::*", rule.target),
        };
        let count = match store.count_failures_in_window(&key, rule.window.as_secs(), ts) {
            Ok(n) => n,
            Err(e) => {
                warn!(meta = %rule.name, error = %e, "alert_failures count failed");
                continue;
            }
        };
        if count >= u64::from(rule.consecutive_failures) {
            info!(
                meta = %rule.name,
                target = %rule.target,
                count = count,
                threshold = rule.consecutive_failures,
                window_secs = rule.window.as_secs(),
                severity = ?rule.severity,
                "meta-alert fired"
            );

            // Build the payload. The AlertPayload struct already carries
            // the right fields for a meta-alert: we put the meta-alert
            // name in `rule`, the target in `target`, the optional
            // reason in `slo` (it's just a free-form string), and the
            // observed count + threshold into the burn_rate/threshold
            // numeric slots so downstream consumers see real numbers.
            let payload = alerts::AlertPayload::meta_alert(
                rule.name.clone(),
                rule.target.clone(),
                rule.reason.clone(),
                count as f64,
                rule.consecutive_failures as f64,
                rule.severity,
                ts,
            );

            // Resolve webhooks: prefer meta-rule's own, fall back to a
            // matching AlertRule's webhooks for the same target.
            let webhook_targets: Vec<alerts::WebhookTarget> = if !rule.webhooks.is_empty() {
                rule.webhooks.clone()
            } else {
                me.inner.config.alert_rules.iter()
                    .find(|ar| {
                        ar.name == rule.rule.clone().unwrap_or_default()
                            && me.inner.config.targets.iter().any(|t| t.name == ar.slo || t.name == rule.target)
                    })
                    .map(|ar| ar.webhooks.clone())
                    .unwrap_or_default()
            };

            if webhook_targets.is_empty() {
                warn!(
                    meta = %rule.name,
                    target = %rule.target,
                    "meta-alert fired but no webhook targets configured"
                );
            } else {
                let reports = webhook::deliver_all(
                    &me.inner.http, &webhook_targets, &payload,
                ).await;
                let mut last = me.inner.last_delivery.lock().await;
                for r in reports {
                    last.insert(r.url.clone(), r);
                }
            }

            fired.push(rule.name.clone());
        }
    }
    fired
}
