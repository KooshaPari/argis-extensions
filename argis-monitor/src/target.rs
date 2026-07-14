//! Per-target polling configuration.
//!
//! A `Target` describes one upstream the monitor polls. The substrate supports
//! many targets in parallel (e.g. the Bifrost gateway + each provider account,
//! or multiple Bifrost instances behind a load balancer).

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// One target the monitor polls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Target {
    /// Stable identifier used as the `provider` metric label.
    pub name: String,
    /// URL to GET on each poll. Convention: gateway health endpoint.
    pub url: String,
    /// Per-target poll interval. Optional; falls back to the global
    /// `poll_interval` from the top-level `Config` if omitted.
    #[serde(default, with = "opt_seconds_as_duration")]
    pub poll_interval: Option<Duration>,
    /// Optional override for the per-call HTTP timeout.
    #[serde(default, with = "opt_seconds_as_duration")]
    pub poll_timeout: Option<Duration>,
}

impl Target {
    /// Construct a target with just `name` and `url`. Other fields default.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            poll_interval: None,
            poll_timeout: None,
        }
    }
}

mod opt_seconds_as_duration {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_u64(d.as_secs()),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Secs(u64),
            Text(String),
            None_,
        }
        let opt: Option<Repr> = Option::deserialize(d)?;
        Ok(match opt {
            None => None,
            Some(Repr::Secs(n)) => Some(Duration::from_secs(n)),
            Some(Repr::Text(t)) => Some(parse_human(&t).map_err(serde::de::Error::custom)?),
            Some(Repr::None_) => None,
        })
    }
    fn parse_human(s: &str) -> Result<Duration, String> {
        let s = s.trim();
        let (num, unit) = s.split_at(s.len().saturating_sub(1));
        let n: u64 = num.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_new_uses_defaults() {
        let t = Target::new("openai", "http://api.openai.com/health");
        assert_eq!(t.name, "openai");
        assert_eq!(t.url, "http://api.openai.com/health");
        assert!(t.poll_interval.is_none());
        assert!(t.poll_timeout.is_none());
    }

    #[test]
    fn target_yaml_round_trip() {
        let yaml = "name: openai\nurl: http://api.openai.com/health\npoll_interval: 30s\n";
        let t: Target = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(t.poll_interval, Some(Duration::from_secs(30)));
    }
}
