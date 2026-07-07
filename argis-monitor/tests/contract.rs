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
