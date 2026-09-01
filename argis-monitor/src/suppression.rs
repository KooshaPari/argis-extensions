//! Alert suppression windows.
//!
//! Two flavors of windows are supported:
//!
//!   1. **Recurring** (time-of-day + days-of-week): e.g. "every weekday
//!      22:00-06:00, suppress alerts for the gateway target".
//!   2. **One-shot** (absolute timestamps): e.g. "2026-07-15T02:00:00Z to
//!      2026-07-15T04:00:00Z, suppress everything".
//!
//! Suppression is checked BEFORE webhook delivery. When a Fire decision is
//! suppressed, the alert state machine still transitions (so we still see
//! "the alert would have fired" in metrics), but the webhook is not called.
//!
//! The check is purely deterministic — no I/O, no clock drift, just
//! arithmetic on the current unix timestamp + window bounds.

use std::time::Duration;

use chrono::{Datelike, Duration as ChronoDuration, NaiveDateTime, Timelike, Weekday};
use serde::{Deserialize, Serialize};

/// Days of the week for recurring windows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Day {
    Mon, Tue, Wed, Thu, Fri, Sat, Sun,
}

impl Day {
    fn from_chrono(w: Weekday) -> Self {
        match w {
            Weekday::Mon => Day::Mon,
            Weekday::Tue => Day::Tue,
            Weekday::Wed => Day::Wed,
            Weekday::Thu => Day::Thu,
            Weekday::Fri => Day::Fri,
            Weekday::Sat => Day::Sat,
            Weekday::Sun => Day::Sun,
        }
    }
}

/// One suppression window. Either a recurring (time-of-day) or one-shot
/// (absolute timestamp) range. Exactly one of {`start_time`/`end_time`,
/// `start_at`/`end_at`} must be set; serde picks based on which keys are
/// present in the YAML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowSpec {
    pub name: String,
    /// Recurring window: "HH:MM" daily start. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    /// Recurring window: "HH:MM" daily end. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    /// Days the recurring window is active. Empty = every day.
    #[serde(default)]
    pub days: Vec<Day>,
    /// One-shot window: RFC3339 start. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_at: Option<String>,
    /// One-shot window: RFC3339 end. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_at: Option<String>,
    /// Only suppress alerts whose target matches one of these names. Empty
    /// = all targets.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Only suppress alerts whose rule name matches one of these. Empty
    /// = all rules.
    #[serde(default)]
    pub rules: Vec<String>,
    /// Free-text reason. Logged on every suppression.
    #[serde(default)]
    pub reason: Option<String>,
}

/// True if the alert at `(target_name, rule_name)` is suppressed at `now_unix`.
/// Returns the matching window's name (for logging) or `None`.
pub fn is_suppressed(
    windows: &[WindowSpec],
    target_name: &str,
    rule_name: &str,
    now_unix: u64,
) -> Option<String> {
    for w in windows {
        if !window_active_now(w, now_unix) { continue; }
        if !w.targets.is_empty() && !w.targets.iter().any(|t| t == target_name) { continue; }
        if !w.rules.is_empty() && !w.rules.iter().any(|r| r == rule_name) { continue; }
        return Some(w.name.clone());
    }
    None
}

fn window_active_now(w: &WindowSpec, now_unix: u64) -> bool {
    // One-shot window takes precedence (exact timestamps).
    if w.start_at.is_some() || w.end_at.is_some() {
        return one_shot_active(w, now_unix);
    }
    if w.start_time.is_some() || w.end_time.is_some() {
        return recurring_active(w, now_unix);
    }
    // No time bounds -> never active.
    false
}

fn one_shot_active(w: &WindowSpec, now_unix: u64) -> bool {
    let parse = |s: &Option<String>| -> Option<i64> {
        s.as_deref().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.timestamp())
        })
    };
    let now = now_unix as i64;
    match (parse(&w.start_at), parse(&w.end_at)) {
        (Some(start), Some(end)) => now >= start && now <= end,
        (Some(start), None) => now >= start,
        (None, Some(end)) => now <= end,
        (None, None) => false,
    }
}

fn recurring_active(w: &WindowSpec, now_unix: u64) -> bool {
    let start_hhmm = match w.start_time.as_deref() {
        Some(s) => match parse_hhmm(s) { Some(p) => p, None => return false },
        None => (0, 0),
    };
    let end_hhmm = match w.end_time.as_deref() {
        Some(s) => match parse_hhmm(s) { Some(p) => p, None => return false },
        None => (23, 59),
    };
    // Convert now_unix to local-time NaiveDateTime. We use UTC for simplicity
    // since this is a substrate; ops can configure their TZ at deployment.
    let dt = match unix_to_naive_utc(now_unix) {
        Some(d) => d,
        None => return false,
    };
    let (hh, mm) = (dt.hour(), dt.minute());
    let now_secs = hh * 3600 + mm * 60;
    let start_secs = start_hhmm.0 * 3600 + start_hhmm.1 * 60;
    let end_secs = end_hhmm.0 * 3600 + end_hhmm.1 * 60;
    // The post-midnight portion of a wrapping window belongs to the day on
    // which that window started (Tuesday 02:00 is Monday's 22:00-06:00).
    let window_day = if start_secs > end_secs && now_secs <= end_secs {
        dt - ChronoDuration::days(1)
    } else {
        dt
    };
    if !w.days.is_empty() {
        let day = Day::from_chrono(window_day.weekday());
        if !w.days.contains(&day) { return false; }
    }
    if start_secs <= end_secs {
        now_secs >= start_secs && now_secs <= end_secs
    } else {
        // Window wraps midnight (e.g. 22:00 - 06:00).
        now_secs >= start_secs || now_secs <= end_secs
    }
}

fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let mut parts = s.split(':');
    let hh: u32 = parts.next()?.parse().ok()?;
    let mm: u32 = parts.next()?.parse().ok()?;
    if hh > 23 || mm > 59 { return None; }
    Some((hh, mm))
}

fn unix_to_naive_utc(unix: u64) -> Option<NaiveDateTime> {
    let dt_utc = chrono::DateTime::<chrono::Utc>::from_timestamp(unix as i64, 0)?;
    Some(dt_utc.naive_utc())
}

/// Helper for tests: build a fixed `now_unix` from a (year, month, day, hour, minute, second) UTC tuple.
pub fn unix_from_utc(year: i32, month: u32, day: u32, hh: u32, mm: u32, ss: u32) -> u64 {
    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap()
            .and_hms_opt(hh, mm, ss).unwrap(),
        chrono::Utc,
    ).timestamp() as u64
}

/// Quick example: returns how many seconds the test caller should advance to
/// skip past a recurring window. Convenience only — the matcher is the API.
pub fn _active_for_at_least(window: &WindowSpec, now_unix: u64) -> Option<Duration> {
    if !window_active_now(window, now_unix) { return None; }
    Some(Duration::from_secs(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_window_is_inactive() {
        let w = WindowSpec { name: "x".into(), start_time: None, end_time: None, days: vec![], start_at: None, end_at: None, targets: vec![], rules: vec![], reason: None };
        assert!(!window_active_now(&w, 1_700_000_000));
        assert!(is_suppressed(&[w], "any", "any", 1_700_000_000).is_none());
    }

    #[test]
    fn recurring_daily_window_matches_within_hours() {
        let w = WindowSpec {
            name: "daytime".into(),
            start_time: Some("09:00".into()),
            end_time: Some("17:00".into()),
            days: vec![],
            start_at: None, end_at: None,
            targets: vec![], rules: vec![],
            reason: None,
        };
        // 2026-07-06 is a Monday. Pick 12:00 UTC -> inside 09-17.
        let t = unix_from_utc(2026, 7, 6, 12, 0, 0);
        assert_eq!(is_suppressed(&[w.clone()], "any", "any", t), Some("daytime".into()));
        // 18:00 -> outside.
        let t = unix_from_utc(2026, 7, 6, 18, 0, 0);
        assert!(is_suppressed(&[w], "any", "any", t).is_none());
    }

    #[test]
    fn recurring_window_wraps_midnight() {
        let w = WindowSpec {
            name: "night".into(),
            start_time: Some("22:00".into()),
            end_time: Some("06:00".into()),
            days: vec![],
            start_at: None, end_at: None,
            targets: vec![], rules: vec![],
            reason: None,
        };
        // 23:00 -> inside (wraps).
        assert!(is_suppressed(&[w.clone()], "any", "any", unix_from_utc(2026, 7, 6, 23, 0, 0)).is_some());
        // 02:00 -> inside (other side).
        assert!(is_suppressed(&[w.clone()], "any", "any", unix_from_utc(2026, 7, 7, 2, 0, 0)).is_some());
        // 12:00 -> outside.
        assert!(is_suppressed(&[w], "any", "any", unix_from_utc(2026, 7, 6, 12, 0, 0)).is_none());
    }

    #[test]
    fn day_filter_restricts_window() {
        let w = WindowSpec {
            name: "weekday-quiet".into(),
            start_time: Some("22:00".into()),
            end_time: Some("06:00".into()),
            days: vec![Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri],
            start_at: None, end_at: None,
            targets: vec![], rules: vec![],
            reason: None,
        };
        // 2026-07-06 Monday 23:00 -> inside (weekday + time).
        assert!(is_suppressed(&[w.clone()], "any", "any", unix_from_utc(2026, 7, 6, 23, 0, 0)).is_some());
        // 2026-07-11 Saturday 23:00 -> outside (weekend).
        assert!(is_suppressed(&[w], "any", "any", unix_from_utc(2026, 7, 11, 23, 0, 0)).is_none());
    }

    #[test]
    fn wrapping_window_uses_its_start_day_for_the_post_midnight_segment() {
        let w = WindowSpec {
            name: "monday-night".into(),
            start_time: Some("22:00".into()), end_time: Some("06:00".into()),
            days: vec![Day::Mon], start_at: None, end_at: None,
            targets: vec![], rules: vec![], reason: None,
        };
        assert!(is_suppressed(&[w.clone()], "any", "any", unix_from_utc(2026, 7, 7, 2, 0, 0)).is_some());
        assert!(is_suppressed(&[w], "any", "any", unix_from_utc(2026, 7, 8, 2, 0, 0)).is_none());
    }

    #[test]
    fn one_shot_window_matches_exact_range() {
        let w = WindowSpec {
            name: "maintenance".into(),
            start_at: Some("2026-07-15T02:00:00Z".into()),
            end_at: Some("2026-07-15T04:00:00Z".into()),
            start_time: None, end_time: None, days: vec![],
            targets: vec![], rules: vec![],
            reason: Some("DB upgrade".into()),
        };
        assert!(is_suppressed(&[w.clone()], "any", "any", unix_from_utc(2026, 7, 15, 3, 0, 0)).is_some());
        assert!(is_suppressed(&[w.clone()], "any", "any", unix_from_utc(2026, 7, 15, 1, 0, 0)).is_none());
        assert!(is_suppressed(&[w.clone()], "any", "any", unix_from_utc(2026, 7, 15, 5, 0, 0)).is_none());
    }

    #[test]
    fn one_shot_window_accepts_either_open_bound() {
        let start_only = WindowSpec {
            name: "from-start".into(), start_at: Some("2026-07-15T02:00:00Z".into()), end_at: None,
            start_time: None, end_time: None, days: vec![], targets: vec![], rules: vec![], reason: None,
        };
        assert!(is_suppressed(&[start_only], "any", "any", unix_from_utc(2026, 7, 16, 0, 0, 0)).is_some());
        let end_only = WindowSpec {
            name: "until-end".into(), start_at: None, end_at: Some("2026-07-15T04:00:00Z".into()),
            start_time: None, end_time: None, days: vec![], targets: vec![], rules: vec![], reason: None,
        };
        assert!(is_suppressed(&[end_only], "any", "any", unix_from_utc(2026, 7, 15, 3, 0, 0)).is_some());
    }

    #[test]
    fn target_and_rule_filters_match_any() {
        let w = WindowSpec {
            name: "gateway-only".into(),
            start_time: Some("00:00".into()),
            end_time: Some("23:59".into()),
            days: vec![], start_at: None, end_at: None,
            targets: vec!["gateway".into(), "openai".into()],
            rules: vec!["fast_burn".into()],
            reason: None,
        };
        let t = unix_from_utc(2026, 7, 6, 12, 0, 0);
        // Match: target=openai, rule=fast_burn
        assert!(is_suppressed(&[w.clone()], "openai", "fast_burn", t).is_some());
        // No match: target=anthropic
        assert!(is_suppressed(&[w.clone()], "anthropic", "fast_burn", t).is_none());
        // No match: rule=slow_burn
        assert!(is_suppressed(&[w], "openai", "slow_burn", t).is_none());
    }

    #[test]
    fn multiple_windows_first_match_wins() {
        let w1 = WindowSpec {
            name: "broad".into(),
            start_time: Some("00:00".into()),
            end_time: Some("23:59".into()),
            days: vec![], start_at: None, end_at: None,
            targets: vec![], rules: vec![], reason: None,
        };
        let w2 = WindowSpec {
            name: "specific".into(),
            start_time: Some("00:00".into()),
            end_time: Some("23:59".into()),
            days: vec![], start_at: None, end_at: None,
            targets: vec![], rules: vec![], reason: None,
        };
        // Either matches; first in the list wins.
        let t = unix_from_utc(2026, 7, 6, 12, 0, 0);
        assert_eq!(is_suppressed(&[w1.clone(), w2.clone()], "x", "y", t), Some("broad".into()));
        assert_eq!(is_suppressed(&[w2, w1], "x", "y", t), Some("specific".into()));
    }

    #[test]
    fn invalid_time_string_is_inactive_not_error() {
        let w = WindowSpec {
            name: "bad".into(),
            start_time: Some("not-a-time".into()),
            end_time: Some("06:00".into()),
            days: vec![], start_at: None, end_at: None,
            targets: vec![], rules: vec![], reason: None,
        };
        assert!(!window_active_now(&w, 1_700_000_000));
    }
}
