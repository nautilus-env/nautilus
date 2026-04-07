//! Diagnostic types for the analysis API.
//!
//! This module defines a stable public contract for errors and warnings
//! that tools (LSP servers, CLI validators, etc.) can consume without
//! depending on the internal [`SchemaError`] representation.

use crate::error::SchemaError;
use crate::span::Span;

/// Severity level of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A hard error that prevents code generation or execution.
    Error,
    /// A warning that should be addressed but doesn't block compilation.
    Warning,
}

/// A single diagnostic message with a source location.
///
/// Spans use byte offsets.  Convert to line/column with
/// [`Span::to_positions`] when needed for display.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Severity of this diagnostic.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
    /// Byte-offset span in the source text.
    pub span: Span,
}

impl Diagnostic {
    /// Create an error-level diagnostic.
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span,
        }
    }

    /// Create a warning-level diagnostic.
    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span,
        }
    }
}

impl From<SchemaError> for Diagnostic {
    fn from(err: SchemaError) -> Self {
        let span = err.span().unwrap_or(Span::single(0));
        let message = err.to_string();
        match err {
            SchemaError::Warning(_, _) => Diagnostic::warning(message, span),
            _ => Diagnostic::error(message, span),
        }
    }
}

impl From<&SchemaError> for Diagnostic {
    fn from(err: &SchemaError) -> Self {
        let span = err.span().unwrap_or(Span::single(0));
        let message = err.to_string();
        match err {
            SchemaError::Warning(_, _) => Diagnostic::warning(message, span),
            _ => Diagnostic::error(message, span),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_schema_error_with_span() {
        let span = Span::new(10, 20);
        let err = SchemaError::Validation("field required".to_string(), span);
        let diag = Diagnostic::from(&err);
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.span, span);
        assert!(diag.message.contains("Validation error"));
    }

    #[test]
    fn from_schema_error_without_span() {
        let err = SchemaError::Other("generic".to_string());
        let diag = Diagnostic::from(err);
        assert_eq!(diag.span, Span::single(0));
    }
}
