//! Contracts of the transaction methods: interactive lifecycle, atomic batch
//! and the isolation level both accept.

use nautilus_protocol::*;
use serde_json::json;

#[test]
fn test_transaction_method_names() {
    assert_eq!(TRANSACTION_START, "transaction.start");
    assert_eq!(TRANSACTION_COMMIT, "transaction.commit");
    assert_eq!(TRANSACTION_ROLLBACK, "transaction.rollback");
    assert_eq!(TRANSACTION_BATCH, "transaction.batch");
}

#[test]
fn test_isolation_level_as_sql() {
    assert_eq!(IsolationLevel::ReadUncommitted.as_sql(), "READ UNCOMMITTED");
    assert_eq!(IsolationLevel::ReadCommitted.as_sql(), "READ COMMITTED");
    assert_eq!(IsolationLevel::RepeatableRead.as_sql(), "REPEATABLE READ");
    assert_eq!(IsolationLevel::Serializable.as_sql(), "SERIALIZABLE");
}

#[test]
fn test_isolation_level_travels_in_camel_case() {
    for (level, wire) in [
        (IsolationLevel::ReadUncommitted, "readUncommitted"),
        (IsolationLevel::ReadCommitted, "readCommitted"),
        (IsolationLevel::RepeatableRead, "repeatableRead"),
        (IsolationLevel::Serializable, "serializable"),
    ] {
        assert_eq!(serde_json::to_value(level).unwrap(), json!(wire));
        assert_eq!(
            serde_json::from_value::<IsolationLevel>(json!(wire)).unwrap(),
            level
        );
    }
}

#[test]
fn test_isolation_level_rejects_unknown_snapshot_variant() {
    let parsed = serde_json::from_value::<IsolationLevel>(json!("snapshot"));
    assert!(parsed.is_err());
}

#[test]
fn test_transaction_start_leaves_timeout_and_isolation_to_the_engine_when_unset() {
    let params: TransactionStartParams =
        serde_json::from_value(json!({"protocolVersion": 1})).unwrap();
    assert!(params.timeout_ms.is_none());
    assert!(params.isolation_level.is_none());

    let json = serde_json::to_value(&params).unwrap();
    assert!(json.get("timeoutMs").is_none());
    assert!(json.get("isolationLevel").is_none());

    let json = serde_json::to_value(&TransactionStartParams {
        protocol_version: 1,
        timeout_ms: Some(2_000),
        isolation_level: Some(IsolationLevel::Serializable),
    })
    .unwrap();
    assert_eq!(json["timeoutMs"], 2_000);
    assert_eq!(json["isolationLevel"], "serializable");
}

#[test]
fn test_transaction_lifecycle_payloads_carry_the_transaction_id() {
    let start: TransactionStartResult = serde_json::from_value(json!({"id": "tx-1"})).unwrap();
    assert_eq!(start.id, "tx-1");

    let commit = serde_json::to_value(&TransactionCommitParams {
        protocol_version: 1,
        id: start.id.clone(),
    })
    .unwrap();
    assert_eq!(commit["id"], "tx-1");

    let rollback = serde_json::to_value(&TransactionRollbackParams {
        protocol_version: 1,
        id: start.id,
    })
    .unwrap();
    assert_eq!(rollback["id"], "tx-1");

    assert_eq!(
        serde_json::to_value(&TransactionCommitResult {}).unwrap(),
        json!({})
    );
    assert_eq!(
        serde_json::to_value(&TransactionRollbackResult {}).unwrap(),
        json!({})
    );
}

#[test]
fn test_batch_operations_keep_their_order_and_method_specific_params() {
    let params = TransactionBatchParams {
        protocol_version: 1,
        operations: vec![
            BatchOperation {
                method: QUERY_CREATE.to_string(),
                params: json!({"protocolVersion": 1, "model": "User", "data": {"name": "Alice"}}),
            },
            BatchOperation {
                method: QUERY_DELETE_MANY.to_string(),
                params: json!({"protocolVersion": 1, "model": "Post", "filter": {}}),
            },
        ],
        isolation_level: Some(IsolationLevel::RepeatableRead),
        timeout_ms: None,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["operations"][0]["method"], "query.create");
    assert_eq!(json["operations"][0]["params"]["data"]["name"], "Alice");
    assert_eq!(json["operations"][1]["method"], "query.deleteMany");
    assert_eq!(json["isolationLevel"], "repeatableRead");
    assert!(json.get("timeoutMs").is_none());

    let parsed: TransactionBatchParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.operations.len(), 2);
    assert_eq!(parsed.operations[1].method, QUERY_DELETE_MANY);
}

#[test]
fn test_batch_result_answers_one_raw_result_per_operation_in_order() {
    let result: TransactionBatchResult = serde_json::from_value(json!({
        "results": [{"count": 1}, {"data": [{"id": 2}]}],
    }))
    .unwrap();

    assert_eq!(result.results.len(), 2);
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["results"][0]["count"], 1);
    assert_eq!(json["results"][1]["data"][0]["id"], 2);
}
