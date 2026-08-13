//! Machine-readable diagnostics for the canonical [`crate::AppError`] API.
//!
//! Each variant has a stable `PHN-*` code for aggregation and a short help
//! message for CLI consumers. The conversion is exhaustive so new
//! `AppError` variants cannot silently fall back to an unrelated code.

use crate::AppError;
use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum AppDiagnostic {
    #[error("domain error: {0}")]
    #[diagnostic(
        code = "PHN-DOM-500",
        help = "Correct the input or state that violates the business rule before retrying."
    )]
    Domain(String),

    #[error("not found: {entity} {id}")]
    #[diagnostic(
        code = "PHN-NOT-404",
        help = "Verify the entity name and identifier, then retry the lookup."
    )]
    NotFound { entity: String, id: String },

    #[error("conflict: {0}")]
    #[diagnostic(
        code = "PHN-CON-409",
        help = "Refresh the resource and retry with the current version."
    )]
    Conflict(String),

    #[error("validation error: {0}")]
    #[diagnostic(
        code = "PHN-VAL-400",
        help = "Correct the invalid input according to the API contract."
    )]
    Validation(String),

    #[error("storage error: {0}")]
    #[diagnostic(
        code = "PHN-STO-500",
        help = "Check the backing storage service and retry when it is available."
    )]
    Storage(String),
}

impl From<&AppError> for AppDiagnostic {
    fn from(error: &AppError) -> Self {
        match error {
            AppError::Domain(message) => Self::Domain(message.clone()),
            AppError::NotFound { entity, id } => Self::NotFound {
                entity: entity.clone(),
                id: id.clone(),
            },
            AppError::Conflict(message) => Self::Conflict(message.clone()),
            AppError::Validation(message) => Self::Validation(message.clone()),
            AppError::Storage(message) => Self::Storage(message.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic_code(error: AppError) -> String {
        let diagnostic = AppDiagnostic::from(&error);
        let code = diagnostic
            .code()
            .expect("every AppDiagnostic variant has a stable code")
            .to_string();
        code
    }

    #[test]
    fn every_canonical_variant_maps_to_a_phn_code() {
        let cases = [
            (AppError::domain("bad state"), "PHN-DOM-500"),
            (AppError::not_found("user", "42"), "PHN-NOT-404"),
            (AppError::conflict("stale version"), "PHN-CON-409"),
            (AppError::validation("missing name"), "PHN-VAL-400"),
            (AppError::storage("database unavailable"), "PHN-STO-500"),
        ];

        for (error, code) in cases {
            assert!(
                diagnostic_code(error) == code,
                "missing diagnostic code {code}"
            );
        }
    }

    #[test]
    fn not_found_diagnostic_preserves_entity_and_id() {
        let error = AppError::not_found("user", "42");
        let diagnostic = AppDiagnostic::from(&error);
        let output = diagnostic.to_string();
        assert!(output.contains("user"));
        assert!(output.contains("42"));
        assert_eq!(diagnostic_code(error), "PHN-NOT-404");
    }
}
