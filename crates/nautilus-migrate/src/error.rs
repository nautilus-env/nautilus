//! Migration error types and the crate-level `Result` alias.

/// Result type for migration operations
pub type Result<T> = std::result::Result<T, MigrationError>;

/// Errors that can occur during migration operations
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// Schema parsing or validation error
    #[error("Schema error: {0}")]
    Schema(#[from] nautilus_schema::SchemaError),

    /// Database execution error
    #[error("Database error: {0}")]
    Database(String),

    /// Migration not found
    #[error("Migration not found: {0}")]
    NotFound(String),

    /// Migration already applied
    #[error("Migration already applied: {0}")]
    AlreadyApplied(String),

    /// Migration checksum mismatch
    #[error("Migration checksum mismatch for {name}: expected {expected}, found {found}")]
    ChecksumMismatch {
        /// Migration name
        name: String,
        /// Expected checksum
        expected: String,
        /// Actual checksum
        found: String,
    },

    /// Invalid migration state
    #[error("Invalid migration state: {0}")]
    InvalidState(String),

    /// Validation error
    #[error("Validation error: {0}")]
    ValidationError(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Generic error
    #[error("{0}")]
    Other(String),

    /// A change cannot be applied with the current provider/schema combination.
    #[error("Unsupported change: {0}")]
    UnsupportedChange(String),

    /// A migration stopped part-way and the database kept some of its
    /// statements, so it is neither the old schema nor the new one.
    #[error(
        "Migration '{name}' stopped on `{statement}`: {message}. \
         {committed} of {total} statement(s) are committed and were not rolled back; \
         reconcile the database before retrying"
    )]
    PartiallyApplied {
        /// Migration that stopped.
        name: String,
        /// Statement the database rejected.
        statement: String,
        /// Error the database reported.
        message: String,
        /// Statements whose effect the database kept.
        committed: usize,
        /// Statements the migration carries.
        total: usize,
    },
}

impl From<sqlx::Error> for MigrationError {
    fn from(err: sqlx::Error) -> Self {
        MigrationError::Database(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::MigrationError;

    #[test]
    fn partially_applied_message_is_single_spaced() {
        let error = MigrationError::PartiallyApplied {
            name: "002_unique_email".to_string(),
            statement: "CREATE UNIQUE INDEX \"User_email_key\" ON \"User\" (\"email\")".to_string(),
            message: "duplicate key value".to_string(),
            committed: 1,
            total: 3,
        };

        assert_eq!(
            error.to_string(),
            "Migration '002_unique_email' stopped on \
             `CREATE UNIQUE INDEX \"User_email_key\" ON \"User\" (\"email\")`: duplicate key value. \
             1 of 3 statement(s) are committed and were not rolled back; \
             reconcile the database before retrying"
        );
    }
}
