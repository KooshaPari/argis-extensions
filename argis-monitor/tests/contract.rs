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
