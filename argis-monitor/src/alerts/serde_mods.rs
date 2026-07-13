//! Serde helpers shared by `AlertRule` and `MetaAlertRule` for serialising
//! `Duration` fields. Kept in its own module so each rule module can refer
//! to it via `super::serde_mods::*`.

use serde::{Deserialize, Deserializer, Serializer};
use std::time::Duration;

pub mod opt_seconds_as_duration {
    use super::*;

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(dur) => s.serialize_u64(dur.as_secs()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr { Secs(u64), Text(String), Null }
        match Repr::deserialize(d)? {
            Repr::Null => Ok(None),
            Repr::Secs(n) => Ok(Some(Duration::from_secs(n))),
            Repr::Text(t) => parse_human(&t).map(Some).map_err(serde::de::Error::custom),
        }
    }
}

pub mod seconds_as_duration {
    use super::*;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr { Secs(u64), Text(String) }
        match Repr::deserialize(d)? {
            Repr::Secs(n) => Ok(Duration::from_secs(n)),
            Repr::Text(t) => parse_human(&t).map_err(serde::de::Error::custom),
        }
    }
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
