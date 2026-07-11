//! Monitor struct + `new()` + `run()` + `run_target()`.
//!
//! The HTTP-driven poll loop lives in `poll_loop::poll_once_target`.
//! The alert + meta-alert evaluators live in `evaluate_alerts` and
//! `evaluate_meta_alerts`.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow;
use prometheus_client::registry::Registry;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::alerts::AlertStateTracker;
use crate::config::Config;
use crate::metrics::Metrics;
use crate::ring_buffer::RingBuffer;
use crate::slo::BurnWindow;
use crate::state_store::StateStore;
use crate::target::Target;

use super::poll_loop::poll_once_target_impl;
use super::types::{MonitorInner, PollError, TargetCounters};

/// The monitor: shared registry + metrics + per-target ring buffers + HTTP client.
#[derive(Clone)]
pub struct Monitor {
    pub(crate) inner: std::sync::Arc<MonitorInner>,
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
            inner: std::sync::Arc::new(MonitorInner {
                config,
                http,
                registry: std::sync::Arc::new(registry),
                metrics: std::sync::Arc::new(Mutex::new(metrics)),
                counters: Mutex::new(counters),
                alert_trackers: Mutex::new(alert_trackers),
                last_delivery: Mutex::new(HashMap::new()),
                state_store: Mutex::new(state_store),
            }),
        })
    }

    pub fn registry(&self) -> std::sync::Arc<Registry> { self.inner.registry.clone() }
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

        // Block on signals.
        tokio::select! {
            _ = sigterm.recv() => { info!("SIGTERM, exiting"); }
            _ = sigint.recv()  => { info!("SIGINT, exiting");  }
        }
        for h in handles { let _ = h.await; }
        Ok(())
    }

    /// One target's poll loop. Spawned as its own tokio task.
    pub(crate) async fn run_target(&self, target: Target, interval: Duration, timeout: Duration) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        info!(target = %target.name, url = %target.url, interval_secs = interval.as_secs(), "target poll loop starting");
        loop {
            ticker.tick().await;
            match poll_once_target_impl(self, &target, timeout).await {
                Ok(outcome) => tracing::debug!(?outcome, target = %target.name, "poll ok"),
                Err(err) => tracing::warn!(target = %target.name, error = %err, "poll failed"),
            }
        }
    }
}
