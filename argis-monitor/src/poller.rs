//! Async poller: drives one tokio task per target, accumulates samples into
//! the shared `Metrics` registry, and feeds the `RingBuffer` per target for
//! proper SLO multi-window burn-rate computation.

use std::collections::{BTreeMap, HashMap};
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
use crate::push;
use crate::state_store::{StateStore, TrackerSnapshot};
use crate::suppression;
use crate::webhook;

/// One poll's outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PollOutcome {
    pub sample: Sample,
    /// Burn rates for every configured SLO, keyed by its stable name.
    ///
    /// The scalar fields below remain as compatibility aliases for the first
    /// configured SLO; callers that track more than one SLO must use this map.
    #[serde(default)]
    pub burn_rates: BTreeMap<String, BurnRates>,
    pub burn_short: f64,
    pub burn_long: f64,
    #[serde(default)]
    pub alert_payloads: Vec<crate::alerts::AlertPayload>,
}

/// Short- and long-window burn rates for one SLO.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct BurnRates {
    pub short: f64,
    pub long: f64,
}

fn compute_burn_rates(
    slos: &[SLO],
    (short_success, short_failure): (u64, u64),
    long: &RingBuffer,
    ts: u64,
) -> BTreeMap<String, BurnRates> {
    slos.iter()
        .map(|slo| {
            let (long_success, long_failure) = long.window(slo.window_secs, ts);
            (
                slo.name.clone(),
                BurnRates {
                    short: burn_rate(short_success, short_failure, slo.target),
                    long: burn_rate(long_success, long_failure, slo.target),
                },
            )
        })
        .collect()
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
        config.validate().map_err(PollError::InvalidConfig)?;
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
        let metrics = Metrics::new(
            &mut registry,
            config
                .targets
                .iter()
                .map(|target| (target.name.clone(), target.url.clone())),
        );
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

        // Spawn one task per target.
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut handles = Vec::with_capacity(cfg.targets.len());
        for target in cfg.targets.clone() {
            let me = self.clone();
            let shutdown = shutdown_rx.clone();
            let interval = target.poll_interval.unwrap_or(cfg.poll_interval);
            let timeout = target.poll_timeout.unwrap_or(cfg.poll_timeout);
            let handle = tokio::spawn(async move {
                me.run_target(target, interval, timeout, shutdown).await;
            });
            handles.push(handle);
        }

        // Block on signals.
        #[cfg(unix)]
        {
            let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
            tokio::select! {
                _ = sigterm.recv() => { info!("SIGTERM, exiting"); }
                _ = sigint.recv()  => { info!("SIGINT, exiting");  }
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await?;
            info!("CTRL-C, exiting");
        }
        let _ = shutdown_tx.send(true);
        for h in handles {
            let _ = tokio::time::timeout(Duration::from_secs(5), h).await;
        }
        Ok(())
    }

    /// One target's poll loop. Spawned as its own tokio task.
    async fn run_target(
        &self,
        target: Target,
        interval: Duration,
        timeout: Duration,
        mut shutdown: watch::Receiver<bool>,
    ) {
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
                        info!(target = %target.name, "target poll loop stopping");
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
        let burn_rates = compute_burn_rates(
            &self.inner.config.slos,
            (s_short, f_short),
            &tc.long,
            ts,
        );
        drop(c);

        let (burn_short, burn_long) = self.inner.config.slos.first()
            .and_then(|slo| burn_rates.get(&slo.name).map(|rates| (rates.short, rates.long)))
            .unwrap_or((0.0, 0.0));
        for (slo_name, rates) in &burn_rates {
            m.record_burn(&format!("{}::{}", target.name, slo_name), BurnWindow::FAST_BURN, rates.short);
            m.record_burn(&format!("{}::{}", target.name, slo_name), BurnWindow::SLOW_BURN, rates.long);
        }
        drop(m);

        // Evaluate alert rules (separately so the metrics lock is released).
        let payloads = self.evaluate_alerts(&target.name, &burn_rates, ts).await;
        Ok(PollOutcome { sample, burn_rates, burn_short, burn_long, alert_payloads: payloads })
    }

    /// Evaluate every alert rule against the latest burn rates. Returns the
    /// list of payloads that fired (already delivered via webhooks).
    async fn evaluate_alerts(&self, target_name: &str, burn_rates: &BTreeMap<String, BurnRates>, ts: u64) -> Vec<alerts::AlertPayload> {
        let mut fired = Vec::new();
        for rule in &self.inner.config.alert_rules {
            let Some(rates) = burn_rates.get(&rule.slo) else {
                warn!(target = %target_name, rule = %rule.name, slo = %rule.slo, "alert rule references an unconfigured SLO");
                continue;
            };
            let burn = match rule.window {
                Some(w) if w >= crate::slo::BurnWindow::SLOW_BURN.long => rates.long,
                _ => rates.short,
            };
            let key = format!("{}::{}", target_name, rule.name);
            let snap;
            let mut fired_payload = None;
            {
                let mut trackers = self.inner.alert_trackers.lock().await;
                let tracker = trackers.entry(key.clone()).or_insert_with(AlertStateTracker::default);
                match alerts::evaluate(rule, target_name, burn, ts, tracker) {
                    Decision::Fire(payload) => {
                        // Suppression check. A matching window swallows the
                        // webhook delivery but the state machine still
                        // transitions (so the alert would have fired is
                        // visible in metrics + the alert_history table).
                        let window_name = suppression::is_suppressed(
                            &self.inner.config.alert_windows,
                            target_name,
                            &rule.name,
                            ts,
                        );
                        if let Some(wname) = &window_name {
                            tracing::info!(
                                target = %target_name,
                                rule = %rule.name,
                                window = %wname,
                                burn = burn,
                                "alert suppressed by window"
                            );
                        } else {
                            let reports = webhook::deliver_all(&self.inner.http, &rule.webhooks, &payload).await;
                            let mut last = self.inner.last_delivery.lock().await;
                            for r in reports {
                                last.insert(r.url.clone(), r);
                            }
                        }
                        fired_payload = Some(payload.clone());
                        fired.push(payload);
                    }
                    Decision::None => {}
                }
                snap = TrackerSnapshot {
                    state: tracker.state.clone(),
                    sustained_secs: tracker.sustained_for.as_secs(),
                };
            }
            // Capture fired-meta for the alert_history insert below.
            let fired_meta = fired_payload.as_ref().map(|p| (p.rule.clone(), p.severity, p.burn_rate, p.threshold, p.fired_at_unix));
            // Persist outside the trackers lock to avoid contention.
            let mut store = self.inner.state_store.lock().await;
            if let Some(s) = store.as_mut() {
                if let Err(e) = s.save(&key, &snap) {
                    tracing::warn!(target = %target_name, rule = %rule.name, error = %e, "state store save failed");
                }
                if let Some((rule_name, severity, burn, threshold, ts)) = &fired_meta {
                    let payload_json = serde_json::to_string(&serde_json::json!({
                        "rule": rule_name,
                        "target": target_name,
                        "slo": &rule.slo,
                        "burn_rate": burn,
                        "threshold": threshold,
                        "severity": format!("{:?}", severity).to_lowercase(),
                        "fired_at_unix": ts,
                    })).unwrap_or_default();
                    let event = match snap.state {
                        crate::alerts::AlertState::Ok => "resolved",
                        _ => "fired",
                    };
                    let severity_str = format!("{:?}", severity).to_lowercase();
                    if let Err(e) = s.record_event(
                        &key, event, &severity_str,
                        *burn, *threshold, &payload_json, *ts,
                    ) {
                        tracing::warn!(target = %target_name, rule = %rule_name, error = %e, "alert history record failed");
                    }
                }
            }
            drop(store);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::AlertRule;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn burn_rates_use_each_slos_configured_window() {
        let mut ring = RingBuffer::with_bucket_size(3_600, 60, 0);
        ring.record(false, 0);
        ring.record(true, 300);
        let slos = vec![
            SLO { name: "short".into(), window_secs: 60, target: 0.5 },
            SLO { name: "long".into(), window_secs: 600, target: 0.5 },
        ];

        let rates = compute_burn_rates(&slos, (1, 0), &ring, 300);
        assert_eq!(rates["short"].long, 0.0, "the short SLO must exclude the old failure");
        assert_eq!(rates["long"].long, 1.0, "the long SLO must include both buckets");
    }

    #[tokio::test]
    async fn alert_history_uses_current_rule_and_slo_payload() {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "argis-monitor-poller-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::SeqCst),
        ));
        let mut config = Config::for_test("http://127.0.0.1:1");
        config.slos = vec![
            SLO { name: "first".into(), window_secs: 60, target: 0.999 },
            SLO { name: "second".into(), window_secs: 60, target: 0.999 },
        ];
        config.data_dir = Some(dir.clone());
        config.alert_rules = vec![
            AlertRule { name: "first-rule".into(), slo: "first".into(), threshold: 2.0, ..Default::default() },
            AlertRule { name: "second-rule".into(), slo: "second".into(), threshold: 2.0, ..Default::default() },
        ];
        let monitor = Monitor::new(config).expect("valid test monitor");
        let rates = BTreeMap::from([
            ("first".into(), BurnRates { short: 3.0, long: 3.0 }),
            ("second".into(), BurnRates { short: 0.0, long: 0.0 }),
        ]);

        // First tick enters Pending; the second tick fires only first-rule.
        monitor.evaluate_alerts("gateway", &rates, 100).await;
        let fired = monitor.evaluate_alerts("gateway", &rates, 101).await;
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].rule, "first-rule");
        assert_eq!(fired[0].slo, "first");

        drop(monitor);
        let store = StateStore::open(&dir.join("alert_state.sqlite")).expect("open test state store");
        let history = store.list_history(None, 10).expect("read test alert history");
        assert_eq!(history.len(), 1, "a non-firing rule must not reuse a prior payload");
        assert_eq!(history[0].key, "gateway::first-rule");
        assert!(history[0].payload_json.contains("first-rule"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
