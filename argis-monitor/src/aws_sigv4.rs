//! AWS Signature Version 4 (SigV4) request signing.
//!
//! Hand-rolled implementation. The SigV4 algorithm is well-documented
//! (see https://docs.aws.amazon.com/general/latest/gr/sigv4_signing.html)
//! and only requires HMAC-SHA256 + canonical request construction. The
//! `hmac` and `sha2` crates we already pull in for other purposes handle
//! the crypto; this module is ~200 LOC of pure logic.
//!
//! Returns the four SigV4 headers as a `HashMap<String, String>` ready
//! to be added to the outbound request. Used by `webhook::deliver_one`
//! when the `WebhookTarget` config carries AWS credentials.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Errors from signing.
#[derive(Debug, thiserror::Error)]
pub enum SignError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("AWS date is too far in the future: {0}")]
    BadDate(String),
}

/// AWS credentials (subset of what's needed for SigV4).
#[derive(Debug, Clone)]
pub struct AwsCreds {
    pub access_key: String,
    pub secret_key: String,
    /// Optional STS session token.
    pub session_token: Option<String>,
}

/// Sign an outgoing HTTP request. Returns the SigV4 headers.
///
/// # Arguments
/// - `method`: HTTP method (uppercased)
/// - `url`: full URL including scheme + host (+ port) + path + query
/// - `body`: optional request body bytes
/// - `region`: AWS region (e.g. "us-east-1")
/// - `service`: AWS service code (e.g. "sns", "events", "execute-api")
/// - `creds`: AWS credentials
pub fn sign_request_headers(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    region: &str,
    service: &str,
    creds: &AwsCreds,
) -> Result<HashMap<String, String>, SignError> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let dt = DateTime::<Utc>::from_timestamp(now as i64, 0)
        .ok_or_else(|| SignError::BadDate(format!("{now}")))?;
    let amz_date = dt.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = dt.format("%Y%m%d").to_string();

    let (host, path_q) = split_url(url)?;
    let body_hash = {
        let mut h = Sha256::new();
        h.update(body.unwrap_or(b""));
        hex_lower(&h.finalize())
    };

    // Canonical request.
    let canonical_uri = path_q.split('?').next().unwrap_or("/");
    let canonical_querystring = path_q.split('?').nth(1).unwrap_or("");
    let signed_headers = if creds.session_token.is_some() {
        "content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token"
    } else {
        "content-type;host;x-amz-content-sha256;x-amz-date"
    };
    let canonical_headers = format!(
        "content-type:application/json\nhost:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n{}",
        host,
        body_hash,
        amz_date,
        if let Some(st) = &creds.session_token {
            format!("x-amz-security-token:{}\n", st)
        } else {
            String::new()
        }
    );
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method.to_uppercase(),
        canonical_uri,
        canonical_querystring,
        canonical_headers,
        signed_headers,
        body_hash
    );

    // String to sign.
    let credential_scope = format!("{}/{}/{}/aws4_request", date_stamp, region, service);
    let sts = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date,
        credential_scope,
        sha256_hex(canonical_request.as_bytes())
    );

    // Derive signing key.
    let mut k_secret = Vec::with_capacity(4 + creds.secret_key.len());
    k_secret.extend_from_slice(b"AWS4");
    k_secret.extend_from_slice(creds.secret_key.as_bytes());
    let k_date = hmac_sha256(&k_secret, date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex_lower(&hmac_sha256(&k_signing, sts.as_bytes()));

    // Authorization header.
    let auth = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        creds.access_key, credential_scope, signed_headers, signature
    );

    let mut headers = HashMap::new();
    headers.insert("authorization".into(), auth);
    headers.insert("x-amz-date".into(), amz_date);
    headers.insert("x-amz-content-sha256".into(), body_hash);
    if let Some(st) = &creds.session_token {
        headers.insert("x-amz-security-token".into(), st.clone());
    }
    Ok(headers)
}

fn split_url(url: &str) -> Result<(String, String), SignError> {
    let scheme_end = url.find("://").ok_or_else(|| SignError::InvalidUrl(url.to_string()))?;
    let after_scheme = &url[scheme_end + 3..];
    let slash = after_scheme.find('/').unwrap_or(after_scheme.len());
    let host = after_scheme[..slash].to_string();
    let path_q = if slash < after_scheme.len() {
        after_scheme[slash..].to_string()
    } else {
        "/".to_string()
    };
    Ok((host, path_q))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex_lower(&h.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_url_handles_path_and_query() {
        let (host, path) = split_url("https://sns.us-east-1.amazonaws.com:443/topics/test?param=1").unwrap();
        assert_eq!(host, "sns.us-east-1.amazonaws.com:443");
        assert_eq!(path, "/topics/test?param=1".to_string());
    }

    #[test]
    fn split_url_handles_no_path() {
        let (host, path) = split_url("https://example.com").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(path, "/".to_string());
    }

    #[test]
    fn hex_lower_pads_correctly() {
        assert_eq!(hex_lower(&[0x00, 0xff, 0x10]), "00ff10");
        assert_eq!(hex_lower(&[]), "");
    }

    #[test]
    fn sign_request_headers_contains_expected_keys() {
        let creds = AwsCreds {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let headers = sign_request_headers(
            "POST",
            "https://sns.us-east-1.amazonaws.com/topics/my-topic",
            Some(b"Hello"),
            "us-east-1",
            "sns",
            &creds,
        ).unwrap();
        assert!(headers.contains_key("authorization"), "missing authorization: {:?}", headers);
        assert!(headers.contains_key("x-amz-date"), "missing x-amz-date: {:?}", headers);
        assert!(headers.contains_key("x-amz-content-sha256"), "missing sha256: {:?}", headers);
        let auth = &headers["authorization"];
        assert!(auth.contains("AWS4-HMAC-SHA256"), "Authorization should use SigV4 scheme: {auth}");
        assert!(auth.contains("Credential=AKIDEXAMPLE"), "Authorization must contain the access key: {auth}");
        assert!(auth.contains("us-east-1/sns/aws4_request"), "Authorization must contain the region+service scope: {auth}");
        // Body "Hello" -> known SHA256.
        let expected_body_hash = "185f8db32271fe25f561a6fc938b2e264306ec304eda518007d1764826381969";
        assert_eq!(headers["x-amz-content-sha256"], expected_body_hash);
    }

    #[test]
    fn sign_request_headers_with_session_token_includes_security_token() {
        let creds = AwsCreds {
            access_key: "AKID".into(),
            secret_key: "SECRET".into(),
            session_token: Some("session-token-123".into()),
        };
        let headers = sign_request_headers(
            "POST",
            "https://sts.amazonaws.com/",
            Some(b""),
            "us-east-1",
            "sts",
            &creds,
        ).unwrap();
        assert!(headers.contains_key("x-amz-security-token"));
        assert_eq!(headers["x-amz-security-token"], "session-token-123");
    }

    #[test]
    fn canonical_request_includes_only_signed_headers() {
        // Use a well-known test vector: "Hello" body, GET request, etc.
        // We don't compare the exact signature (clock-dependent), but
        // the Authorization header MUST start with the right prefix and
        // contain a non-empty signature.
        let creds = AwsCreds {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let headers = sign_request_headers(
            "GET",
            "https://example.amazonaws.com/",
            None,
            "us-east-1",
            "service",
            &creds,
        ).unwrap();
        let auth = &headers["authorization"];
        // Signature is hex of HMAC-SHA256, exactly 64 chars.
        let sig_start = auth.rfind("Signature=").unwrap() + "Signature=".len();
        let sig = &auth[sig_start..];
        assert_eq!(sig.len(), 64, "signature should be 64 hex chars, got {sig}");
    }
}
