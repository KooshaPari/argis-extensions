//! Criterion bench for the burn-rate math.
//!
//! Gate: 1M burn-rate calcs in < 50ms (i.e. ~20ns each). The math is pure
//! f64 arithmetic; CI runs this to catch regressions.

use criterion::{criterion_group, criterion_main, Criterion};
use argis_monitor::slo::burn_rate;

fn bench_burn_rate(c: &mut Criterion) {
    c.bench_function("burn_rate(1M, 1K, 0.999)", |b| {
        b.iter(|| burn_rate(1_000_000, 1_000, 0.999))
    });
}

fn bench_burn_rate_zero_traffic(c: &mut Criterion) {
    c.bench_function("burn_rate(0, 0, 0.999)", |b| {
        b.iter(|| burn_rate(0, 0, 0.999))
    });
}

criterion_group!(benches, bench_burn_rate, bench_burn_rate_zero_traffic);
criterion_main!(benches);
