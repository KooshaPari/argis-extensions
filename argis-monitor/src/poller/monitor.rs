//! Monitor struct + `new()` + `run()` + `run_target()`.
//!
//! The HTTP-driven poll loop lives in `poll_loop::poll_once_target`.
//! The alert + meta-alert evaluators live in `evaluate_alerts` and
//! `evaluate_meta_alerts`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow;
use arc_swap::ArcSwap;
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
///
/// All mutable shared state lives behind an `ArcSwap<MonitorInner>` so a
/// SIGHUP-driven `reload_from_path` can atomically swap the entire inner
/// without touching poll tasks. Reads acquire a cheap `Guard<Arc<…>>` and
/// always see the latest config; writes are `O(1)` lock-free stores.
/// `ArcSwap` itself is `Clone` (it's an Arc internally) so `Monitor::clone`
/// stays a single cheap Arc-bump.
pub struct Monitor {
    pub(crate) inner: ArcSwap<MonitorInner>,
}

impl Clone for Monitor {
    fn clone(&self) -> Self {
        // `arc_swap::ArcSwap::clone(&self.inner)` only exists via `ArcSwapAny`
        // internals, so we go through `load().clone()` (returns `Arc<MonitorInner>`,
        // which is Clone) and rebuild the ArcSwap wrapper.
        Self { inner: arc_swap::ArcSwap::from(self.inner.load().clone()) }
    }
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

        let inner = MonitorInner {
            config,
            http,
            registry: Arc::new(registry),
            metrics: Arc::new(Mutex::new(metrics)),
            counters: Mutex::new(counters),
            alert_trackers: Mutex::new(alert_trackers),
            last_delivery: Mutex::new(HashMap::new()),
            state_store: Mutex::new(state_store),
        };
        Ok(Self {
            inner: ArcSwap::from_pointee(inner),
        })
    }

    pub fn registry(&self) -> Arc<Registry> { self.inner.load().registry.clone() }
    pub fn config(&self) -> Config { self.inner.load().config.clone() }

    /// Hot-reload the monitor from a YAML config file on disk. Builds a fresh
    /// `MonitorInner` from the file and atomically swaps it into place via the
    /// `ArcSwap`. After this returns, every subsequent `load()` (in
    /// `poll_once_target`, `evaluate_meta_alerts`, etc.) sees the new config.
    ///
    /// Existing tokio Mutexes (`alert_trackers`, `metrics`, `counters`,
    /// `state_store`, `last_delivery`) are rebuilt fresh — which means
    /// in-flight rule state machines reset. That is acceptable for a config
    /// change; the alternative (reconciling in place) would require complex
    /// diff logic.
    pub async fn reload_from_path(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let raw = std::fs::read_to_string(path)?;
        let new_config: Config = serde_yaml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("invalid yaml in {}: {e}", path.display()))?;
        // Reuse Monitor::new to build the fresh inner. It validates target list,
        // creates a fresh HTTP client (clone of the original headers), recreates
        // ring buffers + alert trackers, and rehydrates the state store.
        let probe = Monitor::new(new_config.clone())?;
        // Atomically swap. The old inner is dropped here; any tokio Mutex guards
        // held by in-flight poll tasks will see the new state on next access.
        self.inner.store(probe.inner.load_full());
        tracing::info!(
            path = %path.display(),
            targets = new_config.targets.len(),
            rules = new_config.alert_rules.len(),
            meta_alerts = new_config.meta_alerts.len(),
            "Monitor::reload_from_path swap complete"
        );
        Ok(())
    }

    /// Run all configured targets in parallel. Blocks until SIGINT/SIGTERM.
    pub async fn run(&self) -> anyhow::Result<()> {
        let cfg = self.inner.load().config.clone();
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
