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
//!
//! User-supplied HTTP headers are validated before delivery. An invalid name
//! or value is logged and fails closed: no request is sent and no retry is
//! attempted; the returned [`DeliveryReport`] contains the configuration
//! error.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{error, info, warn};

use crate::alerts::{AlertPayload, WebhookTarget};

#[derive(Debug, Clone)]
struct AwsDeliveryConfig {
    region: String,
    service: String,
    creds: crate::aws_sigv4::AwsCreds,
}

#[derive(Debug, Clone)]
struct RequestPayload {
    body: Vec<u8>,
    content_type: &'static str,
    fixed_headers: HashMap<String, String>,
}

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

async fn deliver_one(
    http: &reqwest::Client,
    target: &WebhookTarget,
    payload: &AlertPayload,
) -> DeliveryReport {
    let aws = match resolve_aws_config(target) {
        Ok(config) => config,
        Err(error) => {
            error!(url = %target.url, error = %error, "invalid AWS webhook configuration");
            return DeliveryReport {
                url: target.url.clone(),
                success: false,
                status: None,
                error: Some(error),
            };
        }
    };
    let request_payload = match build_request_payload(target, payload, aws.as_ref()) {
        Ok(request) => request,
        Err(error) => {
            error!(url = %target.url, error = %error, "invalid AWS webhook payload configuration");
            return DeliveryReport {
                url: target.url.clone(),
                success: false,
                status: None,
                error: Some(error),
            };
        }
    };
    if let Err(error) = validate_user_headers(target) {
        error!(url = %target.url, error = %error, "invalid webhook headers; delivery aborted");
        return DeliveryReport {
            url: target.url.clone(),
            success: false,
            status: None,
            error: Some(error),
        };
    }
    let mut last_err = None;
    let mut last_status = None;
    let is_aws_target = aws.is_some();
    for attempt in 0..2u8 {
        let mut req = match http
            .post(&target.url)
            .header(reqwest::header::CONTENT_TYPE, request_payload.content_type)
            .body(request_payload.body.clone())
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(format!("build: {e}"));
                continue;
            }
        };
        // apply user-supplied headers
        let mut headers_for_signing = HashMap::new();
        {
            let headers = req.headers_mut();
            for (k, v) in &target.headers {
                if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                    if is_aws_target
                        && (name == reqwest::header::CONTENT_TYPE
                            || name == reqwest::header::HOST
                            || request_payload
                                .fixed_headers
                                .keys()
                                .any(|fixed| fixed.eq_ignore_ascii_case(name.as_str())))
                    {
                        warn!(header = %name, url = %target.url, "ignoring AWS target header that is fixed by SigV4 signing");
                        continue;
                    }
                    if let Ok(val) = reqwest::header::HeaderValue::from_str(v) {
                        headers.insert(name, val);
                        headers_for_signing.insert(k.clone(), v.clone());
                    }
                }
            }
            for (name, value) in &request_payload.fixed_headers {
                if let (Ok(name), Ok(value)) = (
                    reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                    reqwest::header::HeaderValue::from_str(value),
                ) {
                    let signing_name = name.as_str().to_string();
                    let signing_value = value.to_str().unwrap_or_default().to_string();
                    headers.insert(name, value);
                    headers_for_signing.insert(signing_name, signing_value);
                }
            }
        }
        // SigV4 signing (if AWS config present).
        if let Some(aws) = aws.as_ref() {
            match crate::aws_sigv4::sign_request_headers_with_headers_and_content_type(
                "POST",
                &target.url,
                Some(&request_payload.body),
                &aws.region,
                &aws.service,
                &aws.creds,
                &headers_for_signing,
                request_payload.content_type,
            ) {
                Ok(sig) => {
                    let h = req.headers_mut();
                    for (k, v) in sig {
                        if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                            if let Ok(val) = reqwest::header::HeaderValue::from_str(&v) {
                                h.insert(name, val);
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
                    return DeliveryReport {
                        url: target.url.clone(),
                        success: true,
                        status: Some(s),
                        error: None,
                    };
                } else {
                    last_err = Some(format!("http {s}"));
                    // A signed POST may have reached AWS even when its response is an
                    // error, so retrying it could duplicate the alert delivery.
                    if is_aws_target {
                        break;
                    }
                }
            }
            Err(e) => {
                last_err = Some(e.to_string());
            }
        }
        if attempt == 0 {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    error!(url = %target.url, error = ?last_err, status = ?last_status, "webhook delivery failed");
    DeliveryReport {
        url: target.url.clone(),
        success: false,
        status: last_status,
        error: last_err,
    }
}

fn validate_user_headers(target: &WebhookTarget) -> Result<(), String> {
    let mut errors = Vec::new();
    for (name, value) in &target.headers {
        if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err() {
            warn!(url = %target.url, header = %name, "invalid webhook header name; delivery will be aborted");
            errors.push(format!("invalid header name `{name}`"));
            continue;
        }
        if reqwest::header::HeaderValue::from_str(value).is_err() {
            warn!(url = %target.url, header = %name, "invalid webhook header value; delivery will be aborted");
            errors.push(format!("invalid value for header `{name}`"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("webhook header validation failed: {}", errors.join("; ")))
    }
}

fn resolve_aws_config(target: &WebhookTarget) -> Result<Option<AwsDeliveryConfig>, String> {
    let has_aws_fields = target.aws_region.is_some()
        || target.aws_service.is_some()
        || target.aws_access_key_id.is_some()
        || target.aws_secret_access_key.is_some()
        || target.aws_session_token.is_some()
        || target.aws_topic_arn.is_some();
    if !has_aws_fields {
        return Ok(None);
    }

    let region = target
        .aws_region
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "aws_region is required when AWS options are configured".to_string())?;
    let service = target
        .aws_service
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_ascii_lowercase())
        .ok_or_else(|| "aws_service is required when AWS options are configured".to_string())?;
    let service = if service == "eventbridge" {
        "events".to_string()
    } else {
        service
    };
    if service == "sns"
        && target
            .aws_topic_arn
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err("aws_topic_arn is required for SNS Publish targets".to_string());
    }
    if service != "sns" && target.aws_topic_arn.is_some() {
        return Err("aws_topic_arn is only valid with aws_service: sns".to_string());
    }

    let inline_access = target.aws_access_key_id.as_deref();
    let inline_secret = target.aws_secret_access_key.as_deref();
    if inline_access.is_some() != inline_secret.is_some() {
        return Err(
            "aws_access_key_id and aws_secret_access_key must be configured together".to_string(),
        );
    }
    let access = inline_access
        .map(str::to_owned)
        .or_else(|| std::env::var("AWS_ACCESS_KEY_ID").ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "AWS access key is missing (configure it or AWS_ACCESS_KEY_ID)".to_string()
        })?;
    let secret = inline_secret
        .map(str::to_owned)
        .or_else(|| std::env::var("AWS_SECRET_ACCESS_KEY").ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "AWS secret key is missing (configure it or AWS_SECRET_ACCESS_KEY)".to_string()
        })?;
    let session = target
        .aws_session_token
        .clone()
        .or_else(|| std::env::var("AWS_SESSION_TOKEN").ok())
        .filter(|value| !value.trim().is_empty());

    Ok(Some(AwsDeliveryConfig {
        region: region.trim().to_string(),
        service,
        creds: crate::aws_sigv4::AwsCreds {
            access_key: access,
            secret_key: secret,
            session_token: session,
        },
    }))
}

fn build_request_payload(
    target: &WebhookTarget,
    payload: &AlertPayload,
    aws: Option<&AwsDeliveryConfig>,
) -> Result<RequestPayload, String> {
    let detail = serde_json::to_string(payload).map_err(|error| format!("serialize: {error}"))?;
    let Some(aws) = aws else {
        return Ok(RequestPayload {
            body: detail.into_bytes(),
            content_type: "application/json",
            fixed_headers: HashMap::new(),
        });
    };

    let (body, content_type, fixed_headers) = match aws.service.as_str() {
        "sns" => {
            let topic_arn = target
                .aws_topic_arn
                .as_deref()
                .expect("SNS topic ARN validated by resolve_aws_config");
            let body = serde_json::to_vec(&serde_json::json!({
                "Action": "Publish",
                "Message": detail,
                "TopicArn": topic_arn,
                "Version": "2010-03-31",
            }))
            .map_err(|error| format!("serialize SNS payload: {error}"))?;
            let mut headers = HashMap::new();
            headers.insert("x-amz-target".to_string(), "AmazonSNS.Publish".to_string());
            (body, "application/x-amz-json-1.0", headers)
        }
        "events" => {
            let body = serde_json::to_vec(&serde_json::json!({
                "Entries": [{
                    "Detail": detail,
                    "DetailType": "argis-monitor.alert",
                    "Source": "argis-monitor",
                }],
            }))
            .map_err(|error| format!("serialize EventBridge payload: {error}"))?;
            let mut headers = HashMap::new();
            headers.insert(
                "x-amz-target".to_string(),
                "AWSEvents.PutEvents".to_string(),
            );
            (body, "application/x-amz-json-1.1", headers)
        }
        _ => (detail.into_bytes(), "application/json", HashMap::new()),
    };
    Ok(RequestPayload {
        body,
        content_type,
        fixed_headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_report_serializes_to_json() {
        let r = DeliveryReport {
            url: "http://example.com".into(),
            success: true,
            status: Some(200),
            error: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("true"), "expected true in: {s}");
        assert!(!s.contains("false"), "expected no false in: {s}");
    }

    fn signed_target(service: &str) -> WebhookTarget {
        WebhookTarget {
            url: "https://example.amazonaws.com/".into(),
            aws_region: Some("us-east-1".into()),
            aws_service: Some(service.into()),
            aws_access_key_id: Some("AKID".into()),
            aws_secret_access_key: Some("SECRET".into()),
            ..Default::default()
        }
    }

    #[test]
    fn partial_aws_configuration_is_rejected_before_delivery() {
        let target = WebhookTarget {
            url: "https://example.amazonaws.com/".into(),
            aws_access_key_id: Some("AKID".into()),
            ..Default::default()
        };
        let error = resolve_aws_config(&target).unwrap_err();
        assert!(error.contains("aws_region"), "unexpected error: {error}");

        let target = WebhookTarget {
            url: "https://example.amazonaws.com/".into(),
            aws_region: Some("us-east-1".into()),
            aws_service: Some("events".into()),
            ..Default::default()
        };
        let error = resolve_aws_config(&target).unwrap_err();
        assert!(error.contains("access key"), "unexpected error: {error}");
    }

    #[test]
    fn sns_payload_uses_publish_envelope_and_topic_arn() {
        let mut target = signed_target("sns");
        target.aws_topic_arn = Some("arn:aws:sns:us-east-1:123456789012:alerts".into());
        let aws = resolve_aws_config(&target).unwrap().unwrap();
        let payload = AlertPayload::firing("r", "gateway", "s", 5.0, 2.0, 12345);
        let request = build_request_payload(&target, &payload, Some(&aws)).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(request.content_type, "application/x-amz-json-1.0");
        assert_eq!(request.fixed_headers["x-amz-target"], "AmazonSNS.Publish");
        assert_eq!(body["Action"], "Publish");
        assert_eq!(body["Version"], "2010-03-31");
        assert_eq!(
            body["TopicArn"],
            "arn:aws:sns:us-east-1:123456789012:alerts"
        );
        assert!(body["Message"].as_str().unwrap().contains("\"rule\":\"r\""));
    }

    #[test]
    fn eventbridge_payload_uses_put_events_envelope() {
        let target = signed_target("events");
        let aws = resolve_aws_config(&target).unwrap().unwrap();
        let payload = AlertPayload::resolved("r", "gateway", "s", 0.5, 1.0, 12345);
        let request = build_request_payload(&target, &payload, Some(&aws)).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(request.content_type, "application/x-amz-json-1.1");
        assert_eq!(request.fixed_headers["x-amz-target"], "AWSEvents.PutEvents");
        let entry = &body["Entries"][0];
        assert_eq!(entry["Source"], "argis-monitor");
        assert_eq!(entry["DetailType"], "argis-monitor.alert");
        assert!(entry["Detail"]
            .as_str()
            .unwrap()
            .contains("\"severity\":\"ok\""));
    }

    #[tokio::test]
    async fn invalid_user_header_name_or_value_fails_closed() {
        for (header, value, expected) in [
            ("invalid header", "value", "invalid header name"),
            ("x-valid", "value\nwith-control", "invalid value"),
        ] {
            let target = WebhookTarget {
                url: "http://127.0.0.1:1/should-not-be-called".into(),
                headers: HashMap::from([(header.into(), value.into())]),
                ..Default::default()
            };
            let payload = AlertPayload::firing("r", "gateway", "s", 5.0, 2.0, 12345);
            let reports = deliver_all(&reqwest::Client::new(), &[target], &payload).await;

            assert_eq!(reports.len(), 1);
            assert!(!reports[0].success);
            assert_eq!(reports[0].status, None);
            assert!(reports[0].error.as_deref().unwrap().contains(expected));
        }
    }
}
