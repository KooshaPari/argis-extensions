//! Async poller: drives the poll loop, records samples into `Metrics`.
//!
//! The poller is intentionally simple: one tokio task that pings the
//! gateway every `poll_interval`, classifies the response, and feeds the
//! resulting `Sample` to the shared `Metrics` registry. SLO burn-rate
//! recomputation uses an approximate sliding counter; production should
//! swap in a ring buffer (see docs/SLO_SPEC.md).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use prometheus_client::registry::Registry;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::config::{Config, SLO};
use crate::metrics::{Metrics, Outcome, Sample};
use crate::slo::{burn_rate, BurnWindow};

/// One poll's outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PollOutcome {
    pub sample: Sample,
    pub burn_short: f64,
    pub burn_long: f64,
    /// Burn values keyed by SLO. The legacy pair above mirrors the first SLO.
    pub burn_rates: Vec<SloBurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SloBurn {
    pub slo: String,
    pub burn_short: f64,
    pub burn_long: f64,
}

/// Errors the poller can encounter.
#[derive(Debug, Error)]
pub enum PollError {
    #[error("HTTP transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Timestamped outcomes retained only for the configured rolling windows.
#[derive(Default, Debug)]
struct SlidingCounters {
    samples: VecDeque<(u64, bool)>,
}

impl SlidingCounters {
    fn push(&mut self, timestamp_secs: u64, success: bool) {
        self.samples.push_back((timestamp_secs, success));
    }

    fn prune(&mut self, now: u64, max_window_secs: u64) {
        let cutoff = now.saturating_sub(max_window_secs);
        while self.samples.front().is_some_and(|(ts, _)| *ts < cutoff) {
            self.samples.pop_front();
        }
    }

    fn counts(&self, now: u64, window_secs: u64) -> (u64, u64) {
        let cutoff = now.saturating_sub(window_secs);
        self.samples.iter().filter(|(ts, _)| *ts >= cutoff).fold(
            (0, 0),
            |(success, failure), (ts, ok)| {
                if *ts < cutoff {
                    (success, failure)
                } else if *ok {
                    (success + 1, failure)
                } else {
                    (success, failure + 1)
                }
            },
        )
    }
}

/// The monitor: shared registry + metrics + config + HTTP client.
#[derive(Clone)]
pub struct Monitor {
    inner: Arc<MonitorInner>,
}

pub(crate) struct MonitorInner {
    pub config: Config,
    pub http: reqwest::Client,
    pub registry: Arc<Registry>,
    pub metrics: Arc<Mutex<Metrics>>,
    pub counters: Mutex<SlidingCounters>,
}

impl Monitor {
    /// Build a new monitor from `config`.
    pub fn new(config: Config) -> Result<Self, PollError> {
        config.validate().map_err(PollError::InvalidConfig)?;
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(tok) = &config.bearer_token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {tok}")
                    .parse()
                    .map_err(|_| PollError::InvalidConfig("invalid bearer token".into()))?,
            );
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(config.poll_timeout)
            .build()
            .map_err(PollError::Transport)?;

        let mut registry = Registry::default();
        let metrics = Metrics::new(&mut registry, &config.target);
        for slo in &config.slos {
            metrics.record_slo_target(&slo.name, slo.target);
        }

        Ok(Self {
            inner: Arc::new(MonitorInner {
                config,
                http,
                registry: Arc::new(registry),
                metrics: Arc::new(Mutex::new(metrics)),
                counters: Mutex::new(SlidingCounters::default()),
            }),
        })
    }

    /// Borrow the underlying registry (used by the exporter).
    pub fn registry(&self) -> Arc<Registry> {
        self.inner.registry.clone()
    }

    /// Run the poll loop forever (or until SIGINT/SIGTERM).
    pub async fn run(&self) -> anyhow::Result<()> {
        let cfg = self.inner.config.clone();
        info!(
            target = %cfg.target,
            interval_secs = cfg.poll_interval.as_secs(),
            "argis-monitor starting"
        );

        let mut ticker = tokio::time::interval(cfg.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let shutdown = shutdown_signal();
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match self.poll_once().await {
                        Ok(outcome) => debug!(?outcome, "poll ok"),
                        Err(err) => warn!(error = %err, "poll failed"),
                    }
                }
                signal = &mut shutdown => {
                    info!(signal = signal?, "shutdown signal, exiting");
                    break;
                }
            }
        }
        Ok(())
    }

    /// Run exactly one poll + SLO recompute.
    pub async fn poll_once(&self) -> Result<PollOutcome, PollError> {
        let started = Instant::now();
        let url = format!("{}/health", self.inner.config.target.trim_end_matches('/'));
        let res = self.inner.http.get(&url).send().await;
        let latency = started.elapsed();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let sample = match res {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if resp.status().is_success() {
                    Sample {
                        provider: "gateway".into(),
                        outcome: Outcome::Ok,
                        latency,
                        status_code: status,
                        timestamp_secs: ts,
                    }
                } else {
                    Sample {
                        provider: "gateway".into(),
                        outcome: Outcome::Error,
                        latency,
                        status_code: status,
                        timestamp_secs: ts,
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "transport error");
                Sample {
                    provider: "gateway".into(),
                    outcome: Outcome::Error,
                    latency,
                    status_code: 0,
                    timestamp_secs: ts,
                }
            }
        };

        let mut m = self.inner.metrics.lock().await;
        m.record_sample(&sample);

        let mut c = self.inner.counters.lock().await;
        c.push(ts, sample.outcome == Outcome::Ok);
        let max_window = self
            .inner
            .config
            .slos
            .iter()
            .map(|slo| slo.window_secs)
            .max()
            .unwrap_or(0)
            .min(BurnWindow::SLOW_BURN.long.as_secs());
        c.prune(ts, max_window);

        let mut burn_rates = Vec::with_capacity(self.inner.config.slos.len());
        for slo in &self.inner.config.slos {
            let fast_short = BurnWindow::FAST_BURN.short.as_secs().min(slo.window_secs);
            let fast_long = BurnWindow::FAST_BURN.long.as_secs().min(slo.window_secs);
            let slow_short = BurnWindow::SLOW_BURN.short.as_secs().min(slo.window_secs);
            let slow_long = BurnWindow::SLOW_BURN.long.as_secs().min(slo.window_secs);
            let (fast_short_success, fast_short_failure) = c.counts(ts, fast_short);
            let (fast_long_success, fast_long_failure) = c.counts(ts, fast_long);
            let (slow_short_success, slow_short_failure) = c.counts(ts, slow_short);
            let (slow_long_success, slow_long_failure) = c.counts(ts, slow_long);
            let bs = burn_rate(fast_short_success, fast_short_failure, slo.target);
            let bl = burn_rate(fast_long_success, fast_long_failure, slo.target);
            let slow_bs = burn_rate(slow_short_success, slow_short_failure, slo.target);
            let slow_bl = burn_rate(slow_long_success, slow_long_failure, slo.target);
            m.record_burn(&slo.name, BurnWindow::FAST_BURN, bs);
            m.record_burn(&slo.name, BurnWindow::SLOW_BURN, slow_bl.max(slow_bs));
            burn_rates.push(SloBurn {
                slo: slo.name.clone(),
                burn_short: bs,
                burn_long: bl,
            });
        }
        let (burn_short, burn_long) = burn_rates
            .first()
            .map(|r| (r.burn_short, r.burn_long))
            .unwrap_or((0.0, 0.0));
        Ok(PollOutcome {
            sample,
            burn_short,
            burn_long,
            burn_rates,
        })
    }

    /// Get a clone of the active config.
    pub fn config(&self) -> Config {
        self.inner.config.clone()
    }

    /// Reference the canonical fast-burn / slow-burn windows.
    pub fn windows(&self) -> &'static [BurnWindow] {
        &[BurnWindow::FAST_BURN, BurnWindow::SLOW_BURN]
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> anyhow::Result<&'static str> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    tokio::select! {
        _ = sigterm.recv() => Ok("SIGTERM"),
        _ = sigint.recv() => Ok("SIGINT"),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> anyhow::Result<&'static str> {
    tokio::signal::ctrl_c().await?;
    Ok("CTRL_C")
}

impl Config {
    /// Convenience for tests.
    #[doc(hidden)]
    pub fn for_test(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            ..Default::default()
        }
    }
}

impl SLO {
    /// Convenience builder.
    pub fn with_window_secs(mut self, secs: u64) -> Self {
        self.window_secs = secs;
        self
    }
    /// Convenience builder.
    pub fn with_target(mut self, target: f64) -> Self {
        self.target = target;
        self
    }
}
