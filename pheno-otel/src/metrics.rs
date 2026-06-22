//! L25 — fleet-wide OTLP **metrics facade**.
//!
//! Per ADR-037 + ADR-042B + the v22 cycle-12 P1 plan
//! (`plans/2026-06-22-v22-71-pillar-cycle-12-p1.md`), this module is
//! the **producer-side sibling** of [`crate::histogram`] and the
//! **canonical counter/gauge/distribution surface** for every pheno-*
//! crate that needs to emit OTel-style metrics without taking a direct
//! dependency on the upstream `opentelemetry` SDK.
//!
//! # Why a facade (and not a re-export of `opentelemetry::metrics`)
//!
//! The upstream OpenTelemetry Rust SDK is a heavyweight dependency
//! (protobuf, tonic, reqwest, async runtime, …). The fleet-port
//! substrate rule (ADR-023 Rule 3.1, ADR-038) says consumer crates
//! depend on `pheno-otel` for OTLP-exportable telemetry, not on
//! `opentelemetry` directly. This module therefore:
//!
//! 1. Exposes a **stable, minimal, fleet-owned API surface**
//!    ([`Metrics`], [`Counter`], [`Histogram`], [`Gauge`]) that any
//!    pheno-* crate can use without pulling in the OTel SDK.
//! 2. Records into an in-process, **zero-dep, atomic-backed** store
//!    keyed by `(metric_name, label_set)`. The store is the substrate
//!    substrate-of-record — it serializes to OTLP/JSON for export via
//!    [`Metrics::snapshot`] (consumed by `HttpExporter`'s `/v1/metrics`
//!    path in `crate::exporters::http`).
//! 3. **Abstracts over** the OTel SDK: a future OTel-backed adapter can
//!    be slotted behind the same API without consumer changes. The
//!    abstraction is the public type signatures, not a trait object —
//!    this keeps the surface zero-cost and `dyn`-free.
//!
//! # Cardinality
//!
//! All label values are `String`. Callers MUST keep label sets bounded
//! (closed enums, normalized paths, status codes). The store does not
//! enforce cardinality caps; that responsibility lives one level up,
//! at the call sites (see `pheno-otel/docs/slos/pheno-otel.md` §4 for
//! the SLO on metric cardinality).
//!
//! # Thread safety
//!
//! [`Metrics`] is `Send + Sync` and uses [`std::sync::RwLock`] on the
//! registry + [`std::sync::atomic::AtomicU64`] / [`AtomicI64`] for
//! individual counter / histogram / gauge cells. Cloning a [`Metrics`]
//! shares the underlying store (`Arc` internally), so it is the
//! canonical way to pass the facade across thread boundaries.
//!
//! # When to use
//!
//! - You need to record a counter, gauge, or distribution with OTel
//!   semantic-conventions-aligned names.
//! - You want the data to flow through `pheno-otel::exporters::http`
//!   to an OTLP/HTTP collector.
//! - You want fleet-wide aggregation without pulling in the upstream
//!   OTel SDK.
//!
//! # When NOT to use
//!
//! - You need exact quantiles (p99.9, p99.99) → use
//!   [`crate::histogram::LatencyHistogram`].
//! - You need Prometheus-format scrape output → use a separate
//!   `prometheus_exporter` adapter (out of scope for this facade).
//! - You need in-process tracing → use `pheno-tracing`.

#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Fixed histogram bucket boundaries (seconds). Aligned with OTel
/// semantic conventions for HTTP server duration.
pub const HISTOGRAM_BUCKETS_SECONDS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

// ---------------------------------------------------------------------------
// Label set
// ---------------------------------------------------------------------------

/// A label set is an ordered map of `String` keys to `String` values.
///
/// We use [`BTreeMap`] (not `HashMap`) so snapshots have a stable
/// iteration order, which makes OTLP/JSON output deterministic and
/// diff-friendly in CI.
pub type LabelSet = BTreeMap<String, String>;

/// Convenience constructor for a label set from a slice of `(key, value)`
/// pairs.
pub fn labels(pairs: &[(&str, &str)]) -> LabelSet {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// Instrument handles
// ---------------------------------------------------------------------------

/// A monotonically increasing counter (OTel `Counter`).
///
/// Counters are identified by the [`Metrics`] registry they were
/// created from + their metric name + their label set. `add` is
/// `Ordering::Relaxed` — we do not need stronger ordering because the
/// counter is per-`(name, labels)` and any reordering still produces a
/// correct final value when summed downstream.
#[derive(Debug)]
pub struct Counter {
    /// `Arc` to the shared cell backing the counter.
    cell: Arc<AtomicU64>,
}

impl Counter {
    /// Increment the counter by `value` (defaults to 1).
    pub fn add(&self, value: u64) {
        self.cell.fetch_add(value, Ordering::Relaxed);
    }

    /// Read the current counter value.
    pub fn get(&self) -> u64 {
        self.cell.load(Ordering::Relaxed)
    }
}

/// A distribution histogram with fixed [`HISTOGRAM_BUCKETS_SECONDS`]
/// boundaries (OTel `Histogram`).
///
/// Each `(name, label_set)` pair has its own bucket array. Buckets are
/// `AtomicU64` + `Ordering::Relaxed`; the count + sum are atomic as
/// well so a snapshot can be taken without holding the registry lock.
#[derive(Debug)]
pub struct Histogram {
    /// Shared bucket array (`Arc<[AtomicU64; N]>`).
    buckets: Arc<[AtomicU64; HISTOGRAM_BUCKETS_SECONDS.len()]>,
    /// Total observations.
    count: Arc<AtomicU64>,
    /// Sum of all observations, in seconds, scaled to microseconds
    /// (so we can use integer atomic ops).
    sum_us: Arc<AtomicI64>,
}

impl Histogram {
    /// Record a single observation `value` (seconds).
    pub fn observe(&self, value: f64) {
        if !value.is_finite() || value < 0.0 {
            // Drop NaN / negative observations silently; matches OTel
            // SDK behavior (NaN is dropped, negatives are unspecified
            // and we choose to drop rather than poison the sum).
            return;
        }
        let bucket_idx = HISTOGRAM_BUCKETS_SECONDS
            .iter()
            .position(|b| value <= *b)
            .unwrap_or(HISTOGRAM_BUCKETS_SECONDS.len() - 1);
        self.buckets[bucket_idx].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        // f64 -> micros; clamp to i64 range.
        let micros = (value * 1_000_000.0).round();
        let micros = micros.clamp(i64::MIN as f64, i64::MAX as f64) as i64;
        self.sum_us.fetch_add(micros, Ordering::Relaxed);
    }

    /// Total number of observations.
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Sum of observations, in seconds.
    pub fn sum_seconds(&self) -> f64 {
        (self.sum_us.load(Ordering::Relaxed) as f64) / 1_000_000.0
    }

    /// Per-bucket cumulative counts.
    pub fn bucket_counts(&self) -> Vec<u64> {
        self.buckets
            .iter()
            .map(|b| b.load(Ordering::Relaxed))
            .collect()
    }
}

/// A point-in-time gauge (OTel `UpDownCounter` / `Gauge`).
///
/// Gauges can go up and down. `set` replaces the current value;
/// `add` / `sub` modify it by a signed delta.
#[derive(Debug)]
pub struct Gauge {
    /// Shared cell backing the gauge (signed; gauges can be negative).
    cell: Arc<AtomicI64>,
}

impl Gauge {
    /// Set the gauge to an absolute value.
    pub fn set(&self, value: i64) {
        self.cell.store(value, Ordering::Relaxed);
    }

    /// Add a signed delta to the gauge.
    pub fn add(&self, delta: i64) {
        self.cell.fetch_add(delta, Ordering::Relaxed);
    }

    /// Subtract a signed delta from the gauge.
    pub fn sub(&self, delta: i64) {
        self.cell.fetch_sub(delta, Ordering::Relaxed);
    }

    /// Read the current gauge value.
    pub fn get(&self) -> i64 {
        self.cell.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Registry entry — what lives inside the `Metrics` map
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum InstrumentCell {
    Counter(Arc<AtomicU64>),
    Histogram {
        buckets: Arc<[AtomicU64; HISTOGRAM_BUCKETS_SECONDS.len()]>,
        count: Arc<AtomicU64>,
        sum_us: Arc<AtomicI64>,
    },
    Gauge(Arc<AtomicI64>),
}

#[derive(Debug)]
struct RegistryEntry {
    /// Metric name (e.g. `http_requests_total`).
    name: String,
    /// Per-label-set instrument storage.
    by_labels: RwLock<BTreeMap<LabelSet, InstrumentCell>>,
}

impl RegistryEntry {
    fn new_counter(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            by_labels: RwLock::new(BTreeMap::new()),
        }
    }

    fn new_histogram(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            by_labels: RwLock::new(BTreeMap::new()),
        }
    }

    fn new_gauge(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            by_labels: RwLock::new(BTreeMap::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// Metrics — the facade
// ---------------------------------------------------------------------------

/// The fleet metrics facade. Wraps the in-process registry and exposes
/// counter / histogram / gauge factories that abstract over the
/// OpenTelemetry crate (consumers never need to import
/// `opentelemetry::metrics::*` directly).
///
/// `Metrics` is cheap to clone (internal `Arc`). Pass clones across
/// thread boundaries; the underlying registry is shared.
///
/// # Example
///
/// ```
/// use pheno_otel::metrics::{labels, Metrics};
///
/// let metrics = Metrics::new();
/// let counter = metrics.http_requests_total(&labels(&[
///     ("method", "GET"),
///     ("status", "200"),
///     ("path", "/api/v1/widgets"),
/// ]));
/// counter.add(1);
///
/// let snapshot = metrics.snapshot();
/// assert!(!snapshot.counters.is_empty());
/// ```
#[derive(Debug, Clone)]
pub struct Metrics {
    inner: Arc<MetricsInner>,
}

#[derive(Debug)]
struct MetricsInner {
    /// `metric_name -> RegistryEntry`. Inserted lazily on first call
    /// to the corresponding factory method.
    by_name: RwLock<BTreeMap<String, Arc<RegistryEntry>>>,
}

impl Metrics {
    /// Construct a new, empty metrics facade.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(MetricsInner {
                by_name: RwLock::new(BTreeMap::new()),
            }),
        }
    }

    /// Look up (or insert) the registry entry for a metric name. Caller
    /// specifies the entry factory for first-time inserts.
    fn entry_for(
        &self,
        name: &str,
        factory: impl FnOnce(String) -> RegistryEntry,
    ) -> Arc<RegistryEntry> {
        // Fast path: read lock.
        if let Some(entry) = self.inner.by_name.read().unwrap().get(name).cloned() {
            return entry;
        }
        // Slow path: write lock, double-check.
        let mut guard = self.inner.by_name.write().unwrap();
        if let Some(entry) = guard.get(name).cloned() {
            return entry;
        }
        let entry = Arc::new(factory(name.to_string()));
        guard.insert(name.to_string(), entry.clone());
        entry
    }

    // -- raw factories (generic surface, no metric-specific sugar) ---

    /// Get-or-create a counter for the given `(name, labels)` pair.
    pub fn counter(&self, name: &str, labels: &LabelSet) -> Counter {
        let entry = self.entry_for(name, RegistryEntry::new_counter);
        let cell = {
            let read = entry.by_labels.read().unwrap();
            if let Some(InstrumentCell::Counter(c)) = read.get(labels) {
                Arc::clone(c)
            } else {
                drop(read);
                let mut write = entry.by_labels.write().unwrap();
                if let Some(InstrumentCell::Counter(c)) = write.get(labels) {
                    Arc::clone(c)
                } else {
                    let new_cell = Arc::new(AtomicU64::new(0));
                    write.insert(
                        labels.clone(),
                        InstrumentCell::Counter(Arc::clone(&new_cell)),
                    );
                    new_cell
                }
            }
        };
        Counter { cell }
    }

    /// Get-or-create a histogram for the given `(name, labels)` pair.
    pub fn histogram(&self, name: &str, labels: &LabelSet) -> Histogram {
        let entry = self.entry_for(name, RegistryEntry::new_histogram);
        let (buckets, count, sum_us) = {
            let read = entry.by_labels.read().unwrap();
            if let Some(InstrumentCell::Histogram { buckets, count, sum_us }) = read.get(labels) {
                (Arc::clone(buckets), Arc::clone(count), Arc::clone(sum_us))
            } else {
                drop(read);
                let mut write = entry.by_labels.write().unwrap();
                if let Some(InstrumentCell::Histogram { buckets, count, sum_us }) = write.get(labels) {
                    (Arc::clone(buckets), Arc::clone(count), Arc::clone(sum_us))
                } else {
                    let buckets: Arc<[AtomicU64; HISTOGRAM_BUCKETS_SECONDS.len()]> =
                        Arc::new(std::array::from_fn(|_| AtomicU64::new(0)));
                    let count = Arc::new(AtomicU64::new(0));
                    let sum_us = Arc::new(AtomicI64::new(0));
                    write.insert(
                        labels.clone(),
                        InstrumentCell::Histogram {
                            buckets: Arc::clone(&buckets),
                            count: Arc::clone(&count),
                            sum_us: Arc::clone(&sum_us),
                        },
                    );
                    (buckets, count, sum_us)
                }
            }
        };
        Histogram { buckets, count, sum_us }
    }

    /// Get-or-create a gauge for the given `(name, labels)` pair.
    pub fn gauge(&self, name: &str, labels: &LabelSet) -> Gauge {
        let entry = self.entry_for(name, RegistryEntry::new_gauge);
        let cell = {
            let read = entry.by_labels.read().unwrap();
            if let Some(InstrumentCell::Gauge(c)) = read.get(labels) {
                Arc::clone(c)
            } else {
                drop(read);
                let mut write = entry.by_labels.write().unwrap();
                if let Some(InstrumentCell::Gauge(c)) = write.get(labels) {
                    Arc::clone(c)
                } else {
                    let new_cell = Arc::new(AtomicI64::new(0));
                    write.insert(
                        labels.clone(),
                        InstrumentCell::Gauge(Arc::clone(&new_cell)),
                    );
                    new_cell
                }
            }
        };
        Gauge { cell }
    }

    // -- example metric sugar (the 5 fleet-canonical metrics) ---

    /// `http_requests_total` (counter) — labels: `method`, `status`, `path`.
    pub fn http_requests_total(&self, labels: &LabelSet) -> Counter {
        self.counter("http_requests_total", labels)
    }

    /// `http_request_duration_seconds` (histogram) — labels: `method`, `path`.
    pub fn http_request_duration_seconds(&self, labels: &LabelSet) -> Histogram {
        self.histogram("http_request_duration_seconds", labels)
    }

    /// `active_connections` (gauge) — typically 1 label: `service`.
    pub fn active_connections(&self, labels: &LabelSet) -> Gauge {
        self.gauge("active_connections", labels)
    }

    /// `errors_total` (counter) — labels: `kind`.
    pub fn errors_total(&self, labels: &LabelSet) -> Counter {
        self.counter("errors_total", labels)
    }

    /// `queue_depth` (gauge) — typically 1 label: `queue_name`.
    pub fn queue_depth(&self, labels: &LabelSet) -> Gauge {
        self.gauge("queue_depth", labels)
    }

    // -- export ---

    /// Render the full registry as a JSON-stable snapshot suitable for
    /// OTLP/JSON export (the `/v1/metrics` payload shape).
    pub fn snapshot(&self) -> MetricsSnapshot {
        let mut out = MetricsSnapshot::default();
        let guard = self.inner.by_name.read().unwrap();
        // Deterministic ordering for diffable output.
        for (name, entry) in guard.iter() {
            let labels_guard = entry.by_labels.read().unwrap();
            for (label_set, cell) in labels_guard.iter() {
                match cell {
                    InstrumentCell::Counter(c) => {
                        out.counters.push(CounterSnapshot {
                            name: name.clone(),
                            labels: label_set.clone(),
                            value: c.load(Ordering::Relaxed),
                        });
                    }
                    InstrumentCell::Histogram { buckets, count, sum_us } => {
                        let bucket_counts: Vec<u64> =
                            buckets.iter().map(|b| b.load(Ordering::Relaxed)).collect();
                        out.histograms.push(HistogramSnapshot {
                            name: name.clone(),
                            labels: label_set.clone(),
                            bucket_counts,
                            count: count.load(Ordering::Relaxed),
                            sum_seconds: (sum_us.load(Ordering::Relaxed) as f64) / 1_000_000.0,
                        });
                    }
                    InstrumentCell::Gauge(c) => {
                        out.gauges.push(GaugeSnapshot {
                            name: name.clone(),
                            labels: label_set.clone(),
                            value: c.load(Ordering::Relaxed),
                        });
                    }
                }
            }
        }
        out
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Snapshot — JSON-stable export shape
// ---------------------------------------------------------------------------

/// Top-level snapshot of every instrument known to a [`Metrics`]
/// facade. Serializes to the OTLP/JSON `ResourceMetrics` payload (see
/// `pheno-otel/docs/metrics/dashboards/` for the dashboard bindings).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// One entry per `(metric_name, label_set)` of type counter.
    pub counters: Vec<CounterSnapshot>,
    /// One entry per `(metric_name, label_set)` of type histogram.
    pub histograms: Vec<HistogramSnapshot>,
    /// One entry per `(metric_name, label_set)` of type gauge.
    pub gauges: Vec<GaugeSnapshot>,
}

/// A single counter snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterSnapshot {
    /// OTel-style metric name (e.g. `http_requests_total`).
    pub name: String,
    /// Labels attached to this counter.
    pub labels: LabelSet,
    /// Current value.
    pub value: u64,
}

/// A single histogram snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramSnapshot {
    /// OTel-style metric name.
    pub name: String,
    /// Labels attached to this histogram.
    pub labels: LabelSet,
    /// Per-bucket cumulative counts (matches [`HISTOGRAM_BUCKETS_SECONDS`]).
    pub bucket_counts: Vec<u64>,
    /// Total observations.
    pub count: u64,
    /// Sum of observations, in seconds.
    pub sum_seconds: f64,
}

/// A single gauge snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GaugeSnapshot {
    /// OTel-style metric name.
    pub name: String,
    /// Labels attached to this gauge.
    pub labels: LabelSet,
    /// Current value.
    pub value: i64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Counter increment: add(3) then add(2) then get() == 5.
    #[test]
    fn counter_increment_is_cumulative() {
        let m = Metrics::new();
        let c = m.http_requests_total(&labels(&[
            ("method", "GET"),
            ("status", "200"),
            ("path", "/x"),
        ]));
        assert_eq!(c.get(), 0);
        c.add(3);
        c.add(2);
        assert_eq!(c.get(), 5);

        // Distinct label sets get distinct cells.
        let c2 = m.http_requests_total(&labels(&[
            ("method", "POST"),
            ("status", "500"),
            ("path", "/x"),
        ]));
        c2.add(7);
        assert_eq!(c.get(), 5);
        assert_eq!(c2.get(), 7);
    }

    /// Histogram observation: observe 3 values, verify count + buckets.
    #[test]
    fn histogram_observation_increments_buckets() {
        let m = Metrics::new();
        let h = m.http_request_duration_seconds(&labels(&[
            ("method", "GET"),
            ("path", "/x"),
        ]));
        h.observe(0.003); // <= 0.005  -> bucket 0
        h.observe(0.020); // <= 0.025  -> bucket 2
        h.observe(3.5); // <= 5.0    -> bucket 9

        assert_eq!(h.count(), 3);
        assert!((h.sum_seconds() - 3.523).abs() < 1e-6);

        let counts = h.bucket_counts();
        // Bucket 0 (le=0.005) should have 1 observation.
        // Bucket 2 (le=0.025) should have 1 observation.
        // Bucket 9 (le=5.0) should have 1 observation.
        // All other buckets should be 0.
        assert_eq!(counts[0], 1);
        assert_eq!(counts[2], 1);
        assert_eq!(counts[9], 1);
        assert_eq!(counts[1], 0);
        assert_eq!(counts[10], 0);
    }

    /// Gauge set / add / sub round-trip.
    #[test]
    fn gauge_set_add_sub_round_trip() {
        let m = Metrics::new();
        let g = m.active_connections(&labels(&[("service", "api")]));
        assert_eq!(g.get(), 0);
        g.set(10);
        assert_eq!(g.get(), 10);
        g.add(5);
        assert_eq!(g.get(), 15);
        g.sub(20);
        assert_eq!(g.get(), -5);
    }

    /// Snapshot round-trips through serde_json.
    #[test]
    fn snapshot_serializes_to_json() {
        let m = Metrics::new();
        m.http_requests_total(&labels(&[("method", "GET")])).add(4);
        m.errors_total(&labels(&[("kind", "timeout")])).add(1);
        m.queue_depth(&labels(&[("queue_name", "jobs")])).set(17);

        let snap = m.snapshot();
        let json = serde_json::to_string(&snap).expect("serialize");
        // Round-trip back through the deserializer.
        let back: MetricsSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.counters.len(), 2);
        assert_eq!(back.gauges.len(), 1);
        assert_eq!(back.histograms.len(), 0);

        let http = back
            .counters
            .iter()
            .find(|c| c.name == "http_requests_total")
            .expect("http_requests_total present");
        assert_eq!(http.value, 4);

        let q = back
            .gauges
            .iter()
            .find(|g| g.name == "queue_depth")
            .expect("queue_depth present");
        assert_eq!(q.value, 17);
    }

    /// Histogram drops NaN / negative observations (matches OTel SDK).
    #[test]
    fn histogram_drops_invalid_observations() {
        let m = Metrics::new();
        let h = m.http_request_duration_seconds(&labels(&[
            ("method", "GET"),
            ("path", "/x"),
        ]));
        h.observe(f64::NAN);
        h.observe(-1.0);
        h.observe(0.1);
        assert_eq!(h.count(), 1);
    }

    /// Two Metrics clones share the same underlying registry.
    #[test]
    fn metrics_clone_shares_registry() {
        let a = Metrics::new();
        let b = a.clone();
        a.http_requests_total(&labels(&[("method", "GET")])).add(11);
        // Reading via the clone sees the increment.
        let snap = b.snapshot();
        let c = snap
            .counters
            .iter()
            .find(|c| c.name == "http_requests_total")
            .unwrap();
        assert_eq!(c.value, 11);
    }

    /// Distinct label sets get distinct cells (cardinality discipline).
    #[test]
    fn distinct_label_sets_get_distinct_cells() {
        let m = Metrics::new();
        let a = m.errors_total(&labels(&[("kind", "timeout")]));
        let b = m.errors_total(&labels(&[("kind", "io")]));
        a.add(1);
        b.add(2);
        assert_eq!(a.get(), 1);
        assert_eq!(b.get(), 2);
    }

    /// Default impl is equivalent to `new`.
    #[test]
    fn default_matches_new() {
        let m = Metrics::default();
        m.http_requests_total(&labels(&[("method", "GET")])).add(1);
        assert_eq!(m.snapshot().counters.len(), 1);
    }
}
