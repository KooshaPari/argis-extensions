//! `WebhookTarget` — the destination of an alert payload.
//!
//! Used by both `AlertRule.webhooks` and `MetaAlertRule.webhooks`. Fields
//! are optional where sensible so a webhook target can be built incrementally
//! (e.g. add an AWS region later without re-constructing).

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Where to send an alert payload when a rule fires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebhookTarget {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Static bearer token. Sent as `Authorization: Bearer <token>`.
    /// If both `bearer_token` and `bearer_token_file` are set, file wins.
    #[serde(default)]
    pub bearer_token: Option<String>,
    /// Read the bearer token from a file on every delivery (or every
    /// `bearer_token_refresh_secs`, whichever is sooner). Supports
    /// Kubernetes-mounted secrets that rotate.
    #[serde(default)]
    pub bearer_token_file: Option<std::path::PathBuf>,
    /// When `bearer_token_file` is set, re-read it at most this often.
    /// Default: 30s.
    #[serde(default)]
    pub bearer_token_refresh_secs: Option<u64>,
    /// When set, the request is signed with AWS SigV4 before being sent.
    /// `aws_region` + `aws_service` (e.g. "sns", "events") + credentials.
    /// Useful for SNS / EventBridge / Lambda webhook targets.
    #[serde(default)]
    pub aws_region: Option<String>,
    #[serde(default)]
    pub aws_service: Option<String>,
    /// Inline credentials. If unset, the substrate reads from
    /// `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN`
    /// environment variables.
    #[serde(default)]
    pub aws_access_key_id: Option<String>,
    #[serde(default)]
    pub aws_secret_access_key: Option<String>,
    #[serde(default)]
    pub aws_session_token: Option<String>,
}

impl Default for WebhookTarget {
    fn default() -> Self {
        Self {
            url: String::new(),
            headers: HashMap::new(),
            bearer_token: None,
            bearer_token_file: None,
            bearer_token_refresh_secs: None,
            aws_region: None,
            aws_service: None,
            aws_access_key_id: None,
            aws_secret_access_key: None,
            aws_session_token: None,
        }
    }
}
