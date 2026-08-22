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

The SRE recipe alerts when **both** the short and long windows exceed 14.4x burn simultaneously. The monitor records both windows as separate gauges (`argis_monitor_burn_rate{window="short"}` and `{window="long"}`); the alerting rule lives in the Prometheus config (out of scope for this crate).

## Implementation note: simple counter, not ring buffer

The first slice uses a single `SlidingCounters` struct with four `u64` fields. Both "short" and "long" are cumulative-since-startup approximations. This is **wrong** for windows longer than ~1 hour under realistic traffic; the values drift toward the cumulative error rate, not the trailing-window error rate.

The fix is a per-window ring buffer of timestamped `(success, failure)` pairs. The interface is already prepared for this:

```rust
struct SlidingCounters {
    short_success: u64,  short_failure: u64,
    long_success: u64,   long_failure: u64,
}
```

The next slice replaces these four fields with two `RingBuffer<Bucket>` (one per window) where each `Bucket` covers a coarse-grained time slice. The public `PollOutcome { burn_short, burn_long }` stays stable.

## Tests

The SLO math is unit-tested in `src/slo.rs`:

- Zero traffic returns zero burn.
- All successes returns zero burn.
- At-target error rate returns 1x burn.
- 10x error rate against 99.9% target returns 10x burn.
- `target=1.0` returns `INFINITY` for any failure, `0.0` for zero.

The integration test `multi_window_burn_reflects_error_traffic` confirms the burn rate rises after a 503.
