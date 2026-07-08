//! Bearer token cache for webhook delivery.
//!
//! Resolves the effective bearer token from a `WebhookTarget`:
//! 1. If `bearer_token_file` is set, read the file (with a 30s cache TTL).
//! 2. Else if `bearer_token` is set, use the static value.
//! 3. Else, no token (return None).
//!
//! File-based tokens are read on every call when the cache has expired.
//! This supports Kubernetes-mounted secrets that rotate: the monitor picks
//! up the new token within `refresh_secs` of the rotation, with zero
//! downtime.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::fs;

use crate::alerts::WebhookTarget;

const DEFAULT_REFRESH_SECS: u64 = 30;

#[derive(Debug)]
struct CachedToken {
    value: String,
    fetched_at: Instant,
}

#[derive(Debug, Default)]
pub struct BearerTokenCache {
    inner: Mutex<Option<CachedToken>>,
}

impl BearerTokenCache {
    pub fn new() -> Self { Self::default() }

    /// Resolve the bearer token for `target`. Returns `Some(token)` to be
    /// sent as `Authorization: Bearer <token>`, or `None` if no token is
    /// configured.
    ///
    /// File-based tokens are cached for `refresh_secs`; static tokens are
    /// returned every call.
    pub async fn resolve(&self, target: &WebhookTarget) -> Option<String> {
        if let Some(path) = &target.bearer_token_file {
            let refresh = Duration::from_secs(
                target.bearer_token_refresh_secs.unwrap_or(DEFAULT_REFRESH_SECS)
            );
            if let Some(cached) = self.inner.lock().expect("poisoned").as_ref() {
                if cached.fetched_at.elapsed() < refresh {
                    return Some(cached.value.clone());
                }
            }
            match read_token_file(path).await {
                Ok(value) => {
                    *self.inner.lock().expect("poisoned") = Some(CachedToken {
                        value: value.clone(),
                        fetched_at: Instant::now(),
                    });
                    Some(value)
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "bearer_token_file read failed; skipping delivery");
                    None
                }
            }
        } else {
            target.bearer_token.clone()
        }
    }
}

async fn read_token_file(path: &Path) -> std::io::Result<String> {
    let bytes = fs::read(path).await?;
    // Trim trailing newline + whitespace (k8s mounts often add a final newline).
    let s = String::from_utf8_lossy(&bytes).trim().to_string();
    Ok(s)
}
