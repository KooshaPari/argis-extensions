//! Alert rules + evaluator.
//!
//! A rule fires when the per-target burn rate (from the ring buffer) crosses
//! `threshold` for `for_secs` consecutive seconds. The first firing posts the
//! alert payload to the configured webhook URL(s); subsequent fires within
//! `cooldown_secs` are dropped (rate-limiting). When `burn_rate` returns below
//! `resolve_threshold`, the alert transitions back to OK and the next firing
//! is allowed immediately (resolve-on-recovery is the standard SRE pattern).
//!
//! Decomposed into focused submodules (per the file-size mandate, slice 26):
//!   - `types`        — `Severity`, `AlertState`, `AlertStateTracker`, `Decision`
//!   - `webhook_target` — `WebhookTarget` struct + `Default` impl
//!   - `rules`        — `AlertRule` + `MetaAlertRule` (structs + `Default` impls)
//!   - `payload`      — `AlertPayload` + `firing`/`resolved`/`meta_alert` constructors
//!   - `serde_mods`   — `Duration` serde helpers (opt + required)
//!   - `evaluate`     — the state-machine evaluator
//!   - `tests`        — unit tests
//!
//! No logic in this file; just module declarations + re-exports.

mod evaluate;
mod payload;
mod rules;
mod serde_mods;
mod tests;
mod types;
mod webhook_target;

pub use evaluate::evaluate;
pub use payload::AlertPayload;
pub use rules::{AlertRule, MetaAlertRule};
pub use serde_mods::{opt_seconds_as_duration, seconds_as_duration};
pub use types::{AlertState, AlertStateTracker, Decision, Severity};
pub use webhook_target::WebhookTarget;
