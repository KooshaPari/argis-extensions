# Architecture

## Components

```
        Bifrost gateway  (Go, port 8080)
              ^
              | HTTP GET /health every poll_interval
              |
        +-----+-----+
        |  Monitor  |  (argis-monitor poller, tokio task)
        +-----+-----+
              |
              |  Sample  (provider, outcome, latency, status_code)
              v
        +-----+-----+         +-------------+
        |  Metrics  +-------->|  Registry   |  (prometheus_client::Registry, Arc<>)
        +-----------+         +------+------+
                                     |
                                     |  encode() on /metrics scrape
                                     v
                              +------+------+
                              |   axum     |  (exporter, separate tokio task)
                              |  :9090     |
                              +------+------+
                                     |
                                     |  HTTP GET /metrics every 15s
                                     v
                              Prometheus scrape
```

## Decisions

### One tokio task drives the poll loop

The poll loop is a single `tokio::time::interval` ticker. Multi-window burn rate recomputation happens inline at the end of each tick. This keeps the runtime story simple: one task, no shared mutable state across tasks except the registry. The exporter runs in its own task (`axum::serve`) and only reads the registry.

### Prometheus registry is shared via `Arc<Registry>`

The `prometheus_client::Registry` is internally synchronized (`parking_lot` mutexes). Sharing via `Arc<Registry>` means:
- The poller can write samples without locking the exporter.
- The exporter can encode the registry on scrape without locking the poller.
- No allocation on the hot path (text encoding walks the existing metric families).

### Sliding window counter

The monitor uses a fixed-size `RingBuffer` of timestamped `(success, failure)` buckets for each target. Window queries walk the newest buckets and stop at the requested cutoff, so short and long burn-rate calculations use trailing-window semantics. Advancing the ring rotates buckets without shifting the remaining history.

### Why axum for the exporter

The Bifrost gateway stack already uses `chi` (Go) and the Bifrost core is on `connectrpc`. Picking `axum` on the Rust side keeps the dependency footprint small and matches the rest of the Phenotype fleet.

## Failure modes

| Failure | Behaviour |
|---------|-----------|
| Gateway unreachable | `poll_errors_total{error_kind="transport"}` increments; `argis_monitor_up=0` |
| Gateway returns 5xx | `poll_errors_total{error_kind="upstream_5xx"}` increments; `argis_monitor_up=0` |
| Gateway returns 4xx | `poll_errors_total{error_kind="upstream_4xx"}` increments; `argis_monitor_up=0` |
| Bearer token rejected (401/403) | `poll_errors_total{error_kind="auth"}` increments; `argis_monitor_up=0` |
| Gateway rate-limits (429) | `poll_errors_total{error_kind="rate_limit"}` increments; `argis_monitor_up=0` |
| Exporter scrape times out | Standard Prometheus retry; no side effects on the poller |
| Poller crashes (panic in tokio task) | Process exits non-zero; orchestrator restarts |

## Versioning

- Bump the crate `version` on any breaking change to the public API, the CLI surface, or the metric names/labels.
- Bump `argis_monitor_target_info{version}` automatically follows the crate version.
