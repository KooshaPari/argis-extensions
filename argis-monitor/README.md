# argis-monitor

Observable Integration substrate for [bifrost-extensions](https://github.com/KooshaPari/argis-extensions) (Tenet 4 of the [charter](../CHARTER.md)).

Polls the Bifrost gateway at a configurable interval, computes the current SLO burn rate, and exposes the result as Prometheus metrics on a local HTTP endpoint.

## Why

The bifrost-extensions charter's Tenet 4 says:

> Extension behavior is fully observable. Metrics, logs, and traces flow through the same pipeline as core components. No black box integrations.

`argis-monitor` is the runtime substrate that turns "extension X is healthy" into something a Prometheus alert manager can graph, alert on, and route. It's a small, focused Rust binary that fits next to the existing Go CLI as a peer service.

## Quick start

```bash
# Build
cargo build -p argis-monitor --release

# Run with defaults (polls http://127.0.0.1:8080 every 15s, exposes /metrics on :9090)
./target/release/argis-monitor start

# Or via config file
./target/release/argis-monitor --config examples/basic.yaml start

# One-shot poll (for cron)
./target/release/argis-monitor once --target http://127.0.0.1:8080

# Validate config
./target/release/argis-monitor validate-config --config examples/basic.yaml
```

## Metrics exposed

| Metric | Type | Labels | Meaning |
|--------|------|--------|---------|
| `argis_monitor_polls_total` | counter | `provider`, `outcome` | Total polls attempted |
| `argis_monitor_poll_errors_total` | counter | `provider`, `error_kind` | Polls that failed |
| `argis_monitor_poll_duration_seconds` | histogram | `provider` | Poll latency distribution |
| `argis_monitor_last_poll_timestamp_seconds` | gauge | `provider` | Unix timestamp of last successful poll |
| `argis_monitor_up` | gauge | `provider` | 1 if last poll succeeded, else 0 |
| `argis_monitor_burn_rate` | gauge | `slo`, `window` | Current SLO burn rate (multiplier of error budget consumption) |
| `argis_monitor_slo_target` | gauge | `slo` | Configured SLO target ratio |
| `argis_monitor_target_info` | info | `target`, `version` | Static info about the target gateway |

## Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design, including:

- Why one tokio task drives the poll loop
- Why the Prometheus registry is shared via `Arc<Registry>` rather than reconstructed per scrape
- How the sliding window counter approximates the SRE multi-window burn-rate recipe
- Where to swap in a ring buffer when traffic exceeds the simple counter

## SLO math

See [docs/SLO_SPEC.md](docs/SLO_SPEC.md) for the exact formulas, including the Google SRE multi-window recipe (5m/1h fast burn, 30m/6h slow burn).

## License

MIT OR Apache-2.0
