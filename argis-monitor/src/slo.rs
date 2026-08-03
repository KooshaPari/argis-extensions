//! SLO burn-rate computation.
//!
//! Burn rate at time t over window W is:
//!     burn = (1 - success_ratio_W) / (1 - target)
//! A burn rate of 1.0 means the SLO is exactly on track. >1.0 means the
//! error budget is being consumed faster than sustainable.
//!
//! Multi-window analysis follows the Google SRE workbook
//! (https://sre.google/workbook/alerting-on-slos/) — short and long windows
//! are combined to reduce false positives and catch slow burns.

use std::time::Duration;

/// A pair of windows used for multi-window burn-rate alerts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BurnWindow {
    pub short: Duration,
    pub long: Duration,
}

impl BurnWindow {
    /// The canonical "fast burn" pair: 5m / 1h.
    pub const FAST_BURN: BurnWindow = BurnWindow {
        short: Duration::from_secs(5 * 60),
        long: Duration::from_secs(3600),
    };
    /// The canonical "slow burn" pair: 30m / 6h.
    pub const SLOW_BURN: BurnWindow = BurnWindow {
        short: Duration::from_secs(30 * 60),
        long: Duration::from_secs(6 * 3600),
    };
}

/// Burn-rate math. Operates on (successes, failures, target) over a window.
///
/// Returns the burn rate as a multiplier of the error budget consumption.
/// Returns `f64::INFINITY` if the target is 1.0 (impossible SLO).
/// Returns 0.0 if no requests observed in the window.
pub fn burn_rate(successes: u64, failures: u64, target: f64) -> f64 {
    let total = successes + failures;
    if total == 0 { return 0.0; }
    let success_ratio = successes as f64 / total as f64;
    let error_ratio = 1.0 - success_ratio;
    let allowed_error = 1.0 - target;
    if allowed_error <= 0.0 {
        // SLO is "100% success". Any failure is unbounded burn.
        return if error_ratio > 0.0 { f64::INFINITY } else { 0.0 };
    }
    error_ratio / allowed_error
}

/// Multi-window burn. Returns `(short, long)` burn rates for use by
/// the SRE alerting recipe (alert when short_window_burn > 14.4 AND
/// long_window_burn > 14.4). Both values are returned for richer dashboards.
pub fn multi_window_burn(
    short_success: u64, short_failure: u64,
    long_success: u64, long_failure: u64,
    target: f64,
) -> (f64, f64) {
    let short = burn_rate(short_success, short_failure, target);
    let long = burn_rate(long_success, long_failure, target);
    (short, long)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn zero_traffic_returns_zero_burn() {
        assert_eq!(burn_rate(0, 0, 0.999), 0.0);
    }

    #[test]
    fn all_successes_returns_zero_burn() {
        assert_eq!(burn_rate(1000, 0, 0.999), 0.0);
    }

    #[test]
    fn at_target_returns_one_x_burn() {
        // 0.1% error rate against a 99.9% target = exactly 1x burn.
        assert_relative_eq!(burn_rate(999, 1, 0.999), 1.0, epsilon = 1e-9);
    }

    #[test]
    fn ten_x_overshoot() {
        // 1% error rate against 99.9% target = 10x burn.
        assert_relative_eq!(burn_rate(990, 10, 0.999), 10.0, epsilon = 1e-9);
    }

    #[test]
    fn target_one_zero_returns_inf_for_any_failure() {
        assert!(burn_rate(100, 1, 1.0).is_infinite());
        assert_eq!(burn_rate(100, 0, 1.0), 0.0);
    }

    #[test]
    fn multi_window_returns_short_and_long() {
        let (s, l) = multi_window_burn(999, 100, 9990, 10, 0.999);
        // short: 100/1099 error ratio vs 0.001 budget = 90.99x
        assert_relative_eq!(s, (100.0_f64 / 1099.0) / 0.001, epsilon = 1e-3);
        // long: 10/10000 error ratio vs 0.001 budget = 1x
        assert_relative_eq!(l, 1.0, epsilon = 1e-9);
    }
}
