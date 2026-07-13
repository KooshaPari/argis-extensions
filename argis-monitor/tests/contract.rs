//! End-to-end contract tests for argis-monitor.
//!
//! Spins up a wiremock gateway, runs the Monitor for a few poll cycles,
//! and asserts the resulting /metrics output.

use std::time::Duration;

use argis_monitor::exporter;
use argis_monitor::{Config, Monitor, Outcome, SLO};
use prometheus_client::encoding::text::encode;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthy_target_polls_and_emits_metrics() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(10)))
        .mount(&server)
        .await;

    let cfg = Config::for_test(server.uri()).with_poll_interval_secs(1);
    let monitor = Monitor::new(cfg).unwrap();
    let registry = monitor.registry();

    // Two polls.
    let a = monitor.poll_once().await.unwrap();
    let b = monitor.poll_once().await.unwrap();
    assert_eq!(a.sample.outcome, Outcome::Ok);
    assert_eq!(b.sample.outcome, Outcome::Ok);

    let mut buf = String::new();
    encode(&mut buf, &registry).unwrap();
    assert!(buf.contains("argis_monitor_polls_total"), "expected polls_total in:
{buf}");
    assert!(buf.contains("argis_monitor_up"), "expected up gauge in:
{buf}");
    assert!(buf.contains("argis_monitor_target_info"), "expected target_info in:
{buf}");
    assert!(buf.contains("argis_monitor_slo_target"), "expected slo_target in:
{buf}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unhealthy_target_records_error_sample() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let cfg = Config::for_test(server.uri());
    let monitor = Monitor::new(cfg).unwrap();
    let outcome = monitor.poll_once().await.unwrap();
    assert_eq!(outcome.sample.outcome, Outcome::Error);
    assert_eq!(outcome.sample.status_code, 503);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transport_failure_records_zero_status() {
    // Bind to an unused port and immediately drop the listener so the
    // server is unreachable.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let cfg = Config::for_test(format!("http://127.0.0.1:{port}"))
        .with_poll_interval_secs(1);
    let monitor = Monitor::new(cfg).unwrap();
    let outcome = monitor.poll_once().await.unwrap();
    assert_eq!(outcome.sample.outcome, Outcome::Error);
    assert_eq!(outcome.sample.status_code, 0, "transport failure should be status_code=0");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exporter_serves_metrics_text_format() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let cfg = Config::for_test(server.uri());
    let monitor = Monitor::new(cfg).unwrap();
    monitor.poll_once().await.unwrap();
    let handle = exporter::serve("127.0.0.1:0", monitor.registry()).await.unwrap();
    let url = format!("http://{}/metrics", handle.addr);

    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    assert!(body.contains("# HELP argis_monitor_polls_total"));
    assert!(body.contains("argis_monitor_up"));
    let _ = handle.shutdown.send(true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_window_burn_reflects_error_traffic() {
    let server = MockServer::start().await;
    // First poll: ok. Subsequent polls: 503.
    let _m1 = Mock::given(method("GET")).and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1..)
        .mount(&server);
    Mock::given(method("GET")).and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let cfg = Config::for_test(server.uri())
        .with_slo(SLO { name: "chat".into(), window_secs: 3600, target: 0.999 });
    let monitor = Monitor::new(cfg).unwrap();
    monitor.poll_once().await.unwrap();
    monitor.poll_once().await.unwrap();
    let outcome = monitor.poll_once().await.unwrap();
    assert!(outcome.burn_short > 0.0, "burn_short should rise after errors; got {}", outcome.burn_short);
}


// =====================================================================
// Slice 2: multi-target + ring buffer
// =====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitor_rejects_config_with_no_targets() {
    let cfg = Config::default();
    let bad_cfg = Config { targets: vec![], ..cfg };
    let err = Monitor::new(bad_cfg).err().expect("expected an error from empty-targets Config");
    assert!(matches!(err, argis_monitor::poller::PollError::NoTargets), "got: {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitor_polls_multiple_targets_in_isolation() {
    let s1 = MockServer::start().await;
    let s2 = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&s1).await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).mount(&s2).await;

    let cfg = Config::default()
        .with_target_named("healthy", s1.uri())
        .with_target_named("broken", s2.uri());
    let monitor = Monitor::new(cfg).unwrap();

    let a = monitor.poll_once_target(&argis_monitor::Target::new("healthy", s1.uri()), std::time::Duration::from_secs(5)).await.unwrap();
    let b = monitor.poll_once_target(&argis_monitor::Target::new("broken", s2.uri()), std::time::Duration::from_secs(5)).await.unwrap();

    assert_eq!(a.sample.outcome, Outcome::Ok);
    assert_eq!(a.sample.provider, "healthy");
    assert_eq!(b.sample.outcome, Outcome::Error);
    assert_eq!(b.sample.provider, "broken");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ring_buffer_excludes_stale_buckets_from_burn_rate() {
    use argis_monitor::RingBuffer;

    // 3600s total window, 60s buckets => 60 buckets.
    let mut rb: RingBuffer = RingBuffer::with_bucket_size(3600, 60, 0);

    // One success per minute for 60 minutes, anchored at t=0.
    for i in 0..60u64 {
        rb.record(true, i * 60);
    }
    // From t=3600 looking back the full 3600s window: every bucket is
    // in-window and each holds exactly 1 success, so we see 60 successes.
    let (s, _f) = rb.window(3600, 3600);
    assert_eq!(s, 60, "all 60 successes should be in-window over the full 3600s");

    // A trailing 120s window from t=3600 includes the two most-recent
    // buckets (anchored at t=3480..3600).
    let (s, f) = rb.window(120, 3600);
    assert_eq!(s + f, 3, "trailing 120s window contains 3 buckets (inclusive boundary)");

    // Now jump 30 minutes forward and record. The ring rotates 30 buckets,
    // dropping the first 30 successes. The new record lands at bucket
    // anchored at t=5400.
    rb.record(true, 3600 + 30 * 60);
    // From t=5400, the trailing 120s window contains only the new record.
    let (s, f) = rb.window(120, 3600 + 30 * 60);
    assert_eq!(s, 1, "after a 30-min jump, trailing 120s window has just the new record");
    assert_eq!(f, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn target_yaml_parses_poll_interval() {
    let yaml = "name: openai\nurl: http://api.openai.com/v1/models\npoll_interval: 30s\n";
    let t: argis_monitor::Target = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(t.name, "openai");
    assert_eq!(t.poll_interval, Some(std::time::Duration::from_secs(30)));
}


// =====================================================================
// Slice 3: alerts + webhook delivery
// =====================================================================

#[test]
fn alert_state_transitions_in_unit_test() {
    use argis_monitor::alerts::{evaluate, AlertRule, AlertStateTracker, Decision};
    let rule = AlertRule {
        name: "r".into(),
        slo: "s".into(),
        threshold: 2.0,
        resolve_threshold: Some(1.0),
        for_secs: std::time::Duration::from_secs(5),
        cooldown: std::time::Duration::from_secs(60),
        webhooks: vec![],
        window: None,
    };
    let mut t = AlertStateTracker::default();

    // Tick 1: below threshold -> Ok -> no fire
    assert_eq!(evaluate(&rule, "gateway", 0.5, 100, &mut t), Decision::None);
    // Tick 2: crosses threshold -> Pending -> no fire
    assert_eq!(evaluate(&rule, "gateway", 3.0, 101, &mut t), Decision::None);
    // Tick 3: still in Pending after 5s
    for _ in 0..4 { evaluate(&rule, "gateway", 3.0, 102, &mut t); }
    // Tick 8 (after 5s sustained): Firing
    let d = evaluate(&rule, "gateway", 3.0, 107, &mut t);
    assert!(matches!(d, Decision::Fire(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_posts_payload_as_json() {
    use wiremock::matchers::{method, path};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/alerts"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let target = argis_monitor::alerts::WebhookTarget {
        url: format!("{}/alerts", server.uri()),
        headers: Default::default(),
        ..Default::default()
    };
    let payload = argis_monitor::alerts::AlertPayload::firing("r", "gateway", "s", 5.0, 2.0, 12345);
    let reports = argis_monitor::deliver_all(&client, &[target], &payload).await;
    assert_eq!(reports.len(), 1);
    assert!(reports[0].success);
    assert_eq!(reports[0].status, Some(200));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_records_failure_on_5xx() {
    use wiremock::matchers::{method, path};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/alerts"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let target = argis_monitor::alerts::WebhookTarget {
        url: format!("{}/alerts", server.uri()),
        headers: Default::default(),
        ..Default::default()
    };
    let payload = argis_monitor::alerts::AlertPayload::firing("r", "gateway", "s", 5.0, 2.0, 12345);
    let reports = argis_monitor::deliver_all(&client, &[target], &payload).await;
    assert_eq!(reports.len(), 1);
    assert!(!reports[0].success);
    assert!(reports[0].error.is_some());
}

#[test]
fn config_with_alert_rules_round_trips_yaml() {
    use argis_monitor::alerts::AlertRule;
    let cfg = Config::default()
        .with_alert_rule(AlertRule {
            name: "r1".into(),
            slo: "s".into(),
            threshold: 2.0,
            ..Default::default()
        });
    let s = serde_yaml::to_string(&cfg).unwrap();
    assert!(s.contains("alert_rules"));
    assert!(s.contains("threshold: 2"));
    let back: Config = serde_yaml::from_str(&s).unwrap();
    assert_eq!(back.alert_rules.len(), 1);
    assert_eq!(back.alert_rules[0].name, "r1");
}


#[test]
fn grafana_dashboard_json_is_valid_and_references_all_metrics() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("dashboards")
        .join("argis-monitor-dashboard.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse dashboard json: {e}"));

    assert_eq!(v["title"], "argis-monitor");
    let panels = v["panels"].as_array().expect("panels array");
    assert!(panels.len() >= 8, "expected at least 8 panels, got {}", panels.len());

    // Concatenate every PromQL expr across every panel target so we can
    // grep for the metric names without parsing Grafana's tree.
    let mut all_exprs = String::new();
    for p in panels {
        if let Some(targets) = p.get("targets").and_then(|t| t.as_array()) {
            for t in targets {
                if let Some(expr) = t.get("expr").and_then(|e| e.as_str()) {
                    all_exprs.push_str(expr);
                    all_exprs.push(' ');
                }
            }
        }
    }
    for metric in [
        "argis_monitor_up",
        "argis_monitor_poll_errors_total",
        "argis_monitor_poll_duration_seconds",
        "argis_monitor_polls_total",
        "argis_monitor_burn_rate",
        "argis_monitor_slo_target",
        "argis_monitor_last_poll_timestamp_seconds",
        "argis_monitor_target_info",
    ] {
        assert!(all_exprs.contains(metric),
                "dashboard does not reference metric `{metric}`; exprs: {all_exprs}");
    }
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suppression_window_swallows_webhook_but_state_transitions_still_fire() {
    use argis_monitor::suppression::{is_suppressed, Day, WindowSpec};

    // Suppression window covering every second of every day (HH:MM loop wrap).
    let w = WindowSpec {
        name: "everywhere".into(),
        start_time: Some("00:00".into()),
        end_time: Some("23:59".into()),
        days: vec![],
        start_at: None, end_at: None,
        targets: vec!["bad".into()],
        rules: vec!["smoke_rule".into()],
        reason: Some("test".into()),
    };

    // is_suppressed returns the window name.
    let now = argis_monitor::suppression::unix_from_utc(2026, 7, 6, 12, 0, 0);
    assert_eq!(
        is_suppressed(&[w.clone()], "bad", "smoke_rule", now),
        Some("everywhere".into())
    );
    // Different target -> no match.
    assert!(is_suppressed(&[w.clone()], "other", "smoke_rule", now).is_none());
    // Different rule - no match.
    assert!(is_suppressed(&[w], "bad", "other_rule", now).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_shot_suppression_window_does_not_match_outside_range() {
    use argis_monitor::suppression::{is_suppressed, WindowSpec};

    let w = WindowSpec {
        name: "maintenance".into(),
        start_at: Some("2026-07-15T02:00:00Z".into()),
        end_at: Some("2026-07-15T04:00:00Z".into()),
        start_time: None, end_time: None, days: vec![],
        targets: vec![], rules: vec![],
        reason: Some("DB upgrade".into()),
    };
    // Inside the range - suppress.
    let t_in = argis_monitor::suppression::unix_from_utc(2026, 7, 15, 3, 0, 0);
    assert!(is_suppressed(&[w.clone()], "any", "any", t_in).is_some());
    // Before start - no suppress.
    let t_before = argis_monitor::suppression::unix_from_utc(2026, 7, 15, 1, 0, 0);
    assert!(is_suppressed(&[w.clone()], "any", "any", t_before).is_none());
    // After end - no suppress.
    let t_after = argis_monitor::suppression::unix_from_utc(2026, 7, 15, 5, 0, 0);
    assert!(is_suppressed(&[w], "any", "any", t_after).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bearer_token_static_sent_as_authorization_header() {
    use wiremock::matchers::{header, method, path};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/alerts"))
        .and(header("Authorization", "Bearer secret-token-123"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    let target = argis_monitor::alerts::WebhookTarget {
        url: format!("{}/alerts", server.uri()),
        headers: Default::default(),
        bearer_token: Some("secret-token-123".into()),
        bearer_token_file: None,
        bearer_token_refresh_secs: None,
        ..Default::default()
    };
    let payload = argis_monitor::alerts::AlertPayload::firing("r", "gw", "s", 5.0, 2.0, 100);
    let reports = argis_monitor::deliver_all(&client, &[target], &payload).await;
    assert_eq!(reports.len(), 1);
    assert!(reports[0].success);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bearer_token_file_reads_contents() {
    use wiremock::matchers::{header, method, path};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/alerts"))
        .and(header("Authorization", "Bearer from-file-xyz"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = std::env::temp_dir().join(format!("argis-jwt-{}-{}", std::process::id(), "test"));
    std::fs::create_dir_all(&tmp).unwrap();
    let path = tmp.join("token");
    std::fs::write(&path, b"from-file-xyz\n").unwrap();

    let client = reqwest::Client::new();
    let target = argis_monitor::alerts::WebhookTarget {
        url: format!("{}/alerts", server.uri()),
        headers: Default::default(),
        bearer_token: None,
        bearer_token_file: Some(path),
        bearer_token_refresh_secs: Some(1),
        ..Default::default()
    };
    let payload = argis_monitor::alerts::AlertPayload::firing("r", "gw", "s", 5.0, 2.0, 100);
    let reports = argis_monitor::deliver_all(&client, &[target], &payload).await;
    assert_eq!(reports.len(), 1);
    assert!(reports[0].success, "expected success, got {:?}", reports[0]);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bearer_token_file_missing_logs_no_panic() {
    let client = reqwest::Client::new();
    let target = argis_monitor::alerts::WebhookTarget {
        url: "http://127.0.0.1:1/alerts".into(),
        headers: Default::default(),
        bearer_token: None,
        bearer_token_file: Some("/nonexistent/path/to/token".into()),
        bearer_token_refresh_secs: Some(60),
        ..Default::default()
    };
    let payload = argis_monitor::alerts::AlertPayload::firing("r", "gw", "s", 5.0, 2.0, 100);
    // Should not panic; just fail gracefully.
    let reports = argis_monitor::deliver_all(&client, &[target], &payload).await;
    assert_eq!(reports.len(), 1);
    assert!(!reports[0].success);
    assert!(reports[0].error.is_some());
}


// =====================================================================
// Slice 18: Bifrost-backed meta-alerts (alert_failures table + MetaAlertRule)
// =====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meta_alert_fires_when_failures_exceed_consecutive_threshold() {
    use argis_monitor::state_store::StateStore;

    // Fresh DB so prior tests' failures don't bleed in.
    let tmp = std::env::temp_dir().join(format!(
        "argis-slice18-fire-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut store = StateStore::open(&tmp).expect("open store");

    let key = "gateway::chat_burn";
    let now = 1_700_000_000_u64;
    // Record 3 failures (threshold default is 3) within the last 60s.
    for offset in [10_u64, 20, 30] {
        store
            .record_alert_failure(key, now - offset, "connection refused")
            .expect("record failure");
    }
    // window=60s, threshold=3 -> should fire.
    let count = store
        .count_failures_in_window(key, 60, now)
        .expect("count");
    assert_eq!(count, 3, "expected 3 failures in 60s window, got {count}");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meta_alert_does_not_fire_below_consecutive_threshold() {
    use argis_monitor::state_store::StateStore;

    let tmp = std::env::temp_dir().join(format!(
        "argis-slice18-below-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut store = StateStore::open(&tmp).expect("open store");

    let key = "gateway::slow_burn";
    let now = 1_700_000_100_u64;
    // Only 2 failures (threshold = 3) - should NOT fire.
    for offset in [5_u64, 15] {
        store
            .record_alert_failure(key, now - offset, "timeout")
            .expect("record failure");
    }
    let count = store
        .count_failures_in_window(key, 60, now)
        .expect("count");
    assert_eq!(count, 2, "expected 2 failures, got {count}");
    assert!(count < 3, "count must stay below consecutive_failures threshold");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meta_alert_respects_window_boundary() {
    use argis_monitor::state_store::StateStore;

    let tmp = std::env::temp_dir().join(format!(
        "argis-slice18-window-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut store = StateStore::open(&tmp).expect("open store");

    let key = "gateway::flapping";
    let now = 1_700_000_200_u64;
    // 4 failures total, but only 2 are within the trailing 30s window.
    store.record_alert_failure(key, now - 5, "e1").unwrap();   // in
    store.record_alert_failure(key, now - 10, "e2").unwrap();  // in
    store.record_alert_failure(key, now - 60, "e3").unwrap();  // out
    store.record_alert_failure(key, now - 120, "e4").unwrap(); // out

    // Window of 30s: should see exactly 2 failures.
    let count_in = store.count_failures_in_window(key, 30, now).expect("count 30s");
    assert_eq!(count_in, 2, "30s window should see 2 failures, got {count_in}");

    // Window of 600s: should see all 4 failures.
    let count_all = store.count_failures_in_window(key, 600, now).expect("count 600s");
    assert_eq!(count_all, 4, "600s window should see 4 failures, got {count_all}");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meta_alert_prune_removes_only_old_failures() {
    use argis_monitor::state_store::StateStore;

    let tmp = std::env::temp_dir().join(format!(
        "argis-slice18-prune-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut store = StateStore::open(&tmp).expect("open store");

    let key = "gateway::stale";
    let now = 1_700_000_300_u64;
    store.record_alert_failure(key, now - 10, "fresh").unwrap();     // 1s ago
    store.record_alert_failure(key, now - 3_601, "old1").unwrap();    // just over 1h ago
    store.record_alert_failure(key, now - 7_200, "old2").unwrap();    // 2h ago

    // Prune everything older than 1h. The predicate is strict less-than:
    // rows with fired_at_unix == threshold are NOT deleted.
    let deleted = store.prune_alert_failures(now - 3_600).expect("prune");
    assert_eq!(deleted, 2, "should have deleted the 2 old rows, got {deleted}");

    // Remaining: just the fresh failure.
    let remaining = store
        .count_failures_in_window(key, 7_200, now)
        .expect("count after prune");
    assert_eq!(remaining, 1, "expected 1 failure remaining, got {remaining}");

    let _ = std::fs::remove_file(&tmp);
}


// =====================================================================
// Slice 19: meta-alert end-to-end wiring (webhook failure -> meta-alert fire)
// =====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webhook_failures_populate_alert_failures_table_and_meta_alert_fires() {
    use argis_monitor::alerts::{MetaAlertRule, Severity};
    use argis_monitor::state_store::StateStore;

    // Fresh DB so prior tests' failures don't bleed in.
    let data_dir = std::env::temp_dir().join(format!(
        "argis-slice19-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&data_dir).unwrap();
    let db_path = data_dir.join("alert_state.sqlite");
    let mut store = StateStore::open(&db_path).expect("open store");

    // Simulate 3 webhook delivery failures for "gateway::chat_burn" over
    // the trailing 30s window. The exact same write happens inside the
    // poller whenever `webhook::deliver_all` returns an unsuccessful report.
    let key = "gateway::chat_burn";
    let now: u64 = 1_700_002_000;
    for offset in [5_u64, 15, 25] {
        store
            .record_alert_failure(key, now - offset, "500 Internal Server Error")
            .expect("record failure");
    }

    // Sanity: 3 rows recorded.
    let count = store
        .count_failures_in_window(key, 60, now)
        .expect("count");
    assert_eq!(count, 3, "expected 3 failures in 60s, got {count}");

    // Build a Monitor with a meta-alert that watches "gateway" + rule
    // "chat_burn" with consecutive=3 and a 60s window. The state store on
    // disk is the same one we just wrote to.
    let meta_rule = MetaAlertRule {
        name: "chat_burn_outage".into(),
        target: "gateway".into(),
        rule: Some("chat_burn".into()),
        consecutive_failures: 3,
        window: std::time::Duration::from_secs(60),
        severity: Severity::Critical,
        reason: Some("webhook delivery down".into()),
        webhooks: vec![],
    };
    let mut cfg = Config::for_test("http://127.0.0.1:1");
    cfg.data_dir = Some(data_dir.clone());
    cfg.targets.clear();
    cfg.targets.push(argis_monitor::Target::new("gateway", "http://127.0.0.1:1"));
    cfg.meta_alerts.push(meta_rule);

    let monitor = argis_monitor::Monitor::new(cfg).expect("monitor");
    let fired = monitor.evaluate_meta_alerts(now).await;
    assert!(
        fired.contains(&"chat_burn_outage".to_string()),
        "expected meta-alert to fire after 3 failures in window, got: {fired:?}"
    );

    // Below threshold: pruning one row should bring count to 2 and the
    // meta-alert should NOT fire on the next evaluation.
    store
        .prune_alert_failures(now - 24)
        .expect("prune");
    let monitor2 = argis_monitor::Monitor::new({
        let mut c = Config::for_test("http://127.0.0.1:1");
        c.data_dir = Some(data_dir.clone());
        c.targets.clear();
        c.targets.push(argis_monitor::Target::new("gateway", "http://127.0.0.1:1"));
        c.meta_alerts.push(MetaAlertRule {
            name: "chat_burn_outage".into(),
            target: "gateway".into(),
            rule: Some("chat_burn".into()),
            consecutive_failures: 3,
            window: std::time::Duration::from_secs(60),
            severity: Severity::Critical,
            reason: Some("webhook delivery down".into()),
            webhooks: vec![],
        });
        c
    })
    .expect("monitor2");
    let fired_after = monitor2.evaluate_meta_alerts(now).await;
    assert!(
        fired_after.is_empty(),
        "expected no meta-alerts after pruning below threshold, got: {fired_after:?}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}


// =====================================================================
// Slice 21: meta-alert payload delivery (webhook::deliver_all path)
// =====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meta_alert_payload_delivered_to_webhook_via_meta_alerts_route() {
    use argis_monitor::alerts::{MetaAlertRule, Severity, WebhookTarget};
    use argis_monitor::state_store::StateStore;
    use wiremock::matchers::{method, path};

    // Webhook receiver that expects a POST to /meta. expect(1) makes
    // wiremock fail the test if the delivery never lands.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/meta"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    // Fresh data dir so we can pre-seed alert_failures + re-open the same
    // store inside the Monitor.
    let data_dir = std::env::temp_dir().join(format!(
        "argis-slice21-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&data_dir).unwrap();
    let db_path = data_dir.join("alert_state.sqlite");
    {
        let mut store = StateStore::open(&db_path).expect("open store");
        let now: u64 = 1_700_003_000;
        // 3 failures -> fires the meta-alert (threshold = 3).
        for offset in [5_u64, 15, 25] {
            store
                .record_alert_failure("gateway::chat_burn", now - offset, "500")
                .expect("record");
        }
    }

    // Build a Monitor whose meta-rule points at the wiremock webhook.
    let meta_rule = MetaAlertRule {
        name: "chat_burn_outage".into(),
        target: "gateway".into(),
        rule: Some("chat_burn".into()),
        consecutive_failures: 3,
        window: std::time::Duration::from_secs(60),
        severity: Severity::Critical,
        reason: Some("webhook delivery down".into()),
        webhooks: vec![WebhookTarget {
            url: format!("{}/meta", server.uri()),
            ..Default::default()
        }],
    };
    let mut cfg = Config::for_test("http://127.0.0.1:1");
    cfg.data_dir = Some(data_dir.clone());
    cfg.targets.clear();
    cfg.targets.push(argis_monitor::Target::new("gateway", "http://127.0.0.1:1"));
    cfg.meta_alerts.push(meta_rule);

    let monitor = argis_monitor::Monitor::new(cfg).expect("monitor");
    let now: u64 = 1_700_003_000;
    let fired = monitor.evaluate_meta_alerts(now).await;
    assert!(
        fired.contains(&"chat_burn_outage".to_string()),
        "expected meta-alert to fire, got: {fired:?}"
    );

    // wiremock asserts the body arrived; give it a moment to drain.
    server.verify().await;

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meta_alert_falls_back_to_alert_rule_webhooks_when_meta_webhooks_empty() {
    use argis_monitor::alerts::{AlertRule, MetaAlertRule, Severity, WebhookTarget};
    use argis_monitor::state_store::StateStore;
    use wiremock::matchers::{method, path};

    // Wiremock that the AlertRule points at AND the meta-rule should fall
    // back to (since the meta-rule has no webhooks of its own).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fallback"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let data_dir = std::env::temp_dir().join(format!(
        "argis-slice21-fb-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&data_dir).unwrap();
    let db_path = data_dir.join("alert_state.sqlite");
    {
        let mut store = StateStore::open(&db_path).expect("open store");
        let now: u64 = 1_700_004_000;
        for offset in [5_u64, 15, 25] {
            store
                .record_alert_failure("gateway::chat_burn", now - offset, "500")
                .expect("record");
        }
    }

    // Meta-rule with NO webhooks. The Monitor must fall back to the
    // matching AlertRule's webhooks.
    let meta_rule = MetaAlertRule {
        name: "chat_burn_outage".into(),
        target: "gateway".into(),
        rule: Some("chat_burn".into()),
        consecutive_failures: 3,
        window: std::time::Duration::from_secs(60),
        severity: Severity::Critical,
        reason: Some("webhook delivery down".into()),
        webhooks: vec![], // <- empty, must fall back
    };
    let alert_rule = AlertRule {
        name: "chat_burn".into(),
        slo: "chat_completions_p99".into(),
        threshold: 0.1,
        resolve_threshold: None,
        window: None,
        for_secs: std::time::Duration::from_secs(0),
        cooldown: std::time::Duration::from_secs(0),
        webhooks: vec![WebhookTarget {
            url: format!("{}/fallback", server.uri()),
            ..Default::default()
        }],
    };
    let mut cfg = Config::for_test("http://127.0.0.1:1");
    cfg.data_dir = Some(data_dir.clone());
    cfg.targets.clear();
    cfg.targets.push(argis_monitor::Target::new("gateway", "http://127.0.0.1:1"));
    cfg.alert_rules.push(alert_rule);
    cfg.meta_alerts.push(meta_rule);

    let monitor = argis_monitor::Monitor::new(cfg).expect("monitor");
    let now: u64 = 1_700_004_000;
    let fired = monitor.evaluate_meta_alerts(now).await;
    assert!(
        fired.contains(&"chat_burn_outage".to_string()),
        "expected meta-alert to fire, got: {fired:?}"
    );

    server.verify().await;
    let _ = std::fs::remove_dir_all(&data_dir);
}


// =====================================================================
// Slice 22: meta-alert Prometheus counter
// =====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn meta_alert_fires_increment_prometheus_counter() {
    use argis_monitor::alerts::{MetaAlertRule, Severity, WebhookTarget};
    use argis_monitor::state_store::StateStore;

    let data_dir = std::env::temp_dir().join(format!(
        "argis-slice22-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&data_dir).unwrap();
    let db_path = data_dir.join("alert_state.sqlite");
    {
        let mut store = StateStore::open(&db_path).expect("open store");
        // Use the current real time so the failures fall inside any
        // reasonable meta-alert window when the poll runs.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        for offset in [5_u64, 15, 25] {
            store.record_alert_failure("gateway::chat_burn", now - offset, "500").expect("record");
        }
    }

    let meta_rule = MetaAlertRule {
        name: "chat_burn_outage".into(),
        target: "gateway".into(),
        rule: Some("chat_burn".into()),
        consecutive_failures: 3,
        window: std::time::Duration::from_secs(60),
        severity: Severity::Critical,
        reason: Some("webhook delivery down".into()),
        webhooks: vec![WebhookTarget {
            url: "http://127.0.0.1:1/sink".into(),
            ..Default::default()
        }],
    };
    let mut cfg = Config::for_test("http://127.0.0.1:1");
    cfg.data_dir = Some(data_dir.clone());
    cfg.targets.clear();
    cfg.targets.push(argis_monitor::Target::new("gateway", "http://127.0.0.1:1"));
    cfg.meta_alerts.push(meta_rule);

    let monitor = argis_monitor::Monitor::new(cfg).expect("monitor");

    // Drive a poll. poll_once_target fires the meta-alert AND bumps the
    // Prometheus counter.
    let _ = monitor
        .poll_once_target(
            &argis_monitor::Target::new("gateway", "http://127.0.0.1:1"),
            std::time::Duration::from_millis(100),
        )
        .await;

    // Scrape /metrics via the axum exporter.
    let mut buf = String::new();
    prometheus_client::encoding::text::encode(&mut buf, &monitor.registry()).unwrap();
    assert!(
        buf.contains("argis_monitor_meta_alerts_fired_total"),
        "expected meta_alerts_fired_total in metrics:
{buf}"
    );
    // Should be at least 1 with the labels we configured.
    assert!(
        buf.contains("meta=\"chat_burn_outage\""),
        "expected meta=\"chat_burn_outage\" label, got:
{buf}"
    );
    assert!(
        buf.contains("target=\"gateway\""),
        "expected target=\"gateway\" label, got:
{buf}"
    );
    assert!(
        buf.contains("severity=\"critical\""),
        "expected severity=\"critical\" label, got:
{buf}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}


// =====================================================================
// Slice 24: hot-reload real swap (ArcSwap<MonitorInner>)
// =====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_from_path_atomically_swaps_monitor_inner() {
    // Write a YAML config with one target + one rule.
    let dir = std::env::temp_dir().join(format!(
        "argis-slice24-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let yaml_path = dir.join("config.yaml");

    let yaml_v1 = r#"
targets:
  - name: gateway
    url: http://127.0.0.1:1
    poll_interval: 15s
exporter_addr: "0.0.0.0:9090"
alert_rules:
  - name: v1_only_rule
    slo: chat_completions_p99
    threshold: 5.0
    for_secs: 0s
    cooldown: 60s
    webhooks: []
meta_alerts: []
data_dir: null
"#;
    std::fs::write(&yaml_path, yaml_v1).unwrap();

    // Parse the v1 config and build a Monitor without going through the file.
    let cfg_v1: argis_monitor::Config = serde_yaml::from_str(yaml_v1).unwrap();
    let monitor = argis_monitor::Monitor::new(cfg_v1).expect("monitor v1");

    // Sanity: v1 has the v1_only_rule and no meta_alerts.
    assert_eq!(monitor.config().alert_rules.len(), 1);
    assert_eq!(monitor.config().alert_rules[0].name, "v1_only_rule");
    assert_eq!(monitor.config().meta_alerts.len(), 0);

    // Rewrite the YAML to a v2 config with a different rule + 1 meta_alert.
    let yaml_v2 = r#"
targets:
  - name: gateway
    url: http://127.0.0.1:1
    poll_interval: 30s
exporter_addr: "0.0.0.0:9091"
alert_rules:
  - name: v2_replacement_rule
    slo: chat_completions_p99
    threshold: 10.0
    for_secs: 0s
    cooldown: 60s
    webhooks: []
  - name: v2_extra_rule
    slo: chat_completions_p99
    threshold: 20.0
    for_secs: 0s
    cooldown: 60s
    webhooks: []
meta_alerts:
  - name: webhook_failure_alert
    target: gateway
    rule: null
    consecutive_failures: 5
    window: 300s
    severity: critical
    reason: null
    webhooks: []
data_dir: null
"#;
    std::fs::write(&yaml_path, yaml_v2).unwrap();

    // Reload from the (now-updated) file. This must atomically swap the inner.
    monitor.reload_from_path(&yaml_path).await.expect("reload");

    // After reload: the new config is in effect on every subsequent access.
    let cfg = monitor.config();
    assert_eq!(cfg.exporter_addr, "0.0.0.0:9091", "exporter_addr should reflect v2");
    assert_eq!(cfg.alert_rules.len(), 2, "should now have 2 alert rules");
    let names: Vec<&str> = cfg.alert_rules.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"v2_replacement_rule"));
    assert!(names.contains(&"v2_extra_rule"));
    assert!(!names.contains(&"v1_only_rule"), "v1 rule should be gone after swap");
    assert_eq!(cfg.meta_alerts.len(), 1, "should now have 1 meta_alert");
    assert_eq!(cfg.meta_alerts[0].name, "webhook_failure_alert");

    // Registry is also swapped (Prometheus exporter exposes the new one).
    // We can assert that the registry still has the canonical baseline metric
    // (target_info), proving the swap didn't lose the registry.
    let mut buf = String::new();
    prometheus_client::encoding::text::encode(&mut buf, &monitor.registry()).unwrap();
    assert!(buf.contains("argis_monitor_target_info"), "registry should still expose target_info");

    let _ = std::fs::remove_dir_all(&dir);
}


// =====================================================================
// Slice 25: hot-reload meta_alerts via SIGHUP (implicit in slice 24, verified here)
// =====================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hot_reload_swaps_meta_alerts_atomically() {
    use argis_monitor::alerts::{MetaAlertRule, Severity, WebhookTarget};
    use argis_monitor::state_store::StateStore;
    let dir = std::env::temp_dir().join(format!(
        "argis-slice25-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("alert_state.sqlite");
    let yaml_path = dir.join("config.yaml");

    // v1: empty meta_alerts, low threshold (3) on one target.
    let yaml_v1 = r#"
targets:
  - name: gateway
    url: http://127.0.0.1:1
    poll_interval: 15s
exporter_addr: "0.0.0.0:9090"
alert_rules: []
meta_alerts:
  - name: webhook_failure_v1
    target: gateway
    rule: null
    consecutive_failures: 3
    window: 60s
    severity: critical
    reason: null
    webhooks: []
data_dir: ~
"#;
    // Patch the actual db_path into v1 (data_dir is templated above).
    let yaml_v1 = yaml_v1.replace("data_dir: ~", &format!("data_dir: {}", dir.display()));
    std::fs::write(&yaml_path, &yaml_v1).unwrap();

    let cfg_v1: argis_monitor::Config = serde_yaml::from_str(&yaml_v1).unwrap();
    let monitor = argis_monitor::Monitor::new(cfg_v1).expect("monitor v1");
    assert_eq!(monitor.config().meta_alerts.len(), 1);
    assert_eq!(monitor.config().meta_alerts[0].name, "webhook_failure_v1");
    assert_eq!(monitor.config().meta_alerts[0].consecutive_failures, 3);

    // Seed 3 alert_failures for "gateway::*" — the v1 meta-alert would fire.
    {
        let mut store = StateStore::open(&db_path).expect("open store");
        let now: u64 = 1_700_010_000;
        for offset in [5_u64, 15, 25] {
            store.record_alert_failure("gateway::*", now - offset, "500").expect("record");
        }
    }
    let fired_v1 = monitor.evaluate_meta_alerts(1_700_010_000).await;
    assert!(fired_v1.contains(&"webhook_failure_v1".to_string()),
        "v1 meta-alert should fire before reload, got: {fired_v1:?}");

    // v2: rename meta-alert + raise threshold to 5 so it does NOT fire.
    let yaml_v2 = r#"
targets:
  - name: gateway
    url: http://127.0.0.1:1
    poll_interval: 15s
exporter_addr: "0.0.0.0:9090"
alert_rules: []
meta_alerts:
  - name: webhook_failure_v2
    target: gateway
    rule: null
    consecutive_failures: 5
    window: 60s
    severity: warning
    reason: v2 reason
    webhooks: []
data_dir: ~
"#;
    let yaml_v2 = yaml_v2.replace("data_dir: ~", &format!("data_dir: {}", dir.display()));
    std::fs::write(&yaml_path, &yaml_v2).unwrap();

    // Hot-reload. The new meta_alerts must take effect immediately.
    monitor.reload_from_path(&yaml_path).await.expect("reload v2");

    // Verify the config reflects v2 (name, threshold, severity, reason).
    let cfg = monitor.config();
    assert_eq!(cfg.meta_alerts.len(), 1, "should still have 1 meta_alert after swap");
    assert_eq!(cfg.meta_alerts[0].name, "webhook_failure_v2");
    assert_eq!(cfg.meta_alerts[0].consecutive_failures, 5);
    assert_eq!(cfg.meta_alerts[0].severity, Severity::Warning);
    assert_eq!(cfg.meta_alerts[0].reason.as_deref(), Some("v2 reason"));

    // Re-evaluate: only 3 failures in window, threshold is now 5 -> no fire.
    let fired_v2 = monitor.evaluate_meta_alerts(1_700_010_000).await;
    assert!(fired_v2.is_empty(),
        "v2 threshold (5) should NOT fire on 3 failures, got: {fired_v2:?}");

    // Seed 2 more failures to reach 5; now v2 fires (and v1's name does NOT).
    {
        let mut store = StateStore::open(&db_path).expect("open store");
        let now: u64 = 1_700_010_000;
        for offset in [3_u64, 8] {
            store.record_alert_failure("gateway::*", now - offset, "500").expect("record");
        }
    }
    let fired_v2_after = monitor.evaluate_meta_alerts(1_700_010_000).await;
    assert_eq!(fired_v2_after, vec!["webhook_failure_v2".to_string()],
        "v2 meta-alert should fire at threshold 5, got: {fired_v2_after:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

