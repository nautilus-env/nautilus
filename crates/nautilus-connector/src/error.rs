//! Error types for the Nautilus connector layer.
//!
//! Runtime execution failures (database errors, connection failures, row-decoding
//! problems) are represented here as [`ConnectorError`], keeping `nautilus-core`
//! free of any dependency on database driver concepts.

use std::fmt;

/// Classification of the underlying `sqlx::Error` discriminant.
///
/// Enables programmatic inspection of the original error category without
/// storing the non-`Clone` `sqlx::Error` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlxErrorKind {
    /// Not originating from sqlx (e.g. logic errors, type mismatches).
    None,
    /// A database-level error (constraint violation, syntax error, etc.).
    Database,
    /// Unique constraint violation (e.g. duplicate key).
    UniqueConstraint,
    /// Foreign key constraint violation.
    ForeignKeyConstraint,
    /// Check constraint violation.
    CheckConstraint,
    /// NOT NULL constraint violation (inserting NULL into a non-nullable column).
    NullConstraint,
    /// Deadlock detected between concurrent transactions.
    Deadlock,
    /// Serialization failure — transaction must be retried.
    SerializationFailure,
    /// I/O error during communication.
    Io,
    /// TLS handshake failure.
    Tls,
    /// Protocol-level error.
    Protocol,
    /// Expected row was not found.
    RowNotFound,
    /// Requested type not found in the database.
    TypeNotFound,
    /// Column index out of bounds.
    ColumnIndexOutOfBounds,
    /// Named column not found.
    ColumnNotFound,
    /// Column decode failure.
    ColumnDecode,
    /// General decode failure.
    Decode,
    /// Connection pool timed out.
    PoolTimedOut,
    /// Connection pool was closed.
    PoolClosed,
    /// A background pool worker crashed.
    WorkerCrashed,
    /// Configuration error.
    Configuration,
}

impl SqlxErrorKind {
    /// Classify a `sqlx::Error` into its corresponding kind.
    pub fn from_sqlx(e: &sqlx::Error) -> Self {
        match e {
            sqlx::Error::Database(db_err) => {
                if db_err.is_unique_violation() {
                    SqlxErrorKind::UniqueConstraint
                } else if db_err.is_foreign_key_violation() {
                    SqlxErrorKind::ForeignKeyConstraint
                } else if db_err.is_check_violation() {
                    SqlxErrorKind::CheckConstraint
                } else {
                    // Detect NOT NULL, deadlock, and serialization failures via SQLState
                    // (PostgreSQL) or error message patterns (MySQL, SQLite).
                    let state = db_err.code();
                    let state = state.as_deref().unwrap_or("");
                    let msg = db_err.message();
                    if state == "23502"
                        || msg.contains("NOT NULL constraint")
                        || msg.contains("not-null constraint")
                        || msg.contains("cannot be null")
                    {
                        SqlxErrorKind::NullConstraint
                    } else if state == "40P01" || msg.to_ascii_lowercase().contains("deadlock") {
                        SqlxErrorKind::Deadlock
                    } else if state == "40001"
                        || msg.to_ascii_lowercase().contains("serialization failure")
                        || msg.to_ascii_lowercase().contains("could not serialize")
                    {
                        SqlxErrorKind::SerializationFailure
                    } else {
                        SqlxErrorKind::Database
                    }
                }
            }
            sqlx::Error::Io(_) => SqlxErrorKind::Io,
            sqlx::Error::Tls(_) => SqlxErrorKind::Tls,
            sqlx::Error::Protocol(_) => SqlxErrorKind::Protocol,
            sqlx::Error::RowNotFound => SqlxErrorKind::RowNotFound,
            sqlx::Error::TypeNotFound { .. } => SqlxErrorKind::TypeNotFound,
            sqlx::Error::ColumnIndexOutOfBounds { .. } => SqlxErrorKind::ColumnIndexOutOfBounds,
            sqlx::Error::ColumnNotFound(_) => SqlxErrorKind::ColumnNotFound,
            sqlx::Error::ColumnDecode { .. } => SqlxErrorKind::ColumnDecode,
            sqlx::Error::Decode(_) => SqlxErrorKind::Decode,
            sqlx::Error::PoolTimedOut => SqlxErrorKind::PoolTimedOut,
            sqlx::Error::PoolClosed => SqlxErrorKind::PoolClosed,
            sqlx::Error::WorkerCrashed => SqlxErrorKind::WorkerCrashed,
            sqlx::Error::Configuration(_) => SqlxErrorKind::Configuration,
            // sqlx::Error is #[non_exhaustive]; new variants default to None
            #[allow(unreachable_patterns)]
            _ => SqlxErrorKind::None,
        }
    }
}

/// Error type for database connector operations.
///
/// This covers everything that can go wrong at *runtime* when talking to a
/// database. Query-building errors are represented by [`nautilus_core::Error`]
/// and can be wrapped via the [`ConnectorError::Core`] variant.
///
/// Each variant that originates from a sqlx error carries a [`SqlxErrorKind`]
/// discriminant for programmatic inspection (e.g. constraint violations vs I/O
/// errors) without storing the non-`Clone` `sqlx::Error` itself.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectorError {
    /// A query was executed successfully but the database returned an error.
    Database(SqlxErrorKind, String),
    /// Could not establish or acquire a database connection.
    Connection(SqlxErrorKind, String),
    /// A row could not be decoded into the expected Rust types.
    RowDecode(SqlxErrorKind, String),
    /// A query-building error originating from `nautilus-core`.
    Core(nautilus_core::Error),
}

impl ConnectorError {
    /// Create a `Database` error from a sqlx error with a context message.
    pub fn database(e: sqlx::Error, context: &str) -> Self {
        ConnectorError::Database(SqlxErrorKind::from_sqlx(&e), format!("{}: {}", context, e))
    }

    /// Create a `Connection` error from a sqlx error with a context message.
    pub fn connection(e: sqlx::Error, context: &str) -> Self {
        ConnectorError::Connection(SqlxErrorKind::from_sqlx(&e), format!("{}: {}", context, e))
    }

    /// Create a `RowDecode` error from a sqlx error with a context message.
    pub fn row_decode(e: sqlx::Error, context: &str) -> Self {
        ConnectorError::RowDecode(SqlxErrorKind::from_sqlx(&e), format!("{}: {}", context, e))
    }

    /// Create a `Database` error from a plain message (no sqlx source).
    pub fn database_msg(msg: impl Into<String>) -> Self {
        ConnectorError::Database(SqlxErrorKind::None, msg.into())
    }

    /// Create a `Connection` error from a plain message (no sqlx source).
    pub fn connection_msg(msg: impl Into<String>) -> Self {
        ConnectorError::Connection(SqlxErrorKind::None, msg.into())
    }

    /// Create a `RowDecode` error from a plain message (no sqlx source).
    pub fn row_decode_msg(msg: impl Into<String>) -> Self {
        ConnectorError::RowDecode(SqlxErrorKind::None, msg.into())
    }

    /// Returns the [`SqlxErrorKind`] for this error, if applicable.
    pub fn sqlx_kind(&self) -> SqlxErrorKind {
        match self {
            ConnectorError::Database(k, _)
            | ConnectorError::Connection(k, _)
            | ConnectorError::RowDecode(k, _) => *k,
            ConnectorError::Core(_) => SqlxErrorKind::None,
        }
    }
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectorError::Database(_, msg) => write!(f, "Database error: {}", msg),
            ConnectorError::Connection(_, msg) => write!(f, "Connection error: {}", msg),
            ConnectorError::RowDecode(_, msg) => write!(f, "Row decode error: {}", msg),
            ConnectorError::Core(e) => write!(f, "Core error: {}", e),
        }
    }
}

impl std::error::Error for ConnectorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConnectorError::Core(e) => Some(e),
            _ => None,
        }
    }
}

impl From<nautilus_core::Error> for ConnectorError {
    fn from(e: nautilus_core::Error) -> Self {
        ConnectorError::Core(e)
    }
}

/// Result type alias for connector operations.
pub type Result<T> = std::result::Result<T, ConnectorError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display() {
        assert_eq!(
            ConnectorError::database_msg("query failed").to_string(),
            "Database error: query failed"
        );
        assert_eq!(
            ConnectorError::connection_msg("refused").to_string(),
            "Connection error: refused"
        );
        assert_eq!(
            ConnectorError::row_decode_msg("invalid bool").to_string(),
            "Row decode error: invalid bool"
        );
    }

    #[test]
    fn test_sqlx_kind() {
        let err = ConnectorError::database_msg("test");
        assert_eq!(err.sqlx_kind(), SqlxErrorKind::None);

        let err = ConnectorError::Database(SqlxErrorKind::PoolTimedOut, "timeout".to_string());
        assert_eq!(err.sqlx_kind(), SqlxErrorKind::PoolTimedOut);
    }

    #[test]
    fn test_from_core_error() {
        let core_err = nautilus_core::Error::InvalidQuery("bad query".to_string());
        let conn_err = ConnectorError::from(core_err.clone());
        assert_eq!(conn_err, ConnectorError::Core(core_err));
        assert!(conn_err.to_string().contains("bad query"));
    }
}
