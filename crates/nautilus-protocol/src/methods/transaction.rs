//! Transaction methods: the lifecycle of an interactive transaction, the
//! atomic batch, and the isolation level both accept.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::Value;

/// Start a new interactive transaction.
pub const TRANSACTION_START: &str = "transaction.start";
/// Commit an interactive transaction.
pub const TRANSACTION_COMMIT: &str = "transaction.commit";
/// Rollback an interactive transaction.
pub const TRANSACTION_ROLLBACK: &str = "transaction.rollback";
/// Execute a batch of operations atomically in a single transaction.
pub const TRANSACTION_BATCH: &str = "transaction.batch";

/// Transaction isolation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IsolationLevel {
    /// Read uncommitted — allows dirty reads.
    ReadUncommitted,
    /// Read committed — default for most databases.
    ReadCommitted,
    /// Repeatable read — prevents non-repeatable reads.
    RepeatableRead,
    /// Serializable — strictest isolation level.
    Serializable,
}

impl IsolationLevel {
    /// Returns the SQL representation (e.g., `"READ COMMITTED"`).
    pub fn as_sql(&self) -> &'static str {
        match self {
            IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
            IsolationLevel::ReadCommitted => "READ COMMITTED",
            IsolationLevel::RepeatableRead => "REPEATABLE READ",
            IsolationLevel::Serializable => "SERIALIZABLE",
        }
    }
}

/// Start a new interactive transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStartParams {
    pub protocol_version: u32,
    /// Maximum duration in milliseconds before the transaction is automatically
    /// rolled back. Defaults to 5000 (5 seconds) if omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Optional isolation level override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolation_level: Option<IsolationLevel>,
}

/// Result of starting a transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStartResult {
    /// Unique transaction identifier. Pass this as `transactionId` in
    /// subsequent query requests.
    pub id: String,
}

/// Commit an interactive transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionCommitParams {
    pub protocol_version: u32,
    /// Transaction ID returned by `transaction.start`.
    pub id: String,
}

/// Result of committing a transaction (empty on success).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionCommitResult {}

/// Rollback an interactive transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionRollbackParams {
    pub protocol_version: u32,
    /// Transaction ID returned by `transaction.start`.
    pub id: String,
}

/// Result of rolling back a transaction (empty on success).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRollbackResult {}

/// A single operation inside a batch transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOperation {
    /// JSON-RPC method name (e.g., `"query.create"`).
    pub method: String,
    /// Method-specific params (same shape as the standalone request).
    pub params: Value,
}

/// Execute multiple operations atomically in one transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionBatchParams {
    pub protocol_version: u32,
    /// Ordered list of operations to execute.
    pub operations: Vec<BatchOperation>,
    /// Optional isolation level for the batch transaction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolation_level: Option<IsolationLevel>,
    /// Optional timeout in milliseconds (default: 5000).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Result of a batch transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionBatchResult {
    /// One result per operation, in the same order as the input.
    pub results: Vec<Box<RawValue>>,
}
