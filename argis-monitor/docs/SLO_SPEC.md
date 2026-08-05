# SLO specification

The monitor implements the **Google SRE multi-window burn-rate alert** recipe (https://sre.google/workbook/alerting-on-slos/).

## Burn rate

For an SLO with target `T` (success ratio) over window `W`:

```
burn = (1 - success_ratio_W) / (1 - T)
```

A burn rate of 1.0 means the error budget is being consumed at the sustainable rate. A burn rate of 2.0 means the budget will be exhausted in half the window. A burn rate of `INFINITY` means any failure at all is unbounded (i.e. `T = 1.0`).

## Multi-window

Two windows are tracked per SLO:

| Window | Purpose | Alert threshold |
|--------|---------|-----------------|
| Short (5m / 30m) | Fast burn (likely acute incident) | burn > 14.4 |
| Long (1h / 6h) | Slow burn (likely regression) | burn > 14.4 |

The SRE recipe can be expressed through the monitor's `AlertRule` configuration. Each rule selects an SLO and optional trailing window; the evaluator emits webhook payloads as the rule moves through `Pending`, `Firing`, and `Ok`.

## Implementation note: ring-buffer windows

The implementation uses a fixed-size `RingBuffer` of timestamped `(success, failure)` buckets. Queries include the newest bucket and exclude buckets older than the requested cutoff, so alert evaluation uses trailing-window error rates rather than cumulative-since-startup counters.

## Tests

The SLO math is unit-tested in `src/slo.rs`:

- Zero traffic returns zero burn.
- All successes returns zero burn.
- At-target error rate returns 1x burn.
- 10x error rate against 99.9% target returns 10x burn.
- `target=1.0` returns `INFINITY` for any failure, `0.0` for zero.

The integration test `multi_window_burn_reflects_error_traffic` confirms the burn rate rises after a 503.
