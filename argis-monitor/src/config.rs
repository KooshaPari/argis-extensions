//! Configuration types for argis-monitor.
//!
//! Loaded from CLI flags, env vars (prefix `ARGIS_MONITOR_`), and/or a YAML
//! file. See `examples/basic.yaml` for a complete example.

use std::fmt;
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

fn default_slo_window_secs() -> u64 { 30 * 24 * 3600 }
fn default_slo_target() -> f64 { 0.999 }

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
#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    /// Targets to poll. At least one required. The monitor spawns one tokio
    /// task per target. Each target's `provider` label is its `name`.
    #[serde(default)]
    pub targets: Vec<crate::target::Target>,
    /// How often to poll each target by default. Per-target overrides apply
    /// if the target sets its own `poll_interval`.
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
    /// Alert rules evaluated each tick. Empty by default (alerts off).
    #[serde(default)]
    pub alert_rules: Vec<crate::alerts::AlertRule>,
    /// Optional bearer token sent on every poll. Use for protected gateways.
    #[serde(default)]
    pub bearer_token: Option<String>,
    /// Suppression windows. When an alert would fire, the matcher is checked
    /// against `(target_name, rule_name, now)`; any matching window swallows
    /// the webhook delivery but still records the state transition.
    #[serde(default)]
    pub alert_windows: Vec<crate::suppression::WindowSpec>,
    /// Directory where the alert state store is persisted. Defaults to
    /// `./data` relative to CWD; set to `None` to disable persistence
    /// (useful in tests).
    #[serde(default = "default_data_dir")]
    pub data_dir: Option<std::path::PathBuf>,
    /// Optional Pushgateway URL. When set, the monitor spawns a background
    /// task that POSTs the registry contents every `push_interval_secs`
    /// seconds. Useful for service-discovery-free topologies or for
    /// forwarding to a downstream TSDB.
    #[serde(default)]
    pub push_url: Option<String>,
    /// Push interval (seconds). Defaults to 15s (matches the default poll
    /// interval). Ignored when `push_url` is None.
    #[serde(default = "default_push_interval")]
    pub push_interval_secs: u64,
    /// Job label used in the Pushgateway URL path. Defaults to the host
    /// name from `hostname` or "argis-monitor" if that fails.
    #[serde(default)]
    pub push_job: Option<String>,
    /// Instance label used in the Pushgateway URL path. Defaults to
    /// "host-{pid}" where {pid} is the current process id.
    #[serde(default)]
    pub push_instance: Option<String>,
}

fn default_push_interval() -> u64 { 15 }

fn default_data_dir() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from("./data"))
}

fn default_poll_interval() -> Duration { Duration::from_secs(15) }
fn default_poll_timeout() -> Duration { Duration::from_secs(5) }
fn default_exporter_addr() -> String { "0.0.0.0:9090".to_string() }

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("targets", &self.targets)
            .field("poll_interval", &self.poll_interval)
            .field("poll_timeout", &self.poll_timeout)
            .field("exporter_addr", &self.exporter_addr)
            .field("slos", &self.slos)
            .field("alert_rules_count", &self.alert_rules.len())
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .field("alert_windows", &self.alert_windows)
            .field("data_dir", &self.data_dir)
            .field("push_url", &self.push_url)
            .field("push_interval_secs", &self.push_interval_secs)
            .field("push_job", &self.push_job)
            .field("push_instance", &self.push_instance)
            .finish()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            targets: vec![crate::target::Target::new("gateway", "http://127.0.0.1:8080")],
            poll_interval: default_poll_interval(),
            poll_timeout: default_poll_timeout(),
            exporter_addr: default_exporter_addr(),
            slos: vec![SLO::default()],
            alert_rules: Vec::new(),
            bearer_token: None,
            data_dir: default_data_dir(),
            push_url: None,
            push_interval_secs: default_push_interval(),
            push_job: None,
            push_instance: None,
            alert_windows: Vec::new(),
        }
    }
}

impl Config {
    /// Convenience: the first target's URL (or empty string if no targets).
    pub fn first_url(&self) -> &str {
        self.targets.first().map(|t| t.url.as_str()).unwrap_or("")
    }
    /// Set the first target's URL.
    pub fn with_target_url(mut self, url: impl Into<String>) -> Self {
        if let Some(t) = self.targets.first_mut() {
            t.url = url.into();
        } else {
            self.targets.push(crate::target::Target::new("gateway", url.into()));
        }
        self
    }
    /// Add a target by name + URL.
    pub fn with_target_named(mut self, name: impl Into<String>, url: impl Into<String>) -> Self {
        self.targets.push(crate::target::Target::new(name, url.into()));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_test_disables_persistent_state() {
        assert!(Config::for_test("http://127.0.0.1:8080").data_dir.is_none());
    }

    #[test]
    fn debug_redacts_credentials() {
        let mut config = Config::default();
        config.bearer_token = Some("bearer-secret".into());
        config.alert_rules.push(crate::alerts::AlertRule {
            webhooks: vec![crate::alerts::WebhookTarget {
                aws_secret_access_key: Some("aws-secret".into()),
                ..Default::default()
            }],
            ..Default::default()
        });
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("bearer-secret"));
        assert!(!rendered.contains("aws-secret"));
        assert!(rendered.contains("<redacted>"));
    }
}

impl Config {
    /// Set the poll interval (used when no per-target override is set).
    pub fn with_poll_interval_secs(mut self, secs: u64) -> Self {
        self.poll_interval = Duration::from_secs(secs); self
    }
    /// Add a single SLO.
    pub fn with_slo(mut self, slo: SLO) -> Self {
        self.slos.push(slo); self
    }
    /// Add a single alert rule.
    pub fn with_alert_rule(mut self, rule: crate::alerts::AlertRule) -> Self {
        self.alert_rules.push(rule); self
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
        enum Repr { Secs(u64), Text(String) }
        match Repr::deserialize(d)? {
            Repr::Secs(n) => Ok(Duration::from_secs(n)),
            Repr::Text(t) => crate::duration::parse_human(&t).map_err(serde::de::Error::custom),
        }
    }
}


impl Config {
    /// Convenience for tests: a single-target Config pointing at `target`.
    #[doc(hidden)]
    pub fn for_test(target: impl Into<String>) -> Self {
        let mut config = Self::default().with_target_url(target);
        config.data_dir = None;
        config
    }
}
