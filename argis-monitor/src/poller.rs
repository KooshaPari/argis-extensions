//! Async poller: drives one tokio task per target, accumulates samples into
//! the shared `Metrics` registry, and feeds the `RingBuffer` per target for
//! proper SLO multi-window burn-rate computation.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use prometheus_client::registry::Registry;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{watch, Mutex};
use tracing::{debug, error, info, warn};


use crate::alerts::{self, AlertRule, AlertStateTracker, Decision};
use crate::config::{Config, SLO};
use crate::metrics::{Metrics, Outcome, Sample};
use crate::ring_buffer::RingBuffer;
use crate::slo::{burn_rate, BurnWindow};
use crate::target::Target;
use crate::state_store::{StateStore, TrackerSnapshot};
use crate::webhook;

/// One poll's outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PollOutcome {
    pub sample: Sample,
    pub burn_short: f64,
    pub burn_long: f64,
    /// Burn rates keyed by SLO name, retained for multi-SLO callers.
    #[serde(default)]
    pub burn_by_slo: HashMap<String, (f64, f64)>,
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
    /// Optional SQLite state store. When present, every alert state transition
    /// is persisted so the monitor can rehydrate after a restart.
    pub state_store: Mutex<Option<StateStore>>,
}

impl Monitor {
    /// Build a new monitor from `config`. Errors if `config.targets` is empty.
    pub fn new(config: Config) -> Result<Self, PollError> {
        if config.targets.is_empty() {
            return Err(PollError::NoTargets);
        }
        let mut target_names = HashSet::new();
        for target in &config.targets {
            if !target_names.insert(target.name.clone()) {
                return Err(PollError::InvalidConfig(format!(
                    "duplicate target name: {}", target.name
                )));
            }
        }
        for slo in &config.slos {
            if !slo.target.is_finite() || !(0.0..=1.0).contains(&slo.target) {
                return Err(PollError::InvalidConfig(format!(
                    "SLO {} target must be finite and within [0,1]", slo.name
                )));
            }
            if slo.window_secs == 0 {
                return Err(PollError::InvalidConfig(format!(
                    "SLO {} window_secs must be > 0", slo.name
                )));
            }
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
        for target in config.targets.iter().skip(1) {
            metrics.record_target_info(&target.url);
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

        // Open the state store (if configured) and rehydrate trackers.
        let state_store = match &config.data_dir {
            Some(dir) => {
                let path = dir.join("alert_state.sqlite");
                match StateStore::open(&path) {
                    Ok(mut s) => {
                        match s.load_all() {
                            Ok(restored) => {
                                for (key, snap) in restored {
                                    if let Some(slot) = alert_trackers.get_mut(&key) {
                                        slot.state = snap.state;
                                        slot.sustained_for = std::time::Duration::from_secs(snap.sustained_secs);
                                    }
                                }
                                tracing::info!(path = %path.display(), "state store loaded");
                            }
                            Err(e) => tracing::warn!(error = %e, "failed to load state store; starting fresh"),
                        }
                        Some(s)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, path = %path.display(), "failed to open state store; alerts will be in-memory only");
                        None
                    }
                }
            }
            None => None,
        };

        Ok(Self {
            inner: Arc::new(MonitorInner {
                config,
                http,
                registry: Arc::new(registry),
                metrics: Arc::new(Mutex::new(metrics)),
                counters: Mutex::new(counters),
                alert_trackers: Mutex::new(alert_trackers),
                last_delivery: Mutex::new(HashMap::new()),
                state_store: Mutex::new(state_store),
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

        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Spawn one task per target.
        let mut handles = Vec::with_capacity(cfg.targets.len());
        for target in cfg.targets.clone() {
            let me = self.clone();
            let interval = target.poll_interval.unwrap_or(cfg.poll_interval);
            let timeout = target.poll_timeout.unwrap_or(cfg.poll_timeout);
            let shutdown = shutdown_rx.clone();
            let handle = tokio::spawn(async move {
                me.run_target(target, interval, timeout, shutdown).await;
            });
            handles.push(handle);
        }

        // Block on signals, then tell every polling loop to stop.
        tokio::select! {
            _ = sigterm.recv() => { info!("SIGTERM, exiting"); }
            _ = sigint.recv()  => { info!("SIGINT, exiting");  }
        }
        let _ = shutdown_tx.send(true);
        for h in handles { let _ = h.await; }
        Ok(())
    }

    /// One target's poll loop. Spawned as its own tokio task.
    async fn run_target(&self, target: Target, interval: Duration, timeout: Duration, mut shutdown: watch::Receiver<bool>) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        info!(target = %target.name, url = %target.url, interval_secs = interval.as_secs(), "target poll loop starting");
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match self.poll_once_target(&target, timeout).await {
                        Ok(outcome) => debug!(?outcome, target = %target.name, "poll ok"),
                        Err(err) => warn!(target = %target.name, error = %err, "poll failed"),
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
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
        let (s_short, f_short) = tc.short.window(short_window, ts);
        let mut burn_by_slo = HashMap::new();
        let mut burn_short = 0.0_f64;
        let mut burn_long = 0.0_f64;
        for slo in &self.inner.config.slos {
            let bs = burn_rate(s_short, f_short, slo.target);
            let (s_long, f_long) = tc.long.window(slo.window_secs.max(1), ts);
            let bl = burn_rate(s_long, f_long, slo.target);
            m.record_burn(&format!("{}::{}", target.name, slo.name), BurnWindow::FAST_BURN, bs);
            m.record_burn(&format!("{}::{}", target.name, slo.name), BurnWindow::SLOW_BURN, bl);
            burn_by_slo.insert(slo.name.clone(), (bs, bl));
            burn_short = bs;
            burn_long = bl;
        }
        drop(c);
        drop(m);

        // Evaluate alert rules using the rates for each rule's SLO.
        let payloads = self.evaluate_alerts(&target.name, &burn_by_slo, ts).await;
        Ok(PollOutcome { sample, burn_short, burn_long, burn_by_slo, alert_payloads: payloads })
    }

    /// Evaluate every alert rule against the latest burn rates. Returns the
    /// list of payloads that fired (already delivered via webhooks).
    async fn evaluate_alerts(&self, target_name: &str, burns: &HashMap<String, (f64, f64)>, ts: u64) -> Vec<alerts::AlertPayload> {
        let mut fired = Vec::new();
        for rule in &self.inner.config.alert_rules {
            let (burn_short, burn_long) = burns.get(&rule.slo).copied().unwrap_or((0.0, 0.0));
            let burn = match rule.window {
                Some(w) if w >= crate::slo::BurnWindow::SLOW_BURN.long => burn_long,
                _ => burn_short,
            };
            let key = format!("{}::{}", target_name, rule.name);

            // Advance the state machine while holding only the tracker lock.
            // Webhook delivery must not block other target evaluations.
            let (decision, snap) = {
                let mut trackers = self.inner.alert_trackers.lock().await;
                let tracker = trackers.entry(key.clone()).or_insert_with(AlertStateTracker::default);
                let decision = alerts::evaluate(rule, target_name, burn, ts, tracker);
                let snap = TrackerSnapshot {
                    state: tracker.state.clone(),
                    sustained_secs: tracker.sustained_for.as_secs(),
                };
                (decision, snap)
            };

            // Persist immediately after advancing the tracker. This
            // prevents a slow webhook from allowing a stale snapshot to
            // overwrite a newer state from another target task.
            {
                let mut store = self.inner.state_store.lock().await;
                if let Some(s) = store.as_mut() {
                    if let Err(e) = s.save(&key, &snap) {
                        tracing::warn!(target = %target_name, rule = %rule.name, error = %e, "state store save failed");
                    }
                }
            }

            if let Decision::Fire(payload) = decision {
                let reports = webhook::deliver_all(&self.inner.http, &rule.webhooks, &payload).await;
                let mut last = self.inner.last_delivery.lock().await;
                for r in reports {
                    last.insert(r.url.clone(), r);
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
