use std::time::Duration;

pub(crate) fn parse_human(s: &str) -> Result<Duration, String> {
    let s = s.trim();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_units() {
        assert_eq!(parse_human("15s"), Ok(Duration::from_secs(15)));
        assert_eq!(parse_human("2m"), Ok(Duration::from_secs(120)));
        assert_eq!(parse_human("1h"), Ok(Duration::from_secs(3600)));
        assert_eq!(parse_human("1d"), Ok(Duration::from_secs(86_400)));
    }
}
