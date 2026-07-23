//! axum-based Prometheus exposition server.
//!
//! `argis-monitor` exposes a single HTTP endpoint at `/metrics` that returns
//! the Prometheus text exposition format. Run alongside the `Monitor` poll
//! loop (typically in the same binary).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Router};
use prometheus_client::encoding::text::encode;
use prometheus_client::registry::Registry;
use tokio::sync::watch;
use tower_http::trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::{error, info, Level};

/// Handle to the running exporter.
#[derive(Clone)]
pub struct ExporterHandle {
    pub addr: SocketAddr,
    pub shutdown: Arc<watch::Sender<bool>>,
}

#[derive(Clone)]
struct AppState {
    registry: Arc<Registry>,
}

/// Spawn the exporter. Resolves once the server is listening.
///
/// Slice 33: every incoming HTTP request gets a tracing span
/// (method, uri, status, latency). We use `tower_http::trace::TraceLayer` with
/// default hooks at INFO; the span name is `http_request`. Operators can
/// adjust the level via `RUST_LOG` (e.g. `RUST_LOG=argis_monitor::exporter=debug`).
pub async fn serve(addr: &str, registry: Arc<Registry>) -> anyhow::Result<ExporterHandle> {
    let state = AppState { registry };
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/healthz", get(healthz))
        .with_state(state)
        // TraceLayer must be the outermost layer so it captures the final
        // status + latency after the inner handlers run.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO).include_headers(false))
                .on_request(DefaultOnRequest::new().level(Level::DEBUG))
                .on_response(DefaultOnResponse::new().level(Level::INFO).include_headers(false)),
        );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let (tx, mut rx) = watch::channel(false);
    let shutdown_tx = Arc::new(tx);

    tokio::spawn(async move {
        info!(addr = %local_addr, "argis-monitor exporter listening");
        let server = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = rx.changed().await;
            });
        if let Err(e) = server.await {
            error!(error = %e, "exporter server stopped");
        }
    });

    Ok(ExporterHandle {
        addr: local_addr,
        shutdown: shutdown_tx,
    })
}

async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let mut buf = String::new();
    if let Err(e) = encode(&mut buf, &state.registry) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("encode error: {e}
"),
        );
    }
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        buf,
    )
}

async fn healthz() -> &'static str { "ok" }
