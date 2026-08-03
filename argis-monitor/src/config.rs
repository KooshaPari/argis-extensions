//! Configuration types for argis-monitor.
//!
//! Loaded from CLI flags, env vars (prefix `ARGIS_MONITOR_`), and/or a YAML
//! file. See `examples/basic.yaml` for a complete example.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A single Service Level Objective the monitor tracks.
///
/// Burn rate is computed across `window_secs` of samples against `target`.
/// The default 30-day window with 99.9% target is the industry standard
/// "three nines" reliability SLO.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SLO {
    /// Stable identifier used as the Prometheus label value.
    pub name: String,
    /// Rolling window length over which the success ratio is computed.
    #[serde(default = "default_slo_window_secs")]
    pub window_secs: u64,
    /// Success ratio target in [0.0, 1.0]. 0.999 = "three nines".
    #[serde(default = "default_slo_target")]
    pub target: f64,
}

fn default_slo_window_secs() -> u64 {
    30 * 24 * 3600
}
fn default_slo_target() -> f64 {
    0.999
}

impl Default for SLO {
    fn default() -> Self {
        Self {
            name: "chat_completions_p99".to_string(),
            window_secs: default_slo_window_secs(),
            target: default_slo_target(),
        }
    }
}

/// Top-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Base URL of the Bifrost gateway to poll (e.g. `http://127.0.0.1:8080`).
    pub target: String,
    /// How often to poll the gateway. Defaults to 15s.
    #[serde(default = "default_poll_interval", with = "seconds_as_duration")]
    pub poll_interval: Duration,
    /// How long a single HTTP poll may take before failing. Defaults to 5s.
    #[serde(default = "default_poll_timeout", with = "seconds_as_duration")]
    pub poll_timeout: Duration,
    /// Address the Prometheus exporter listens on. Defaults to `0.0.0.0:9090`.
    #[serde(default = "default_exporter_addr")]
    pub exporter_addr: String,
    /// SLOs to track. Defaults to a single "three nines" chat-completions SLO.
    #[serde(default)]
    pub slos: Vec<SLO>,
    /// Optional bearer token sent on every poll. Use for protected gateways.
    #[serde(default)]
    pub bearer_token: Option<String>,
}

fn default_poll_interval() -> Duration {
    Duration::from_secs(15)
}
fn default_poll_timeout() -> Duration {
    Duration::from_secs(5)
}
fn default_exporter_addr() -> String {
    "0.0.0.0:9090".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target: "http://127.0.0.1:8080".to_string(),
            poll_interval: default_poll_interval(),
            poll_timeout: default_poll_timeout(),
            exporter_addr: default_exporter_addr(),
            slos: vec![SLO::default()],
            bearer_token: None,
        }
    }
}

impl Config {
    /// Set the target gateway URL.
    pub fn with_target(mut self, target: String) -> Self {
        self.target = target;
        self
    }
    /// Set the poll interval.
    pub fn with_poll_interval_secs(mut self, secs: u64) -> Self {
        self.poll_interval = Duration::from_secs(secs);
        self
    }
    /// Add a single SLO.
    pub fn with_slo(mut self, slo: SLO) -> Self {
        self.slos.push(slo);
        self
    }
}

/// Serde helper: serialise `Duration` as integer seconds (default) but allow
/// strings like "15s", "30s" to be parsed in YAML. Kept minimal — we don't
/// pull in `humantime-serde` to avoid an extra dep.
mod seconds_as_duration {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Secs(u64),
            Text(String),
        }
        match Repr::deserialize(d)? {
            Repr::Secs(n) => Ok(Duration::from_secs(n)),
            Repr::Text(t) => parse_human(&t).map_err(serde::de::Error::custom),
        }
    }

    fn parse_human(s: &str) -> Result<Duration, String> {
        let s = s.trim();
        let (num, unit) = s.split_at(s.len().saturating_sub(1));
        let n: u64 = num
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?;
        let mul = match unit {
            "s" => 1,
            "m" => 60,
            "h" => 3600,
            "d" => 86_400,
            _ => return Err(format!("unknown duration unit: {unit}")),
        };
        Ok(Duration::from_secs(n * mul))
    }
}
