//! Nautilus protocol errors with stable error codes.
//!
//! Error code ranges:
//! - `1000..1999`: Schema / Validation errors
//! - `2000..2999`: Query planning / rendering errors
//! - `3000..3999`: Database execution errors
//! - `9000..9999`: Internal engine errors
//!
//! Standard JSON-RPC errors (negative codes) are reserved for protocol-level issues.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::wire::RpcError;

pub const ERR_SCHEMA_VALIDATION: i32 = 1000;
pub const ERR_INVALID_MODEL: i32 = 1001;
pub const ERR_INVALID_FIELD: i32 = 1002;
pub const ERR_TYPE_MISMATCH: i32 = 1003;

pub const ERR_QUERY_PLANNING: i32 = 2000;
pub const ERR_INVALID_FILTER: i32 = 2001;
pub const ERR_INVALID_ORDERBY: i32 = 2002;
pub const ERR_UNSUPPORTED_OPERATION: i32 = 2003;

pub const ERR_DATABASE_EXECUTION: i32 = 3000;
pub const ERR_CONNECTION_FAILED: i32 = 3001;
pub const ERR_CONSTRAINT_VIOLATION: i32 = 3002;
pub const ERR_QUERY_TIMEOUT: i32 = 3003;
pub const ERR_RECORD_NOT_FOUND: i32 = 3004;
pub const ERR_UNIQUE_CONSTRAINT: i32 = 3005;
pub const ERR_FOREIGN_KEY_CONSTRAINT: i32 = 3006;
pub const ERR_CHECK_CONSTRAINT: i32 = 3007;
pub const ERR_NULL_CONSTRAINT: i32 = 3008;
pub const ERR_DEADLOCK: i32 = 3009;
pub const ERR_SERIALIZATION_FAILURE: i32 = 3010;

pub const ERR_TRANSACTION_NOT_FOUND: i32 = 4001;
pub const ERR_TRANSACTION_TIMEOUT: i32 = 4002;
pub const ERR_TRANSACTION_ALREADY_CLOSED: i32 = 4003;
pub const ERR_TRANSACTION_FAILED: i32 = 4004;

pub const ERR_INTERNAL: i32 = 9000;
pub const ERR_UNSUPPORTED_PROTOCOL_VERSION: i32 = 9001;
pub const ERR_INVALID_METHOD: i32 = 9002;
pub const ERR_INVALID_REQUEST_PARAMS: i32 = 9003;

/// The original error returned by a failed protocol operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolErrorCause {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Structured `error.data` payload emitted when `transaction.batch` fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOperationErrorData {
    pub batch_operation_index: usize,
    pub batch_operation_method: String,
    pub cause: ProtocolErrorCause,
}

/// Nautilus protocol error.
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Schema validation error: {0}")]
    SchemaValidation(String),

    #[error("Invalid model: {0}")]
    InvalidModel(String),

    #[error("Invalid field: {0}")]
    InvalidField(String),

    #[error("Type mismatch: {0}")]
    TypeMismatch(String),

    #[error("Query planning error: {0}")]
    QueryPlanning(String),

    #[error("Invalid filter: {0}")]
    InvalidFilter(String),

    #[error("Invalid order by: {0}")]
    InvalidOrderBy(String),

    #[error("Unsupported operation: {0}")]
    UnsupportedOperation(String),

    #[error("Database execution error: {0}")]
    DatabaseExecution(String),

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Constraint violation: {0}")]
    ConstraintViolation(String),

    #[error("Unique constraint violation: {0}")]
    UniqueConstraintViolation(String),

    #[error("Foreign key constraint violation: {0}")]
    ForeignKeyConstraintViolation(String),

    #[error("Check constraint violation: {0}")]
    CheckConstraintViolation(String),

    #[error("NOT NULL constraint violation: {0}")]
    NullConstraintViolation(String),

    #[error("Deadlock detected: {0}")]
    Deadlock(String),

    #[error("Serialization failure: {0}")]
    SerializationFailure(String),

    #[error("Query timeout: {0}")]
    QueryTimeout(String),

    #[error("Record not found: {0}")]
    RecordNotFound(String),

    #[error("Transaction not found: {0}")]
    TransactionNotFound(String),

    #[error("Transaction timed out: {0}")]
    TransactionTimeout(String),

    #[error("Transaction already closed: {0}")]
    TransactionAlreadyClosed(String),

    #[error("Transaction failed: {0}")]
    TransactionFailed(String),

    #[error("{source}")]
    BatchOperationFailed {
        index: usize,
        method: String,
        #[source]
        source: Box<ProtocolError>,
    },

    #[error("Internal engine error: {0}")]
    Internal(String),

    #[error("Unsupported protocol version: {actual}, expected {expected}")]
    UnsupportedProtocolVersion { actual: u32, expected: u32 },

    #[error("Invalid method: {0}")]
    InvalidMethod(String),

    #[error("Invalid request params: {0}")]
    InvalidParams(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

impl ProtocolError {
    /// Get the error code for this error.
    pub fn code(&self) -> i32 {
        match self {
            ProtocolError::SchemaValidation(_) => ERR_SCHEMA_VALIDATION,
            ProtocolError::InvalidModel(_) => ERR_INVALID_MODEL,
            ProtocolError::InvalidField(_) => ERR_INVALID_FIELD,
            ProtocolError::TypeMismatch(_) => ERR_TYPE_MISMATCH,
            ProtocolError::QueryPlanning(_) => ERR_QUERY_PLANNING,
            ProtocolError::InvalidFilter(_) => ERR_INVALID_FILTER,
            ProtocolError::InvalidOrderBy(_) => ERR_INVALID_ORDERBY,
            ProtocolError::UnsupportedOperation(_) => ERR_UNSUPPORTED_OPERATION,
            ProtocolError::DatabaseExecution(_) => ERR_DATABASE_EXECUTION,
            ProtocolError::ConnectionFailed(_) => ERR_CONNECTION_FAILED,
            ProtocolError::ConstraintViolation(_) => ERR_CONSTRAINT_VIOLATION,
            ProtocolError::UniqueConstraintViolation(_) => ERR_UNIQUE_CONSTRAINT,
            ProtocolError::ForeignKeyConstraintViolation(_) => ERR_FOREIGN_KEY_CONSTRAINT,
            ProtocolError::CheckConstraintViolation(_) => ERR_CHECK_CONSTRAINT,
            ProtocolError::NullConstraintViolation(_) => ERR_NULL_CONSTRAINT,
            ProtocolError::Deadlock(_) => ERR_DEADLOCK,
            ProtocolError::SerializationFailure(_) => ERR_SERIALIZATION_FAILURE,
            ProtocolError::QueryTimeout(_) => ERR_QUERY_TIMEOUT,
            ProtocolError::RecordNotFound(_) => ERR_RECORD_NOT_FOUND,
            ProtocolError::TransactionNotFound(_) => ERR_TRANSACTION_NOT_FOUND,
            ProtocolError::TransactionTimeout(_) => ERR_TRANSACTION_TIMEOUT,
            ProtocolError::TransactionAlreadyClosed(_) => ERR_TRANSACTION_ALREADY_CLOSED,
            ProtocolError::TransactionFailed(_) => ERR_TRANSACTION_FAILED,
            ProtocolError::BatchOperationFailed { source, .. } => source.code(),
            ProtocolError::Internal(_) => ERR_INTERNAL,
            ProtocolError::UnsupportedProtocolVersion { .. } => ERR_UNSUPPORTED_PROTOCOL_VERSION,
            ProtocolError::InvalidMethod(_) => ERR_INVALID_METHOD,
            ProtocolError::InvalidParams(_) => ERR_INVALID_REQUEST_PARAMS,
            ProtocolError::Serialization(_) => ERR_INTERNAL,
        }
    }

    fn rpc_data(&self) -> Option<Value> {
        match self {
            ProtocolError::UnsupportedProtocolVersion { actual, expected } => Some(json!({
                "actual": actual,
                "expected": expected,
            })),
            ProtocolError::BatchOperationFailed {
                index,
                method,
                source,
            } => Some(
                serde_json::to_value(BatchOperationErrorData {
                    batch_operation_index: *index,
                    batch_operation_method: method.clone(),
                    cause: ProtocolErrorCause {
                        code: source.code(),
                        message: source.to_string(),
                        data: source.rpc_data(),
                    },
                })
                .expect("serializing batch-operation error data should succeed"),
            ),
            _ => None,
        }
    }
}

impl From<ProtocolError> for RpcError {
    fn from(err: ProtocolError) -> Self {
        let code = err.code();
        let message = err.to_string();
        let data = err.rpc_data();
        RpcError {
            code,
            message,
            data,
        }
    }
}

impl From<serde_json::Error> for ProtocolError {
    fn from(err: serde_json::Error) -> Self {
        ProtocolError::Serialization(err.to_string())
    }
}

/// Result type alias for protocol operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_error_codes() {
        assert_eq!(
            ProtocolError::SchemaValidation("test".into()).code(),
            ERR_SCHEMA_VALIDATION
        );
        assert_eq!(
            ProtocolError::InvalidModel("User".into()).code(),
            ERR_INVALID_MODEL
        );
        assert_eq!(
            ProtocolError::QueryPlanning("bad query".into()).code(),
            ERR_QUERY_PLANNING
        );
        assert_eq!(
            ProtocolError::DatabaseExecution("timeout".into()).code(),
            ERR_DATABASE_EXECUTION
        );
        assert_eq!(
            ProtocolError::UnsupportedProtocolVersion {
                actual: 2,
                expected: 1
            }
            .code(),
            ERR_UNSUPPORTED_PROTOCOL_VERSION
        );
    }

    #[test]
    fn test_error_to_rpc_error() {
        let err = ProtocolError::InvalidModel("Post".to_string());
        let rpc_err: RpcError = err.into();

        assert_eq!(rpc_err.code, ERR_INVALID_MODEL);
        assert_eq!(rpc_err.message, "Invalid model: Post");
        assert!(rpc_err.data.is_none());
    }

    #[test]
    fn test_batch_operation_error_to_rpc_error_preserves_source_code_and_context() {
        let err = ProtocolError::BatchOperationFailed {
            index: 1,
            method: "query.create".to_string(),
            source: Box::new(ProtocolError::UniqueConstraintViolation(
                "duplicate email".to_string(),
            )),
        };

        let rpc_err: RpcError = err.into();

        assert_eq!(rpc_err.code, ERR_UNIQUE_CONSTRAINT);
        assert_eq!(
            rpc_err.message,
            "Unique constraint violation: duplicate email"
        );

        let data = rpc_err
            .data
            .expect("batch failures should include error.data");
        assert_eq!(data["batchOperationIndex"], 1);
        assert_eq!(data["batchOperationMethod"], "query.create");
        assert_eq!(data["cause"]["code"], ERR_UNIQUE_CONSTRAINT);
        assert_eq!(
            data["cause"]["message"],
            "Unique constraint violation: duplicate email"
        );
    }

    #[test]
    fn test_unsupported_protocol_version_rpc_error_includes_structured_data() {
        let rpc_err: RpcError = ProtocolError::UnsupportedProtocolVersion {
            actual: 2,
            expected: 1,
        }
        .into();

        assert_eq!(rpc_err.code, ERR_UNSUPPORTED_PROTOCOL_VERSION);
        let data = rpc_err
            .data
            .expect("version mismatch should include error.data");
        assert_eq!(data["actual"], 2);
        assert_eq!(data["expected"], 1);
    }

    #[test]
    fn test_error_display() {
        let err = ProtocolError::UnsupportedProtocolVersion {
            actual: 3,
            expected: 1,
        };
        assert_eq!(
            err.to_string(),
            "Unsupported protocol version: 3, expected 1"
        );
    }

    #[test]
    fn test_serde_error_conversion() {
        let bad_json = "{invalid json}";
        let err: serde_json::Error = serde_json::from_str::<Value>(bad_json).unwrap_err();
        let protocol_err: ProtocolError = err.into();

        match protocol_err {
            ProtocolError::Serialization(msg) => {
                assert!(!msg.is_empty());
            }
            _ => panic!("Expected Serialization error"),
        }
    }
}
