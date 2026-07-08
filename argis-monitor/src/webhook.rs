//! Webhook delivery for alert payloads.
//!
//! Each firing rule POSTs its `AlertPayload` as JSON to every configured
//! webhook URL. Delivery is best-effort: a single retry with exponential
//! backoff, then drop. Failed deliveries are logged but do not affect the
//! alert state machine.
//!
//! When the WebhookTarget carries AWS credentials, the request is signed
//! with AWS SigV4 (see `crate::aws_sigv4`) before being sent. This is
//! required for SNS / EventBridge / Lambda webhook targets.

use std::time::Duration;

fn headers_mut_insert(h: &mut reqwest::header::HeaderMap, name: reqwest::header::HeaderName, val: reqwest::header::HeaderValue) {
    h.insert(name, val);
}

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{error, info, warn};

use crate::alerts::{AlertPayload, WebhookTarget};

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("HTTP transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("webhook returned non-2xx: {status}")]
    NonSuccess { status: u16 },
    #[error("webhook URL parse: {0}")]
    InvalidUrl(String),
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
    let mut reports = Vec::with_capacity(targets.len());
    for t in targets {
        reports.push(deliver_one(http, t, payload).await);
    }
    reports
}

async fn deliver_one(http: &reqwest::Client, target: &WebhookTarget, payload: &AlertPayload) -> DeliveryReport {
    let mut last_err = None;
    let mut last_status = None;
    for attempt in 0..2u8 {
        let body = match serde_json::to_vec(payload) {
            Ok(b) => b,
            Err(e) => { last_err = Some(format!("serialize: {e}")); continue; }
        };
        let mut req = match http.post(&target.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.clone())
            .build() {
            Ok(r) => r,
            Err(e) => { last_err = Some(format!("build: {e}")); continue; }
        };
        // apply user-supplied headers
        {
            let headers = req.headers_mut();
            for (k, v) in &target.headers {
                if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                    if let Ok(val) = reqwest::header::HeaderValue::from_str(v) {
                        headers.insert(name, val);
                    }
                }
            }
        }
        // bearer_token / bearer_token_file (slice 10d).
        if target.bearer_token.is_some() || target.bearer_token_file.is_some() {
            // No global cache yet; re-resolve per request. Cheap (Option<String>).
            // A process-global cache is in src/auth.rs::BearerTokenCache for future use.
            let resolved = if let Some(path) = &target.bearer_token_file {
                match tokio::fs::read(path).await {
                    Ok(b) => Some(String::from_utf8_lossy(&b).trim().to_string()),
                    Err(e) => {
                        tracing::warn!(error = %e, "bearer_token_file read failed; skipping delivery");
                        None
                    }
                }
            } else {
                target.bearer_token.clone()
            };
            if let Some(token) = resolved {
                if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")) {
                    req.headers_mut().insert(reqwest::header::AUTHORIZATION, val);
                }
            } else {
                return DeliveryReport { url: target.url.clone(), success: false, status: None, error: Some("bearer_token_file read failed".into()) };
            }
        }
        // SigV4 signing (if AWS config present).
        if target.aws_region.is_some() && target.aws_service.is_some() {
            let creds = crate::aws_sigv4::AwsCreds {
                access_key: target.aws_access_key_id.clone()
                    .or_else(|| std::env::var("AWS_ACCESS_KEY_ID").ok())
                    .unwrap_or_default(),
                secret_key: target.aws_secret_access_key.clone()
                    .or_else(|| std::env::var("AWS_SECRET_ACCESS_KEY").ok())
                    .unwrap_or_default(),
                session_token: target.aws_session_token.clone()
                    .or_else(|| std::env::var("AWS_SESSION_TOKEN").ok()),
            };
            match crate::aws_sigv4::sign_request_headers(
                "POST",
                &target.url,
                Some(&body),
                target.aws_region.as_deref().unwrap_or("us-east-1"),
                target.aws_service.as_deref().unwrap(),
                &creds,
            ) {
                Ok(sig) => {
                    let h = req.headers_mut();
                    for (k, v) in sig {
                        if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                            if let Ok(val) = reqwest::header::HeaderValue::from_str(&v) {
                                headers_mut_insert(h, name, val);
                            }
                        }
                    }
                }
                Err(e) => {
                    last_err = Some(format!("aws sign: {e}"));
                    continue;
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
