//! Display contract tests for [`crate::AppError`].
//!
//! `AppError` derives its [`std::fmt::Display`] implementation in `lib.rs`
//! with `thiserror`; keeping this module test-only avoids a competing manual
//! implementation drifting from the canonical five-variant API.

#[cfg(test)]
mod tests {
    use crate::AppError;

    #[test]
    fn display_domain_uses_derived_message() {
        assert_eq!(
            AppError::domain("bad state").to_string(),
            "domain error: bad state"
        );
    }

    #[test]
    fn display_not_found_includes_entity_and_id() {
        let error = AppError::not_found("user", "123");
        assert_eq!(error.to_string(), "not found: user 123");
    }

    #[test]
    fn display_covers_remaining_canonical_variants() {
        assert_eq!(
            AppError::conflict("stale version").to_string(),
            "conflict: stale version"
        );
        assert_eq!(
            AppError::validation("missing name").to_string(),
            "validation error: missing name"
        );
        assert_eq!(
            AppError::storage("database unavailable").to_string(),
            "storage error: database unavailable"
        );
    }
}
