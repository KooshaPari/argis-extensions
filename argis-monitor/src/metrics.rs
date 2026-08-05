//! Prometheus metrics registry + sample type.
//!
//! ## Unit scaling
//!
//! The `prometheus-client` crate's `Gauge` only supports integer atomic
//! types (i64/u64). To preserve fractional precision for SLO targets and
//! burn rates, the monitor stores:
//!
//! - `argis_monitor_slo_target`: per-mille integer (0.999 -> 999)
//! - `argis_monitor_burn_rate`: basis-points x 100 (1.0 -> 10000, 14.4 -> 144000)
//!
//! Both HELP strings document the scaling; Prometheus consumers divide.

use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family, gauge::Gauge, histogram::Histogram},
    registry::Registry,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::slo::BurnWindow;

// =====================================================================
// Typed label sets.
// =====================================================================

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct PollLabels {
    pub provider: String,
    pub outcome: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ErrorLabels {
    pub provider: String,
    pub error_kind: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ProviderLabels {
    pub provider: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct SloLabels {
    pub slo: String,
    pub window: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct SloOnlyLabels {
    pub slo: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct InfoLabels {
    pub target: String,
    pub version: String,
}

// =====================================================================
// Sample type.
// =====================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub provider: String,
    pub outcome: Outcome,
    pub latency: Duration,
    pub status_code: u16,
    #[serde(with = "ts_secs")]
    pub timestamp_secs: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome { Ok, Error }

// =====================================================================
// Metrics.
// =====================================================================

pub struct Metrics {
    pub polls_total: Family<PollLabels, Counter>,
    pub poll_errors_total: Family<ErrorLabels, Counter>,
    pub poll_duration: Family<ProviderLabels, Histogram>,
    pub last_poll_ts: Family<ProviderLabels, Gauge>,
    pub up: Family<ProviderLabels, Gauge>,
    pub burn_rate: Family<SloLabels, Gauge>,
    pub slo_target: Family<SloOnlyLabels, Gauge>,
    pub target_info: Family<InfoLabels, Counter>,
}

impl Metrics {
    pub fn new(registry: &mut Registry, targets: &[String]) -> Self {
        let polls_total = Family::<PollLabels, Counter>::default();
        let poll_errors_total = Family::<ErrorLabels, Counter>::default();
        // Histogram bucket boundaries in seconds (f64); we observe `latency.as_secs_f64()`.
        let poll_duration = Family::<ProviderLabels, Histogram>::new_with_constructor(|| {
            Histogram::new(
                [0.005_f64, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
                    .into_iter(),
            )
        });
        let last_poll_ts = Family::<ProviderLabels, Gauge>::default();
        let up = Family::<ProviderLabels, Gauge>::default();
        let burn_rate = Family::<SloLabels, Gauge>::default();
        let slo_target = Family::<SloOnlyLabels, Gauge>::default();
        let target_info = Family::<InfoLabels, Counter>::default();

        registry.register("argis_monitor_polls_total", "Total polls attempted.", polls_total.clone());
        registry.register("argis_monitor_poll_errors_total", "Total polls that failed.", poll_errors_total.clone());
        registry.register("argis_monitor_poll_duration_seconds", "Poll latency in seconds.", poll_duration.clone());
        registry.register("argis_monitor_last_poll_timestamp_seconds", "Unix timestamp of last successful poll.", last_poll_ts.clone());
        registry.register("argis_monitor_up", "1 if last poll succeeded for this provider, else 0.", up.clone());
        registry.register(
            "argis_monitor_burn_rate",
            "Current SLO burn rate as basis-points x 100 (1.0 -> 10000, 14.4 -> 144000). Divide by 10000 to get the float burn rate.",
            burn_rate.clone(),
        );
        registry.register(
            "argis_monitor_slo_target",
            "Configured SLO target as per-mille integer (0.999 -> 999, 0.95 -> 950). Divide by 1000 to get the float target.",
            slo_target.clone(),
        );
        registry.register("argis_monitor_target_info", "Static info about the target gateway.", target_info.clone());

        for target in targets {
            target_info
                .get_or_create(&InfoLabels {
                    target: target.clone(),
                    version: env!("CARGO_PKG_VERSION").into(),
                })
                .inc();
        }

        Self { polls_total, poll_errors_total, poll_duration, last_poll_ts, up, burn_rate, slo_target, target_info }
    }

    pub fn record_sample(&self, s: &Sample) {
        let outcome_str = match s.outcome {
            Outcome::Ok => "ok",
            Outcome::Error => "error",
        };
        self.polls_total.get_or_create(&PollLabels {
            provider: s.provider.clone(),
            outcome: outcome_str.into(),
        }).inc();

        let provider = ProviderLabels { provider: s.provider.clone() };

        if s.outcome == Outcome::Error {
            self.poll_errors_total.get_or_create(&ErrorLabels {
                provider: s.provider.clone(),
                error_kind: error_kind(s.status_code),
            }).inc();
            self.up.get_or_create(&provider).set(0);
        } else {
            self.up.get_or_create(&provider).set(1);
            self.last_poll_ts.get_or_create(&provider).set(s.timestamp_secs as i64);
            self.poll_duration.get_or_create(&provider).observe(s.latency.as_secs_f64());
        }
    }

    /// Set the current burn rate. Stored as basis-points x 100.
    pub fn record_burn(&self, slo: &str, window: BurnWindow, value: f64) {
        let scaled = (value * 10_000.0).round() as i64;
        self.burn_rate.get_or_create(&SloLabels {
            slo: slo.into(),
            window: window_label(window),
        }).set(scaled);
    }

    /// Pin the SLO target ratio. Stored as per-mille integer.
    pub fn record_slo_target(&self, slo: &str, target: f64) {
        let scaled = (target * 1_000.0).round() as i64;
        self.slo_target.get_or_create(&SloOnlyLabels { slo: slo.into() }).set(scaled);
    }
}

fn window_label(w: BurnWindow) -> String {
    match w {
        BurnWindow::FAST_BURN => "5m_1h".into(),
        BurnWindow::SLOW_BURN => "30m_6h".into(),
        _ => format!("{}s_{}s", w.short.as_secs(), w.long.as_secs()),
    }
}

fn error_kind(status: u16) -> String {
    match status {
        0 => "transport".into(),
        401 | 403 => "auth".into(),
        408 | 504 => "timeout".into(),
        429 => "rate_limit".into(),
        s if s >= 500 => "upstream_5xx".into(),
        s if s >= 400 => "upstream_4xx".into(),
        s => format!("status_{s}"),
    }
}

mod ts_secs {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> { s.serialize_u64(*v) }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> { u64::deserialize(d) }
}
