//! The HTTP-driven per-target poll cycle. Lives in its own module so the
//! `Monitor` construction / run-loop code stays compact.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tracing::{error, info};

use crate::alerts::{self, Severity};
use crate::metrics::{Metrics, Outcome, Sample};
use crate::ring_buffer::RingBuffer;
use crate::slo::{burn_rate, BurnWindow};
use crate::state_store::TrackerSnapshot;
use crate::target::Target;

use super::evaluate_alerts::evaluate_alerts_impl;
use super::evaluate_meta_alerts::evaluate_meta_alerts_impl;
use super::monitor::Monitor;
use super::types::{PollError, PollOutcome};

/// Poll one specific target once. Free function so it can be unit-tested
/// without going through the `Monitor` API surface.
///
/// The target URL is used as-is if it contains a path; otherwise `/health`
/// is appended. This matches the slice-1 convention so the wiremock
/// fixtures (which mount on `/health`) keep working unchanged.
pub(crate) async fn poll_once_target_impl(
    me: &Monitor,
    target: &Target,
    timeout: std::time::Duration,
) -> Result<PollOutcome, PollError> {
    let started = Instant::now();
    let url = match target.url.find("://") {
        Some(idx) if target.url[idx + 3..].contains('/') => target.url.clone(),
        _ => format!("{}/health", target.url.trim_end_matches('/')),
    };
    let res = me.inner.http.get(&url).timeout(timeout).send().await;
    let latency = started.elapsed();
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

    let sample = match res {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let outcome = if resp.status().is_success() { Outcome::Ok } else { Outcome::Error };
            Sample {
                provider: target.name.clone(),
                outcome,
                latency,
                status_code: status,
                timestamp_secs: ts,
            }
        }
        Err(e) => {
            error!(target = %target.name, error = %e, "transport error");
            Sample {
                provider: target.name.clone(),
                outcome: Outcome::Error,
                latency,
                status_code: 0,
                timestamp_secs: ts,
            }
        }
    };

    let mut m = me.inner.metrics.lock().await;
    m.record_sample(&sample);

    // Update per-target ring buffers + compute burn against each SLO.
    let mut c = me.inner.counters.lock().await;
    let tc = c.get_mut(&target.name).ok_or_else(|| {
        PollError::InvalidConfig(format!("target {} not initialised", target.name))
    })?;
    let is_success = sample.outcome == Outcome::Ok;
    tc.short.record(is_success, ts);
    tc.long.record(is_success, ts);

    let short_window = BurnWindow::FAST_BURN.long.as_secs();
    let long_window = tc.long.bucket_size_secs().max(1) * tc.long.len() as u64;
    let (s_short, f_short) = tc.short.window(short_window, ts);
    let (s_long, f_long) = tc.long.window(long_window, ts);

    let mut burn_short = 0.0_f64;
    let mut burn_long = 0.0_f64;
    for slo in &me.inner.config.slos {
        let bs = burn_rate(s_short, f_short, slo.target);
        let bl = burn_rate(s_long, f_long, slo.target);
        m.record_burn(&format!("{}::{}", target.name, slo.name), BurnWindow::FAST_BURN, bs);
        m.record_burn(&format!("{}::{}", target.name, slo.name), BurnWindow::SLOW_BURN, bl);
        burn_short = bs;
        burn_long = bl;
    }
    drop(m);

    // Evaluate alert rules (separately so the metrics lock is released).
    let payloads = evaluate_alerts_impl(me, &target.name, burn_short, burn_long, ts).await;
    // Meta-alerts run after alert evaluation so the alert_failures rows
    // recorded above (for failed webhook deliveries) are visible to the
    // next read. Bumps the Prometheus counter for each fire.
    let meta_fired = evaluate_meta_alerts_impl(me, ts).await;
    if !meta_fired.is_empty() {
        let m = me.inner.metrics.lock().await;
        for name in &meta_fired {
            // Severity isn't part of the returned names; we look up the
            // configured rule to get the severity label. Default to
            // "critical" since that's the meta-alert default.
            let severity = me
                .inner
                .config
                .meta_alerts
                .iter()
                .find(|r| &r.name == name)
                .map(|r| match r.severity {
                    Severity::Critical => "critical",
                    Severity::Warning => "warning",
                    Severity::Ok => "ok",
                })
                .unwrap_or("critical");
            m.record_meta_alert_fire(name, &target.name, severity);
            info!(
                target = %target.name,
                meta_alert = %name,
                severity = severity,
                "meta-alert fired during poll"
            );
        }
    }
    Ok(PollOutcome { sample, burn_short, burn_long, alert_payloads: payloads })
}
