//! Webhook delivery for alert payloads.
//!
//! Each firing rule POSTs its `AlertPayload` as JSON to every configured
//! webhook URL. Delivery is best-effort: a single retry with exponential
//! backoff, then drop. Failed deliveries are logged but do not affect the
//! alert state machine.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{error, info};

use crate::alerts::{AlertPayload, WebhookTarget};

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("HTTP transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("webhook returned non-2xx: {status}")]
    NonSuccess { status: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReport {
    pub url: String,
    pub success: bool,
    pub status: Option<u16>,
    pub error: Option<String>,
}

/// Deliver `payload` to every webhook in `targets`. Returns one report per
/// target (success or failure). Best-effort: 1 retry with 250ms backoff.
pub async fn deliver_all(
    http: &reqwest::Client,
    targets: &[WebhookTarget],
    payload: &AlertPayload,
) -> Vec<DeliveryReport> {
    let mut tasks = tokio::task::JoinSet::new();
    for target in targets.iter().cloned() {
        let http = http.clone();
        let payload = payload.clone();
        tasks.spawn(async move { deliver_one(&http, &target, &payload).await });
    }

    let mut reports = Vec::with_capacity(targets.len());
    while let Some(result) = tasks.join_next().await {
        if let Ok(report) = result {
            reports.push(report);
        }
    }
    reports
}

async fn deliver_one(http: &reqwest::Client, target: &WebhookTarget, payload: &AlertPayload) -> DeliveryReport {
    let mut last_err = None;
    let mut last_status = None;
    for attempt in 0..2u8 {
        let mut req = match http.post(&target.url).json(payload).build() {
            Ok(r) => r,
            Err(e) => { last_err = Some(format!("build: {e}")); continue; }
        };
        // apply headers
        let headers = req.headers_mut();
        for (k, v) in &target.headers {
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                if let Ok(val) = reqwest::header::HeaderValue::from_str(v) {
                    headers.insert(name, val);
                }
            }
        }
        match http.execute(req).await {
            Ok(resp) => {
                let s = resp.status().as_u16();
                last_status = Some(s);
                if resp.status().is_success() {
                    info!(url = %target.url, attempt, status = s, "webhook delivered");
                    return DeliveryReport { url: target.url.clone(), success: true, status: Some(s), error: None };
                } else {
                    last_err = Some(format!("http {s}"));
                }
            }
            Err(e) => { last_err = Some(e.to_string()); }
        }
        if attempt == 0 {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    error!(url = %target.url, error = ?last_err, status = ?last_status, "webhook delivery failed");
    DeliveryReport { url: target.url.clone(), success: false, status: last_status, error: last_err }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_report_serializes_to_json() {
        let r = DeliveryReport { url: "http://example.com".into(), success: true, status: Some(200), error: None };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("true"), "expected true in: {s}");
        assert!(!s.contains("false"), "expected no false in: {s}");
    }
}
