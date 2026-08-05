//! Shared parsing for compact duration strings used in YAML configuration.

use std::time::Duration;

/// Parse an integer duration with an s, m, h, or d suffix.
pub(crate) fn parse_human(input: &str) -> Result<Duration, String> {
    let s = input.trim();
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: u64 = num
        .parse()
        .map_err(|e: std::num::ParseIntError| e.to_string())?;
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(format!("unknown duration unit: {unit}")),
    };
    Ok(Duration::from_secs(n * multiplier))
}
