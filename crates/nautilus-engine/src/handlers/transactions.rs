//! Transaction handlers: start, commit, rollback, batch.

use nautilus_protocol::{
    ProtocolError, RpcId, RpcRequest, TransactionBatchParams, TransactionBatchResult,
    TransactionCommitParams, TransactionCommitResult, TransactionRollbackParams,
    TransactionRollbackResult, TransactionStartParams, TransactionStartResult,
};

use super::dispatch;
use crate::state::EngineState;

/// Handle `transaction.start`.
///
/// Begins a new database transaction with an optional isolation level and timeout.
/// Returns `TransactionStartResult { id }` containing the generated UUID.
pub(super) async fn handle_transaction_start(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: TransactionStartParams = serde_json::from_value(request.params).map_err(|e| {
        ProtocolError::InvalidParams(format!("Invalid transactionStart params: {}", e))
    })?;

    let timeout = std::time::Duration::from_millis(params.timeout_ms.unwrap_or(5000) as u64);

    let tx_id = uuid::Uuid::new_v4().to_string();
    eprintln!("[engine] Starting transaction {}", tx_id);

    state
        .begin_transaction(tx_id.clone(), timeout, params.isolation_level)
        .await?;

    let result = TransactionStartResult { id: tx_id };
    let s = sonic_rs::to_string(&result)
        .map_err(|e| ProtocolError::Internal(format!("Failed to serialize result: {}", e)))?;
    serde_json::value::RawValue::from_string(s)
        .map_err(|e| ProtocolError::Internal(format!("Failed to wrap result: {}", e)))
}

/// Handle `transaction.commit` — commits the transaction identified by `id`.
pub(super) async fn handle_transaction_commit(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: TransactionCommitParams = serde_json::from_value(request.params).map_err(|e| {
        ProtocolError::InvalidParams(format!("Invalid transactionCommit params: {}", e))
    })?;

    eprintln!("[engine] Committing transaction {}", params.id);

    state.commit_transaction(&params.id).await?;

    let result = TransactionCommitResult {};
    let s = sonic_rs::to_string(&result)
        .map_err(|e| ProtocolError::Internal(format!("Failed to serialize result: {}", e)))?;
    serde_json::value::RawValue::from_string(s)
        .map_err(|e| ProtocolError::Internal(format!("Failed to wrap result: {}", e)))
}

/// Handle `transaction.rollback` — rolls back the transaction identified by `id`.
pub(super) async fn handle_transaction_rollback(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: TransactionRollbackParams =
        serde_json::from_value(request.params).map_err(|e| {
            ProtocolError::InvalidParams(format!("Invalid transactionRollback params: {}", e))
        })?;

    eprintln!("[engine] Rolling back transaction {}", params.id);

    state.rollback_transaction(&params.id).await?;

    let result = TransactionRollbackResult {};
    let s = sonic_rs::to_string(&result)
        .map_err(|e| ProtocolError::Internal(format!("Failed to serialize result: {}", e)))?;
    serde_json::value::RawValue::from_string(s)
        .map_err(|e| ProtocolError::Internal(format!("Failed to wrap result: {}", e)))
}

/// Handle `transaction.batch`.
///
/// Runs a sequence of operations inside a single auto-managed transaction.
/// If any operation fails the transaction is rolled back and the error is returned.
/// On success all results are committed and returned in order.
pub(super) async fn handle_transaction_batch(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: TransactionBatchParams = serde_json::from_value(request.params).map_err(|e| {
        ProtocolError::InvalidParams(format!("Invalid transactionBatch params: {}", e))
    })?;

    let timeout = std::time::Duration::from_millis(params.timeout_ms.unwrap_or(5000) as u64);

    let tx_id = uuid::Uuid::new_v4().to_string();
    eprintln!(
        "[engine] Starting batch transaction {} with {} operations",
        tx_id,
        params.operations.len()
    );

    state
        .begin_transaction(tx_id.clone(), timeout, params.isolation_level)
        .await?;

    let mut results: Vec<Box<serde_json::value::RawValue>> = Vec::new();

    for (i, op) in params.operations.iter().enumerate() {
        let mut op_params = op.params.clone();
        if let serde_json::Value::Object(ref mut m) = op_params {
            m.insert("transactionId".into(), serde_json::json!(tx_id));
        }

        let sub_request = RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(RpcId::Number(i as i64)),
            method: op.method.clone(),
            params: op_params,
        };

        match Box::pin(dispatch(state, sub_request)).await {
            Ok(value) => results.push(value),
            Err(e) => {
                eprintln!(
                    "[engine] Batch operation {} failed, rolling back: {:?}",
                    i, e
                );
                let _ = state.rollback_transaction(&tx_id).await;
                return Err(ProtocolError::BatchOperationFailed {
                    index: i,
                    method: op.method.clone(),
                    source: Box::new(e),
                });
            }
        }
    }

    state.commit_transaction(&tx_id).await?;

    let result = TransactionBatchResult { results };
    let s = sonic_rs::to_string(&result)
        .map_err(|e| ProtocolError::Internal(format!("Failed to serialize result: {}", e)))?;
    serde_json::value::RawValue::from_string(s)
        .map_err(|e| ProtocolError::Internal(format!("Failed to wrap result: {}", e)))
}
