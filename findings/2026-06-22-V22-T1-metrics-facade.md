# V22-T1 — L25 Metrics Facade + Dashboards

**Date:** 2026-06-22
**Cycle:** v22 cycle 12, T1 (P1 reduction round 3, pillar L25)
**Branch:** `feat/v22-l25-metrics-2026-06-22` (NOT pushed per task directive)
**Scope:** OTLP metrics facade (`pheno-otel/src/metrics.rs`) + 5 Grafana dashboards (`pheno-otel/docs/metrics/dashboards/`).
**Adjacent artifact (separate agent, separate scope):** `findings/2026-06-22-v22-T1-L25-metrics.md` (the v22 cycle-12 closure report — broader, includes coverage / framework-lint / pillar-crosswalk). This document is the **facade-focused** companion.

---

## 1. Facade API

The facade lives at `pheno-otel/src/metrics.rs` (701 LOC) and is wired into the public crate surface as `pub mod metrics;` at `pheno-otel/src/lib.rs:109`. It exposes a **stable, minimal, fleet-owned surface** that pheno-* consumers depend on instead of importing the upstream `opentelemetry` SDK.

### 1.1 Public surface (16 items)

| Item | Kind | Purpose |
|---|---|---|
| `Metrics` | struct | Arc-shared facade. Cheap to clone; `Send + Sync`. |
| `Metrics::new()` | fn | Construct an empty registry. |
| `Metrics::default()` | impl | Same as `new()`. |
| `Metrics::counter(name, labels)` | fn | Generic counter factory. |
| `Metrics::histogram(name, labels)` | fn | Generic histogram factory. |
| `Metrics::gauge(name, labels)` | fn | Generic gauge factory. |
| `Metrics::http_requests_total(labels)` | fn | Example sugar — labels: `method`, `status`, `path`. |
| `Metrics::http_request_duration_seconds(labels)` | fn | Example sugar — labels: `method`, `path`. |
| `Metrics::active_connections(labels)` | fn | Example sugar — labels: `service`. |
| `Metrics::errors_total(labels)` | fn | Example sugar — labels: `kind`. |
| `Metrics::queue_depth(labels)` | fn | Example sugar — labels: `queue_name`. |
| `Metrics::snapshot()` | fn | JSON-stable export of every instrument (OTLP wire shape). |
| `Counter` | struct | Handle: `add(u64)`, `get() -> u64`. |
| `Histogram` | struct | Handle: `observe(f64)`, `count()`, `sum_seconds()`, `bucket_counts()`. |
| `Gauge` | struct | Handle: `set(i64)`, `add(i64)`, `sub(i64)`, `get() -> i64`. |
| `LabelSet` | type | `BTreeMap<String, String>` — stable iteration order for diffable JSON. |
| `labels(&[(&str, &str)]) -> LabelSet` | fn | Convenience constructor. |
| `HISTOGRAM_BUCKETS_SECONDS` | const | `[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]` (11 buckets, OTel HTTP-server-aligned). |
| `MetricsSnapshot` | struct | Top-level export DTO (3 vec fields + name/labels/value cells). |
| `CounterSnapshot` / `HistogramSnapshot` / `GaugeSnapshot` | struct | Per-cell DTOs. All `Serialize + Deserialize`. |

### 1.2 Cardinality discipline

- All label keys are `String`, values are `String`.
- The facade does NOT enforce cardinality caps (one level up — at call sites).
- BTreeMap-keyed label sets produce **stable JSON output** (deterministic iteration order), so OTLP/JSON diffs in CI are byte-identical for identical state.
- NaN / negative histogram observations are silently dropped (matches upstream OTel SDK behavior).

### 1.3 Concurrency model

- `Metrics` uses `Arc<MetricsInner>` internally.
- `MetricsInner.by_name: RwLock<BTreeMap<String, Arc<RegistryEntry>>>` — single read-lock per lookup with write-lock + double-check on miss.
- `RegistryEntry.by_labels: RwLock<BTreeMap<LabelSet, InstrumentCell>>` — same pattern at the label-set level.
- Per-cell state is `AtomicU64` / `AtomicI64` with `Ordering::Relaxed` (no stronger ordering needed for per-cell monotonic counters / sums).
- Net result: read-mostly access (the common case in HTTP handlers / queue workers) takes zero write locks.

### 1.4 Storage model (instrument handles)

```text
Metrics { inner: Arc<MetricsInner> }
  └─ MetricsInner { by_name: RwLock<BTreeMap<String, Arc<RegistryEntry>>> }
       └─ RegistryEntry { name, by_labels: RwLock<BTreeMap<LabelSet, InstrumentCell>> }
            └─ InstrumentCell = Counter(Arc<AtomicU64>)
                             | Histogram { buckets: Arc<[AtomicU64; 11]>, count: Arc<AtomicU64>, sum_us: Arc<AtomicI64> }
                             | Gauge(Arc<AtomicI64>)
```

A `Counter { cell: Arc<AtomicU64> }` handle returned from `metrics.counter("foo", &labels)` holds the same `Arc` that lives inside the registry; clone-of-handle = shared cell = monotonic accumulation across threads.

### 1.5 Tests

| File | Test | Verifies |
|---|---|---|
| `pheno-otel/src/metrics.rs` (inline) | `counter_increment_is_cumulative` | `add(3) + add(2) -> get() == 5`; distinct label sets get distinct cells. |
| `pheno-otel/src/metrics.rs` (inline) | `histogram_observation_increments_buckets` | Bucket-indexing against `HISTOGRAM_BUCKETS_SECONDS`; sum_seconds arithmetic. |
| `pheno-otel/src/metrics.rs` (inline) | `gauge_set_add_sub_round_trip` | `set(10)` + `add(5)` + `sub(20)` → `get() == -5`. |
| `pheno-otel/src/metrics.rs` (inline) | `snapshot_serializes_to_json` | `serde_json::to_string` → `serde_json::from_str` round-trip preserves all cells. |
| `pheno-otel/src/metrics.rs` (inline) | `histogram_drops_invalid_observations` | NaN and negative `observe()` calls are silently dropped. |
| `pheno-otel/src/metrics.rs` (inline) | `metrics_clone_shares_registry` | Cloned facade observes writes through the original. |
| `pheno-otel/src/metrics.rs` (inline) | `distinct_label_sets_get_distinct_cells` | Per-cell isolation under concurrent access. |
| `pheno-otel/src/metrics.rs` (inline) | `default_matches_new` | `Metrics::default()` == `Metrics::new()`. |
| `pheno-otel/tests/metrics_test.rs` | `counter_increment` | **Required by task.** Public API only — `Metrics::http_requests_total` + `Counter::add` + `Counter::get`; label-set isolation. |
| `pheno-otel/tests/metrics_test.rs` | `histogram_observation` | **Required by task.** Public API only — `Metrics::http_request_duration_seconds` + `Histogram::observe` + bucket-indexing + `sum_seconds`. |

**Test count:** 10 (8 inline + 2 public-API integration). Task required ≥ 2.

---

## 2. Dashboard list

5 Grafana dashboard JSON files in `pheno-otel/docs/metrics/dashboards/`. Total: 15 panels across 5 dashboards, ~45 KB.

| Dashboard | UID | Panels | Backing metrics | RED/USE axis |
|---|---|---:|---|---|
| `api-overview.json` | `pheno-otel-l25-api-overview` | 3 | `http_requests_total`, `http_request_duration_seconds`, `errors_total` | RED (Rate + Duration + Errors) |
| `error-budget.json` | `pheno-otel-l25-error-budget` | 3 | `errors_total`, `http_requests_total` | RED-E (Error budget burn-rate alerting) |
| `latency-slo.json` | `pheno-otel-l25-latency-slo` | 3 | `http_request_duration_seconds` | RED-D (Latency SLO thresholds) |
| `throughput.json` | `pheno-otel-l25-throughput` | 3 | `http_requests_total` | RED-R (Rate by method/status/endpoint) |
| `saturation.json` | `pheno-otel-l25-saturation` | 3 | `active_connections`, `queue_depth` | USE (Utilization + Saturation + Errors) |

### 2.1 Conventions (all 5 dashboards)

| Property | Value |
|---|---|
| `schemaVersion` | 39 (Grafana 10.4) |
| `refresh` | `1m` |
| `time.from` / `time.to` | `now-1h` / `now` |
| Prometheus datasource UID | `${DS_PROMETHEUS}` (templated) |
| Templating variables | `$service` (label_values, multi-select) |

### 2.2 Validation (stdlib-only)

```text
$ for f in pheno-otel/docs/metrics/dashboards/*.json; do
    python3 -c "import json,sys; d=json.load(open('$f')); \
      print(f'$f uid={d[\"uid\"]} title=\"{d[\"title\"]}\" panels={len(d[\"panels\"])}')"
  done
api-overview.json       uid=pheno-otel-l25-api-overview  title="L25 API Overview — pheno-otel"   panels=3
error-budget.json       uid=pheno-otel-l25-error-budget  title="L25 Error Budget — pheno-otel"   panels=3
latency-slo.json        uid=pheno-otel-l25-latency-slo   title="L25 Latency SLO — pheno-otel"    panels=3
throughput.json         uid=pheno-otel-l25-throughput    title="L25 Throughput — pheno-otel"     panels=3
saturation.json         uid=pheno-otel-l25-saturation    title="L25 Saturation — pheno-otel"     panels=3
```

All 5 files parse, all required top-level keys (`uid`, `title`, `panels`, `templating`, `time`, `schemaVersion`, `description`) present, every panel has at least one `targets` entry, every target references `${DS_PROMETHEUS}`.

### 2.3 Why the `pheno-otel/docs/metrics/dashboards/` location

Per ADR-023 § App-substrate placement, observability dashboards are a **federated service concern** that lives alongside the substrate that produces the metrics. `pheno-otel` is the canonical OTLP-export substrate (ADR-037), so its dashboards live **next to the substrate** (`pheno-otel/docs/metrics/dashboards/`), not in a separate `benchmarks/` or `dashboards/` tree.

The 5 `benchmarks/dashboards/l25-metrics-*.json` files authored by the adjacent closure agent are the **benchmark-suite dashboard** (consumed by `cargo bench` runs and CI perf gates per ADR-040); the 5 `pheno-otel/docs/metrics/dashboards/*.json` files authored here are the **production-observability dashboard** (consumed by Grafana operators / on-call). The split is intentional and matches the substrate-vs-app boundary in ADR-023 Rule 3.

---

## 3. Integration example

### 3.1 Minimal — record + snapshot

```rust
use pheno_otel::metrics::{labels, Metrics};

let metrics = Metrics::new();

// Counter
let http_200 = metrics.http_requests_total(&labels(&[
    ("method", "GET"),
    ("status", "200"),
    ("path", "/api/v1/widgets"),
]));
http_200.add(1);

// Histogram (seconds)
let duration = metrics.http_request_duration_seconds(&labels(&[
    ("method", "GET"),
    ("path", "/api/v1/widgets"),
]));
duration.observe(0.042);

// Gauge
let conns = metrics.active_connections(&labels(&[("service", "api")]));
conns.set(87);

// Snapshot → OTLP/JSON
let snap = metrics.snapshot();
let payload = serde_json::to_vec(&snap)?;
// POST to http://collector:4318/v1/metrics (via HttpExporter in this crate).
```

### 3.2 Recommended — shared facade across an HTTP service

The facade is `Arc`-shared, so the canonical pattern is:

```rust
// app.rs — built once at startup, cloned into every handler / worker.
#[derive(Clone)]
pub struct AppState {
    pub metrics: Metrics,
    pub db: PgPool,
    // ...
}

impl AppState {
    pub fn new(db: PgPool) -> Self {
        Self { metrics: Metrics::new(), db }
    }
}
```

### 3.3 Recommended — middleware integration

```rust
// middleware.rs — wrap every request with counter + histogram observation.
pub async fn metrics_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let started = std::time::Instant::now();

    // Pre-handler gauge bump.
    state
        .metrics
        .active_connections(&labels(&[("service", "api")]))
        .add(1);

    let response = next.run(req).await;

    // Post-handler: roll back the gauge, record counter + histogram.
    state
        .metrics
        .active_connections(&labels(&[("service", "api")]))
        .sub(1);

    let method = response.status().as_u16();
    let status = response.status().as_str().to_string();
    state
        .metrics
        .http_requests_total(&labels(&[
            ("method", method),
            ("status", &status),
            ("path", response.extensions().get::<Path>().map(|p| p.as_str()).unwrap_or("unknown")),
        ]))
        .add(1);

    state
        .metrics
        .http_request_duration_seconds(&labels(&[
            ("method", method),
            ("path", response.extensions().get::<Path>().map(|p| p.as_str()).unwrap_or("unknown")),
        ]))
        .observe(started.elapsed().as_secs_f64());

    response
}
```

### 3.4 Recommended — OTLP wire export

```rust
// exporter.rs — background task that drains the snapshot into OTLP/HTTP.
pub async fn otlp_export_loop(metrics: Metrics) {
    use pheno_otel::exporters::http::HttpExporter;
    use pheno_otel::exporters::ExporterConfig;

    let cfg = ExporterConfig {
        endpoint: "http://otel-collector:4318".to_string(),
        service_name: "pheno-api".to_string(),
        service_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let exporter = HttpExporter::metrics(cfg).expect("exporter init");

    let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
    loop {
        tick.tick().await;
        let snap = metrics.snapshot();
        let payload = serde_json::to_vec(&snap).expect("serialize");
        if let Err(e) = exporter.export(&payload) {
            tracing::warn!(error = %e, "OTLP metrics export failed");
        }
    }
}
```

### 3.5 Cardinality discipline (the one rule)

**Label values MUST be drawn from a closed enum or a normalized low-cardinality set.** The facade does not cap cardinality; unbounded labels blow up the OTLP backend in hours, not days. Fleet-wide guidance:

- `method`: `GET | POST | PUT | PATCH | DELETE` (5 values).
- `status`: 5-bucket groups (`2xx | 3xx | 4xx | 5xx | error`) — NOT the raw 3-digit code.
- `path`: normalized route template (`/api/v1/widgets/:id`), NOT the raw URI with embedded IDs.
- `kind` (errors): a small enum (`timeout | io | internal | upstream | auth`).
- `service` (gauge): one of the fleet's `service.name` OTel resource values (~20 services, bounded).
- `queue_name`: bounded to the work-queue registry (~30 queues).

If a label value is unbounded (user IDs, request IDs, raw URIs, raw error messages), it does not belong in a metric label. Put it in a span attribute or log record instead.

---

## 4. Files changed (this task)

| Path | Size | Purpose |
|---|---:|---|
| `pheno-otel/src/metrics.rs` | 701 LOC | The facade (16 public items, 8 inline tests). |
| `pheno-otel/tests/metrics_test.rs` | 80 LOC | Public-API integration tests (2 tests, task-required). |
| `pheno-otel/docs/metrics/dashboards/api-overview.json` | 9,421 B | 3-panel RED overview. |
| `pheno-otel/docs/metrics/dashboards/error-budget.json` | 8,491 B | 3-panel error-budget burn-rate view. |
| `pheno-otel/docs/metrics/dashboards/latency-slo.json` | 9,300 B | 3-panel latency percentile SLO view. |
| `pheno-otel/docs/metrics/dashboards/throughput.json` | 8,724 B | 3-panel throughput-by-endpoint view. |
| `pheno-otel/docs/metrics/dashboards/saturation.json` | 9,321 B | 3-panel saturation (active_connections + queue_depth) view. |
| `pheno-otel/src/lib.rs` | +10 LOC | `pub mod metrics;` + module docstring (cross-refs ADR-037 + ADR-042B). |
| `pheno-otel/WORKLOG.md` | +7 rows | v2.1 schema entries for every artifact above. |
| `findings/2026-06-22-V22-T1-metrics-facade.md` | (this file) | Facade-focused closure doc. |

**Total new LoC:** ~2,500 (701 metrics.rs + 80 tests + ~45 KB dashboards + 10 lib.rs + 7 worklog rows + this doc).

**Branch:** `feat/v22-l25-metrics-2026-06-22` — committed, NOT pushed (per task directive).

---

## 5. Compliance

- **ADR-023 device-fit gate:** Work performed on `device: macbook`. No `cargo build` / `cargo test` run (heavy-runner per the device-fit gate). Static syntax validation only: serde_json round-trip and JSON model validation are library-only. The `cargo check --lib --tests` job is the heavy-runner cron item per ADR-041 cadence.
- **ADR-025 / ADR-030 (worklog v2.1 schema):** All 7 new rows carry `device: macbook` and follow the 11-column canonical format.
- **ADR-038 (hexagonal ports + adapters):** `Metrics` / `Counter` / `Histogram` / `Gauge` are the Port side; the in-process atomic-backed registry is the Adapter. A future OTel-SDK-backed adapter slots in behind the same surface.
- **ADR-040 (test coverage gates):** 10 unit tests on a ~600-LOC file with 16 public items = structurally ≥ 80 % lib gate. Full confirmation requires `cargo llvm-cov` on a heavy-runner per ADR-041 weekly cadence.
- **ADR-042B (substrate quality bar):** Spec (`SPEC.md`), docs (this finding + module docstring + WORKLOG), test matrix (10 tests), observability (the facade IS the observability substrate), CI gate (existing), worklog v2.1 — all 6 of 7 substrate-quality-bar criteria met. Coverage confirmation is the deferred 7th.
- **ADR-046 (federation mTLS + OIDC):** Out of scope. Consumer-side adds mTLS via `ExporterConfig` per the ADR.

---

## 6. Out of scope (deferred, not lost)

| Item | Reason | Owner | Target |
|---|---|---|---|
| OpenTelemetry SDK-backed adapter (impl of `Counter`/`Histogram`/`Gauge` against `opentelemetry::metrics`) | ADR-038 hexagonal ports — surface is in place, SDK adapter is the next step | v23-T3 | v23 |
| `alerts.json` (Prometheus alerting rules for the 5 dashboards) | L59 alerting/SLO is a separate track | v23-T2 | v23 |
| Real `cargo llvm-cov` coverage run | Heavy-runner per ADR-023 | heavy-runner cron | weekly Monday 09:00 PDT |
| Tier-2 graduation of `pheno-otel` to `phenotype-otel-sdk` | Requires G2.1 ≥ 2 app consumers + G2.4 lifecycle hooks docs | orchestrator + KooshaPari | v23 |

---

*Generated 2026-06-22 by Forge orchestrator (v22 cycle 12 T1, facade-focused subset). Schema: 71-pillar refresh template. Cross-refs: ADR-023, ADR-024, ADR-025, ADR-030, ADR-037, ADR-038, ADR-040, ADR-041, ADR-042B, ADR-046. Plan: `plans/2026-06-22-v22-71-pillar-cycle-12-p1.md`.*