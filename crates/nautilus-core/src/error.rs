//! Error types for Nautilus core.

/// Error type for Nautilus core.
///
/// These variants cover query-building and type-conversion failures only.
/// Runtime execution errors (database, connection, row decoding) live in
/// `nautilus-connector::ConnectorError`.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    /// Invalid query construction.
    #[error("Invalid query: {0}")]
    InvalidQuery(String),
    /// Missing required field.
    #[error("Missing required field: {0}")]
    MissingField(String),
    /// Type conversion error.
    #[error("Type error: {0}")]
    TypeError(String),
    /// Record not found (used by `*_or_throw` operations).
    #[error("Record not found: {0}")]
    NotFound(String),
    /// Generic error.
    #[error("{0}")]
    Other(String),
}

/// Result type alias.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Error::InvalidQuery("test".to_string());
        assert_eq!(err.to_string(), "Invalid query: test");

        let err = Error::MissingField("name".to_string());
        assert_eq!(err.to_string(), "Missing required field: name");
    }

    #[test]
    fn test_result_type() {
        let ok: Result<i32> = Ok(42);
        assert!(ok.is_ok());

        let err: Result<i32> = Err(Error::Other("failed".to_string()));
        assert!(err.is_err());
    }

    #[test]
    fn test_not_found_and_type_error() {
        let err = Error::NotFound("user with id=99".to_string());
        assert_eq!(err.to_string(), "Record not found: user with id=99");

        let err = Error::TypeError("expected i64, got Bool".to_string());
        assert_eq!(err.to_string(), "Type error: expected i64, got Bool");

        let err = Error::Other("some failure".to_string());
        assert_eq!(err.to_string(), "some failure");

        let err = Error::MissingField("email".to_string());
        assert_eq!(err.to_string(), "Missing required field: email");
    }
}
