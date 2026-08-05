//! Async poller: drives one tokio task per target, accumulates samples into
//! the shared `Metrics` registry, and feeds the `RingBuffer` per target for
//! proper SLO multi-window burn-rate computation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use prometheus_client::registry::Registry;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};


use crate::alerts::{self, AlertRule, AlertStateTracker, Decision};
use crate::config::{Config, SLO};
use crate::metrics::{Metrics, Outcome, Sample};
use crate::ring_buffer::RingBuffer;
use crate::slo::{burn_rate, BurnWindow};
use crate::target::Target;
use crate::webhook;

/// One poll's outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PollOutcome {
    pub sample: Sample,
    pub burn_short: f64,
    pub burn_long: f64,
    #[serde(default)]
    pub alert_payloads: Vec<crate::alerts::AlertPayload>,
}

/// Errors the poller can encounter.
#[derive(Debug, Error)]
#[allow(clippy::large_enum_variant)]
pub enum PollError {
    #[error("HTTP transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("no targets configured")]
    NoTargets,
}

/// Per-target ring buffer state.
struct TargetCounters {
    short: RingBuffer,
    long: RingBuffer,
}

/// The monitor: shared registry + metrics + per-target ring buffers + HTTP client.
#[derive(Clone)]
pub struct Monitor {
    inner: Arc<MonitorInner>,
}

pub(crate) struct MonitorInner {
    pub config: Config,
    pub http: reqwest::Client,
    pub registry: Arc<Registry>,
    pub metrics: Arc<Mutex<Metrics>>,
    pub counters: Mutex<HashMap<String, TargetCounters>>,
    /// Per-(target, rule) state machine. Keyed by "{target}::{rule.name}".
    pub alert_trackers: Mutex<HashMap<String, AlertStateTracker>>,
    /// Last delivery report per webhook URL (for tests + ops introspection).
    pub last_delivery: Mutex<HashMap<String, webhook::DeliveryReport>>,
}

impl Monitor {
    /// Build a new monitor from `config`. Errors if `config.targets` is empty.
    pub fn new(config: Config) -> Result<Self, PollError> {
        if config.targets.is_empty() {
            return Err(PollError::NoTargets);
        }
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
        let metrics = Metrics::new(&mut registry, config.first_url());
        for slo in &config.slos {
            metrics.record_slo_target(&slo.name, slo.target);
        }

        // Pre-create per-target ring buffers covering the longest SLO window.
        let max_window_secs = config.slos.iter().map(|s| s.window_secs).max().unwrap_or(30 * 86_400);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let mut counters = HashMap::new();
        for target in &config.targets {
            counters.insert(target.name.clone(), TargetCounters {
                short: RingBuffer::new(BurnWindow::FAST_BURN.long.as_secs(), now),
                long: RingBuffer::new(max_window_secs, now),
            });
        }

        let mut alert_trackers = HashMap::new();
        for target in &config.targets {
            for rule in &config.alert_rules {
                alert_trackers.insert(format!("{}::{}", target.name, rule.name), AlertStateTracker::default());
            }
        }
        Ok(Self {
            inner: Arc::new(MonitorInner {
                config,
                http,
                registry: Arc::new(registry),
                metrics: Arc::new(Mutex::new(metrics)),
                counters: Mutex::new(counters),
                alert_trackers: Mutex::new(alert_trackers),
                last_delivery: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn registry(&self) -> Arc<Registry> { self.inner.registry.clone() }
    pub fn config(&self) -> Config { self.inner.config.clone() }

    /// Run all configured targets in parallel. Blocks until SIGINT/SIGTERM.
    pub async fn run(&self) -> anyhow::Result<()> {
        let cfg = self.inner.config.clone();
        info!(
            targets = cfg.targets.len(),
            exporter_addr = %cfg.exporter_addr,
            "argis-monitor starting"
        );

        // Spawn one task per target.
        let mut handles = Vec::with_capacity(cfg.targets.len());
        for target in cfg.targets.clone() {
            let me = self.clone();
            let interval = target.poll_interval.unwrap_or(cfg.poll_interval);
            let timeout = target.poll_timeout.unwrap_or(cfg.poll_timeout);
            let handle = tokio::spawn(async move {
                me.run_target(target, interval, timeout).await;
            });
            handles.push(handle);
        }

        // Block on signals. Unix handles SIGTERM/SIGINT; Windows uses Ctrl-C.
        #[cfg(unix)]
        {
            let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
            tokio::select! {
                _ = sigterm.recv() => { info!("SIGTERM, exiting"); }
                _ = sigint.recv()  => { info!("SIGINT, exiting"); }
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await?;
            info!("Ctrl-C received, exiting");
        }

        // Cancel target loops so shutdown does not wait on their infinite tickers.
        for handle in &handles {
            handle.abort();
        }
        for handle in handles {
            let _ = handle.await;
        }
        Ok(())
    }

    /// One target's poll loop. Spawned as its own tokio task.
    async fn run_target(&self, target: Target, interval: Duration, timeout: Duration) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        info!(target = %target.name, url = %target.url, interval_secs = interval.as_secs(), "target poll loop starting");
        loop {
            ticker.tick().await;
            match self.poll_once_target(&target, timeout).await {
                Ok(outcome) => debug!(?outcome, target = %target.name, "poll ok"),
                Err(err) => warn!(target = %target.name, error = %err, "poll failed"),
            }
        }
    }

    /// Poll one specific target once.
    ///
    /// The target URL is used as-is if it contains a path; otherwise `/health`
    /// is appended. This matches the slice-1 convention so the wiremock
    /// fixtures (which mount on `/health`) keep working unchanged.
    pub async fn poll_once_target(&self, target: &Target, timeout: Duration) -> Result<PollOutcome, PollError> {
        let started = Instant::now();
        let url = match target.url.find("://") {
            Some(idx) if target.url[idx + 3..].contains('/') => target.url.clone(),
            _ => format!("{}/health", target.url.trim_end_matches('/')),
        };
        let res = self.inner.http.get(&url).timeout(timeout).send().await;
        let latency = started.elapsed();
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

        let sample = match res {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let outcome = if resp.status().is_success() { Outcome::Ok } else { Outcome::Error };
                Sample {
                    provider: target.name.clone(),
                    outcome,
                    latency,
                    status_code: status,
                    timestamp_secs: ts,
                }
            }
            Err(e) => {
                error!(target = %target.name, error = %e, "transport error");
                Sample {
                    provider: target.name.clone(),
                    outcome: Outcome::Error,
                    latency,
                    status_code: 0,
                    timestamp_secs: ts,
                }
            }
        };

        let mut m = self.inner.metrics.lock().await;
        m.record_sample(&sample);

        // Update per-target ring buffers + compute burn against each SLO.
        let mut c = self.inner.counters.lock().await;
        let tc = c.get_mut(&target.name).ok_or_else(|| {
            PollError::InvalidConfig(format!("target {} not initialised", target.name))
        })?;
        let is_success = sample.outcome == Outcome::Ok;
        tc.short.record(is_success, ts);
        tc.long.record(is_success, ts);

        let short_window = BurnWindow::FAST_BURN.long.as_secs();
        let long_window = tc.long.bucket_size_secs().max(1) * tc.long.len() as u64;
        let (s_short, f_short) = tc.short.window(short_window, ts);
        let (s_long, f_long) = tc.long.window(long_window, ts);

        let mut burn_short = 0.0_f64;
        let mut burn_long = 0.0_f64;
        for slo in &self.inner.config.slos {
            let bs = burn_rate(s_short, f_short, slo.target);
            let bl = burn_rate(s_long, f_long, slo.target);
            m.record_burn(&format!("{}::{}", target.name, slo.name), BurnWindow::FAST_BURN, bs);
            m.record_burn(&format!("{}::{}", target.name, slo.name), BurnWindow::SLOW_BURN, bl);
            burn_short = bs;
            burn_long = bl;
        }

        // Release shared state before alert evaluation and webhook I/O.
        drop(c);
        drop(m);

        let payloads = self.evaluate_alerts(&target.name, ts).await;
        Ok(PollOutcome { sample, burn_short, burn_long, alert_payloads: payloads })
    }

    /// Evaluate every alert rule against its configured SLO/window. Returns
    /// payloads that fired (already delivered via webhooks).
    async fn evaluate_alerts(&self, target_name: &str, ts: u64) -> Vec<alerts::AlertPayload> {
        let mut fired = Vec::new();
        for rule in &self.inner.config.alert_rules {
            let slo_target = self.inner.config.slos.iter()
                .find(|slo| slo.name == rule.slo)
                .map(|slo| slo.target)
                .unwrap_or(0.999);
            let window_secs = rule.window
                .unwrap_or(BurnWindow::FAST_BURN.long)
                .as_secs();

            // Keep the counters lock scoped to the ring query. Never hold it
            // while waiting for webhook transport.
            let burn = {
                let counters = self.inner.counters.lock().await;
                let Some(tc) = counters.get(target_name) else { continue };
                let (success, failure) = tc.long.window(window_secs, ts);
                burn_rate(success, failure, slo_target)
            };

            let decision = {
                let mut trackers = self.inner.alert_trackers.lock().await;
                let key = format!("{}::{}", target_name, rule.name);
                let tracker = trackers.entry(key).or_insert_with(AlertStateTracker::default);
                alerts::evaluate(rule, target_name, burn, ts, tracker)
            };
            if let Decision::Fire(payload) = decision {
                let reports = webhook::deliver_all(&self.inner.http, &rule.webhooks, &payload).await;
                let mut last = self.inner.last_delivery.lock().await;
                for report in reports {
                    last.insert(report.url.clone(), report);
                }
                fired.push(payload);
            }
        }
        fired
    }

    /// Backward-compat helper: poll the first target once.
    pub async fn poll_once(&self) -> Result<PollOutcome, PollError> {
        let target = self.inner.config.targets.first()
            .ok_or(PollError::NoTargets)?
            .clone();
        self.poll_once_target(&target, self.inner.config.poll_timeout).await
    }

    pub fn windows(&self) -> &'static [BurnWindow] {
        &[BurnWindow::FAST_BURN, BurnWindow::SLOW_BURN]
    }
}

impl SLO {
    pub fn with_window_secs(mut self, secs: u64) -> Self { self.window_secs = secs; self }
    pub fn with_target(mut self, target: f64) -> Self { self.target = target; self }
}
