//! FFI bindings to a vendored slice of `maximhq/bifrost` (Go C-archive).
//!
//! The vendored Go source lives in `go/`. The build script (`build.rs`)
//! compiles it with `go build -buildmode=c-archive` and links the resulting
//! `libbifrost_shim.a` into this crate. Rust calls into the shim via
//! `extern "C"` declarations.
//!
//! ## Wrap-over-hand-roll
//!
//! Per the locked argis-extensions preference (Rust first, wrap libs,
//! hand-roll only core IP), the entire Bifrost FFI surface area is
//! approached by wrapping the Go core as a cdylib/staticlib. This
//! demonstrates the same pattern used by the tokn FFI in OmniRoute PR #305
//! and the argis-monitor FFI in #185-#200.
//!
//! ## What this slice ships
//!
//! A minimal FFI surface (two functions) that demonstrates the build
//! infrastructure. The full Bifrost surface (Provider, Account,
//! CompletionRequest, ...) is large; future slices add bindings
//! incrementally without changing the build.rs / Cargo.toml layout.

#![allow(non_camel_case_types)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

extern "C" {
    /// Returns a heap-allocated C string (caller must NOT free; the
    /// C-archive owns the buffer). Stable for the lifetime of the process.
    fn bifrost_version() -> *const c_char;

    /// Returns the number of LLM provider constants the vendored Bifrost
    /// surface knows about (OpenAI, Anthropic, Gemini, Custom = 4 in the
    /// demo shim; the upstream package has many more).
    fn bifrost_provider_count() -> c_int;

    /// Returns the i-th provider name as a heap-allocated C string
    /// (caller must NOT free; stable for the lifetime of the process).
    /// Index must be < bifrost_provider_count().
    fn bifrost_provider_name(index: c_int) -> *const c_char;

    /// Returns a heap-allocated C string with a sample Bifrost Account
    /// (proves the wrap pattern works against the full upstream package).
    fn bifrost_schema_dump() -> *const c_char;

    /// Real chat completion: returns a JSON blob with the request and
    /// response (echoes the prompt; a real provider would call the API).
    /// The Rust side parses the JSON to extract the response content.
    fn bifrost_chat_completion(model: *const c_char, prompt: *const c_char) -> *const c_char;
}

/// Cached version string. Computed once on first use; the C side keeps
/// the buffer alive for the process lifetime.
pub fn version() -> &'static str {
    use std::sync::OnceLock;
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED.get_or_init(|| {
        // SAFETY: bifrost_version returns a NUL-terminated C string that
        // the Go side guarantees is valid for the process lifetime. The
        // buffer is owned by the Go runtime, not Rust.
        let ptr = unsafe { bifrost_version() };
        if ptr.is_null() {
            return "<unknown>".to_string();
        }
        // SAFETY: ptr is non-null and NUL-terminated; to_string_lossy
        // handles non-UTF-8 defensively.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }).as_str()
}

/// Number of provider constants the vendored Bifrost surface ships.
pub fn provider_count() -> i32 {
    unsafe { bifrost_provider_count() }
}

/// Names of the providers the vendored Bifrost surface ships, sorted.
/// Returns a static Vec (lives for the process lifetime; the underlying
/// C strings are owned by the Go runtime).
pub fn provider_names() -> Vec<&'static str> {
    let n = provider_count();
    (0..n)
        .map(|i| {
            // SAFETY: each entry is a NUL-terminated C string owned by
            // the Go runtime; the lifetime is `'static` for the process.
            let ptr = unsafe { bifrost_provider_name(i as c_int) };
            if ptr.is_null() {
                return "";
            }
            unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("")
        })
        .collect()
}

/// Convenience: returns the version plus the provider count as a
/// formatted string. Useful for startup banners.
pub fn summary() -> String {
    format!("{} ({} providers)", version(), provider_count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_non_empty() {
        let v = version();
        assert!(!v.is_empty(), "version should not be empty, got: {v:?}");
        // Sanity: the vendored shim is "bifrost-ffi vX.Y.Z".
        assert!(v.starts_with("bifrost-ffi"), "got unexpected version: {v}");
    }

    #[test]
    fn provider_count_is_positive() {
        let n = provider_count();
        assert!(n > 0, "provider count should be positive, got: {n}");
        // The vendored shim hardcodes 4 (OpenAI, Anthropic, Gemini, Custom).
        assert_eq!(n, 4, "vendored shim should report 4 providers");
    }

    #[test]
    fn summary_includes_both() {
        let s = summary();
        assert!(s.contains("providers"), "summary should mention 'providers', got: {s}");
        assert!(s.contains("4"), "summary should include the count 4, got: {s}");
    }

    #[test]
    fn summary_is_idempotent() {
        // Calling summary() repeatedly should return the same string each time
        // (the version is cached + the provider count is constant).
        let s1 = summary();
        let s2 = summary();
        assert_eq!(s1, s2);
    }
}

    #[test]
    fn provider_names_returns_n_strings() {
        let names = provider_names();
        assert_eq!(names.len() as i32, provider_count());
        assert!(!names.is_empty());
        // All names should be non-empty.
        for n in &names {
            assert!(!n.is_empty(), "provider name should be non-empty: {n:?}");
        }
    }

/// Returns a small text dump of a sample Bifrost Account (proves the
/// wrap pattern works against the full upstream package, not just
/// our local hand-rolled shim). The Go side constructs a `schemas.Account`
/// and returns a one-line summary.
pub fn schema_dump() -> &'static str {
    use std::sync::OnceLock;
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED.get_or_init(|| {
        // SAFETY: see version() above.
        let ptr = unsafe { bifrost_schema_dump() };
        if ptr.is_null() {
            return "<unknown>".to_string();
        }
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }).as_str()
}
    #[test]
    fn schema_dump_includes_known_account_id() {
        let d = schema_dump();
        // The vendored shim hardcodes ID "acc-001".
        assert!(d.contains("acc-001"), "schema_dump should mention acc-001, got: {d}");
        // The real upstream package's ProviderOpenAI const is "openai".
        assert!(d.contains("openai"), "schema_dump should mention openai, got: {d}");
    }

/// JSON envelope returned by `bifrost_chat_completion`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChatCompletionEnvelope {
    pub request: serde_json::Value,
    pub response: serde_json::Value,
}

impl ChatCompletionEnvelope {
    pub fn content(&self) -> Option<&str> {
        self.response.get("completion_response")
            .and_then(|cr| cr.get("content"))
            .and_then(|c| c.as_str())
    }
}

/// Run a chat completion against the vendored Bifrost (mocked echo provider).
/// The Go side constructs a real `schemas.CompletionRequest` and returns a
/// real `schemas.BifrostResponse` (echo content), serialized as JSON.
pub fn chat_completion(model: &str, prompt: &str) -> Result<ChatCompletionEnvelope, String> {
    let m = CString::new(model).map_err(|e| e.to_string())?;
    let p = CString::new(prompt).map_err(|e| e.to_string())?;
    // SAFETY: the Go side returns a heap-allocated C string that is valid
    // for the lifetime of the process; the buffer is owned by the Go
    // runtime, not Rust.
    let ptr = unsafe { bifrost_chat_completion(m.as_ptr(), p.as_ptr()) };
    if ptr.is_null() {
        return Err("chat_completion returned null".into());
    }
    let raw = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned();
    serde_json::from_str(&raw).map_err(|e| format!("invalid json from FFI: {e}: {raw}"))
}
    #[test]
    fn chat_completion_returns_echo_of_prompt() {
        let env = chat_completion("gpt-4o-mini", "hello world").expect("chat_completion");
        // The request should carry our model + prompt.
        assert_eq!(env.request.get("model").and_then(|v| v.as_str()), Some("gpt-4o-mini"));
        let msgs = env.request.get("messages").and_then(|v| v.as_array()).expect("messages array");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get("role").and_then(|v| v.as_str()), Some("user"));
        assert_eq!(msgs[0].get("content").and_then(|v| v.as_str()), Some("hello world"));
        // The response should echo the prompt.
        assert_eq!(env.content(), Some("echo: hello world"));
    }

    #[test]
    fn chat_completion_preserves_model() {
        let env = chat_completion("claude-opus-4", "ping").expect("chat_completion");
        assert_eq!(env.request.get("model").and_then(|v| v.as_str()), Some("claude-opus-4"));
        assert_eq!(env.content(), Some("echo: ping"));
    }

    #[test]
    fn chat_completion_handles_empty_prompt() {
        let env = chat_completion("gpt-4", "").expect("chat_completion");
        assert_eq!(env.content(), Some("echo: "));
    }
