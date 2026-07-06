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
