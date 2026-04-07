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
}

impl From<sqlx::Error> for MigrationError {
    fn from(err: sqlx::Error) -> Self {
        MigrationError::Database(err.to_string())
    }
}
