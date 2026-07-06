//! Async poller: drives the poll loop, records samples into `Metrics`.
//!
//! The poller is intentionally simple: one tokio task that pings the
//! gateway every `poll_interval`, classifies the response, and feeds the
//! resulting `Sample` to the shared `Metrics` registry. SLO burn-rate
//! recomputation uses an approximate sliding counter; production should
//! swap in a ring buffer (see docs/SLO_SPEC.md).

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
}

/// Errors the poller can encounter.
#[derive(Debug, Error)]
pub enum PollError {
    #[error("HTTP transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// Aggregated counters per (provider, SLO) used by the burn-rate calculator.
#[derive(Default, Debug)]
struct SlidingCounters {
    short_success: u64,
    short_failure: u64,
    long_success: u64,
    long_failure: u64,
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
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(tok) = &config.bearer_token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {tok}").parse().map_err(|_| {
                    PollError::InvalidConfig("invalid bearer token".into())
                })?,
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
    pub fn registry(&self) -> Arc<Registry> { self.inner.registry.clone() }

    /// Run the poll loop forever (or until SIGINT/SIGTERM).
    pub async fn run(&self) -> anyhow::Result<()> {
        let cfg = self.inner.config.clone();
        info!(
            target = %cfg.target,
            interval_secs = cfg.poll_interval.as_secs(),
            "argis-monitor starting"
        );

        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

        let mut ticker = tokio::time::interval(cfg.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match self.poll_once().await {
                        Ok(outcome) => debug!(?outcome, "poll ok"),
                        Err(err) => warn!(error = %err, "poll failed"),
                    }
                }
                _ = sigterm.recv() => { info!("SIGTERM, exiting"); break; }
                _ = sigint.recv()  => { info!("SIGINT, exiting");  break; }
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
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

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
        match sample.outcome {
            Outcome::Ok => {
                c.short_success += 1;
                c.long_success += 1;
            }
            Outcome::Error => {
                c.short_failure += 1;
                c.long_failure += 1;
            }
        }

        let mut burn_short = 0.0_f64;
        let mut burn_long = 0.0_f64;
        for slo in &self.inner.config.slos {
            let bs = burn_rate(c.short_success, c.short_failure, slo.target);
            let bl = burn_rate(c.long_success, c.long_failure, slo.target);
            m.record_burn(&slo.name, BurnWindow::FAST_BURN, bs);
            m.record_burn(&slo.name, BurnWindow::SLOW_BURN, bl);
            burn_short = bs;
            burn_long = bl;
        }
        Ok(PollOutcome { sample, burn_short, burn_long })
    }

    /// Get a clone of the active config.
    pub fn config(&self) -> Config { self.inner.config.clone() }

    /// Reference the canonical fast-burn / slow-burn windows.
    pub fn windows(&self) -> &'static [BurnWindow] {
        &[BurnWindow::FAST_BURN, BurnWindow::SLOW_BURN]
    }
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
    pub fn with_window_secs(mut self, secs: u64) -> Self { self.window_secs = secs; self }
    /// Convenience builder.
    pub fn with_target(mut self, target: f64) -> Self { self.target = target; self }
}
