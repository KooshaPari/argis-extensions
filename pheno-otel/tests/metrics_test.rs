//! Integration tests for the `pheno-otel::metrics` facade.
//!
//! V22-T1 (L25 metrics depth) — verifies the public counter + histogram
//! surfaces via the documented example metric factory methods.
//!
//! These tests live in `tests/` (not the inline `#[cfg(test)] mod tests`
//! inside `src/metrics.rs`) so they exercise the crate only through
//! its public API (`pheno_otel::metrics::{Metrics, labels}`) — the same
//! surface fleet consumers will use. The inline `mod tests` covers
//! internal invariants (Arc sharing, label-set isolation, NaN
//! handling) that the public-API test suite can't reach.

use pheno_otel::metrics::{labels, Metrics};

/// Counter increment: `add(3)` then `add(2)` then `get()` returns `5`.
///
/// Exercises the `Metrics::http_requests_total` example factory method
/// (the fleet-canonical HTTP request counter) and verifies that
/// repeated `add` calls accumulate monotonically into the underlying
/// `Arc<AtomicU64>` cell.
#[test]
fn counter_increment() {
    let m = Metrics::new();
    let counter = m.http_requests_total(&labels(&[
        ("method", "GET"),
        ("status", "200"),
        ("path", "/api/v1/widgets"),
    ]));
    assert_eq!(counter.get(), 0, "fresh counter starts at 0");
    counter.add(3);
    assert_eq!(counter.get(), 3);
    counter.add(2);
    assert_eq!(counter.get(), 5);
    // Distinct label sets get distinct cells (cardinality discipline).
    let counter_500 = m.http_requests_total(&labels(&[
        ("method", "GET"),
        ("status", "500"),
        ("path", "/api/v1/widgets"),
    ]));
    counter_500.add(7);
    assert_eq!(counter.get(), 5, "GET 200 cell unchanged");
    assert_eq!(counter_500.get(), 7, "GET 500 cell is independent");
}

/// Histogram observation: `observe(0.003)` increments the bucket at
/// `le=0.005`; `observe(0.020)` increments `le=0.025`; `observe(3.5)`
/// increments `le=5.0`. The total `count()` is 3 and `sum_seconds()`
/// is the sum of the three observations.
///
/// Exercises the `Metrics::http_request_duration_seconds` example
/// factory method (the fleet-canonical HTTP duration histogram) and
/// verifies the bucket-indexing logic against
/// [`pheno_otel::metrics::HISTOGRAM_BUCKETS_SECONDS`].
#[test]
fn histogram_observation() {
    let m = Metrics::new();
    let histogram = m.http_request_duration_seconds(&labels(&[
        ("method", "GET"),
        ("path", "/api/v1/widgets"),
    ]));
    histogram.observe(0.003); // <= 0.005  -> bucket 0
    histogram.observe(0.020); // <= 0.025  -> bucket 2
    histogram.observe(3.5); // <= 5.0    -> bucket 9
    assert_eq!(histogram.count(), 3);
    // Sum: 0.003 + 0.020 + 3.5 = 3.523 seconds.
    let sum = histogram.sum_seconds();
    assert!(
        (sum - 3.523).abs() < 1e-9,
        "sum_seconds() = {sum}, expected ≈ 3.523"
    );
    let counts = histogram.bucket_counts();
    // Bucket layout matches HISTOGRAM_BUCKETS_SECONDS:
    //   [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    // Observations: 0.003 -> 0; 0.020 -> 2; 3.5 -> 9.
    assert_eq!(counts[0], 1, "bucket le=0.005");
    assert_eq!(counts[2], 1, "bucket le=0.025");
    assert_eq!(counts[9], 1, "bucket le=5.0");
    assert_eq!(counts[1], 0, "bucket le=0.01 untouched");
    assert_eq!(counts[10], 0, "bucket le=10.0 untouched");
}
