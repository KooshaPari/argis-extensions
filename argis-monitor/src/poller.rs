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
use crate::push;
use crate::state_store::{StateStore, TrackerSnapshot};
use crate::suppression;
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
        drop(m);

        // Evaluate alert rules (separately so the metrics lock is released).
        let payloads = self.evaluate_alerts(&target.name, burn_short, burn_long, ts).await;
        // Meta-alerts run after alert evaluation so the alert_failures rows
        // recorded above (for failed webhook deliveries) are visible to the
        // next read. Result is currently only used for structured logging;
        // future slice will wire delivery of meta-alert payloads.
        let meta_fired = self.evaluate_meta_alerts(ts).await;
        for name in &meta_fired {
            tracing::info!(
                target = %target.name,
                meta_alert = %name,
                "meta-alert fired during poll"
            );
        }
        Ok(PollOutcome { sample, burn_short, burn_long, alert_payloads: payloads })
    }

    /// Evaluate every meta-alert rule and deliver the resulting payloads.
    /// Returns the names of the meta-alerts that fired in this tick.
    ///
    /// A meta-alert fires when the alert_failures table holds at least
    /// `consecutive_failures` rows for the target (and optional specific
    /// rule) within the trailing `window` seconds. This is the "Bifrost-
    /// backed" piece: persistent failure history that survives restarts.
    ///
    /// Delivery: each fired meta-alert is wrapped in an `AlertPayload` and
    /// POSTed via `webhook::deliver_all`. Webhook targets are taken from
    /// `MetaAlertRule.webhooks` when non-empty; otherwise the matching
    /// `AlertRule.webhooks` for the same target is used as the fallback so
    /// operators don't have to configure meta-alert webhooks twice.
    #[tracing::instrument(skip(self))]
    pub async fn evaluate_meta_alerts(&self, ts: u64) -> Vec<String> {
        let rules = self.inner.config.meta_alerts.clone();
        if rules.is_empty() { return Vec::new(); }
        let mut store_guard = self.inner.state_store.lock().await;
        let store = match store_guard.as_mut() {
            Some(s) => s,
            None => {
                tracing::debug!("meta-alert evaluation skipped: no state store configured");
                return Vec::new();
            }
        };
        let mut fired = Vec::new();
        for rule in &rules {
            // The state_store key is "{target}::{rule_name}" for per-rule
            // counts. When the meta-alert specifies a `rule`, scope to it.
            let key = match &rule.rule {
                Some(r) => format!("{}::{}", rule.target, r),
                None => format!("{}::*", rule.target),
            };
            let count = match store.count_failures_in_window(&key, rule.window.as_secs(), ts) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(meta = %rule.name, error = %e, "alert_failures count failed");
                    continue;
                }
            };
            if count >= u64::from(rule.consecutive_failures) {
                tracing::info!(
                    meta = %rule.name,
                    target = %rule.target,
                    count = count,
                    threshold = rule.consecutive_failures,
                    window_secs = rule.window.as_secs(),
                    severity = ?rule.severity,
                    "meta-alert fired"
                );

                // Build the payload. The AlertPayload struct already carries
                // the right fields for a meta-alert: we put the meta-alert
                // name in `rule`, the target in `target`, the optional
                // reason in `slo` (it's just a free-form string), and the
                // observed count + threshold into the burn_rate/threshold
                // numeric slots so downstream consumers see real numbers.
                let payload = alerts::AlertPayload::meta_alert(
                    rule.name.clone(),
                    rule.target.clone(),
                    rule.reason.clone(),
                    count as f64,
                    rule.consecutive_failures as f64,
                    rule.severity,
                    ts,
                );

                // Resolve webhooks: prefer meta-rule's own, fall back to a
                // matching AlertRule's webhooks for the same target.
                let webhook_targets: Vec<alerts::WebhookTarget> = if !rule.webhooks.is_empty() {
                    rule.webhooks.clone()
                } else {
                    self.inner.config.alert_rules.iter()
                        .find(|ar| {
                            ar.name == rule.rule.clone().unwrap_or_default()
                                && self.inner.config.targets.iter().any(|t| t.name == ar.slo || t.name == rule.target)
                        })
                        .map(|ar| ar.webhooks.clone())
                        .unwrap_or_default()
                };

                if webhook_targets.is_empty() {
                    tracing::warn!(
                        meta = %rule.name,
                        target = %rule.target,
                        "meta-alert fired but no webhook targets configured"
                    );
                } else {
                    let reports = webhook::deliver_all(
                        &self.inner.http, &webhook_targets, &payload,
                    ).await;
                    let mut last = self.inner.last_delivery.lock().await;
                    for r in reports {
                        last.insert(r.url.clone(), r);
                    }
                }

                fired.push(rule.name.clone());
            }
        }
        fired
    }

    /// Evaluate every alert rule against the latest burn rates. Returns the
    /// list of payloads that fired (already delivered via webhooks).
    async fn evaluate_alerts(&self, target_name: &str, burn_short: f64, burn_long: f64, ts: u64) -> Vec<alerts::AlertPayload> {
        let mut fired = Vec::new();
        let mut store = self.inner.state_store.lock().await;
        for rule in &self.inner.config.alert_rules {
            let burn = match rule.window {
                Some(w) if w >= crate::slo::BurnWindow::SLOW_BURN.long => burn_long,
                _ => burn_short,
            };
            let key = format!("{}::{}", target_name, rule.name);
            let snap;
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
                                if !r.success {
                                    // Seed the meta-alert pipeline. The key
                                    // is "{target}::{rule.name}" so the meta-
                                    // alert layer can scope by rule when it
                                    // wants to (or use a "{target}::*" key
                                    // for the unscoped variant). We rely on
                                    // the persisted row rather than an in-
                                    // memory counter so failures survive a
                                    // monitor restart.
                                    if let Some(s) = store.as_mut() {
                                        let msg = r.error.as_deref().unwrap_or("webhook delivery failed");
                                        if let Err(e) = s.record_alert_failure(&key, ts, msg) {
                                            tracing::warn!(target = %target_name, rule = %rule.name, error = %e, "alert failure record failed");
                                        }
                                    }
                                }
                                last.insert(r.url.clone(), r);
                            }
                        }
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
            let fired_meta = fired.last().map(|p| (p.rule.clone(), p.severity, p.burn_rate, p.threshold, p.fired_at_unix));
            // Persist outside the trackers lock to avoid contention.
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
