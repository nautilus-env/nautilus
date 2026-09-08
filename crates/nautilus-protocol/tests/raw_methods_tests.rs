//! Contracts of the raw SQL methods.

use nautilus_protocol::*;
use serde_json::json;

#[test]
fn test_raw_method_names() {
    assert_eq!(QUERY_RAW, "query.rawQuery");
    assert_eq!(QUERY_RAW_STMT, "query.rawStmtQuery");
}

#[test]
fn test_raw_query_params_serialization() {
    let params = RawQueryParams {
        protocol_version: 1,
        sql: "SELECT 1".to_string(),
        transaction_id: Some("tx-1".to_string()),
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["sql"], "SELECT 1");
    assert_eq!(json["transactionId"], "tx-1");

    let parsed: RawQueryParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.sql, "SELECT 1");
}

#[test]
fn test_raw_stmt_query_params_keep_parameter_order_and_default_to_none() {
    let params: RawStmtQueryParams =
        serde_json::from_value(json!({"protocolVersion": 1, "sql": "SELECT 1"})).unwrap();
    assert!(params.params.is_empty());
    assert!(params.transaction_id.is_none());

    let json = serde_json::to_value(&RawStmtQueryParams {
        protocol_version: 1,
        sql: "SELECT * FROM users WHERE id = ? AND name = ?".to_string(),
        params: vec![json!(7), json!("Alice")],
        transaction_id: None,
    })
    .unwrap();
    assert_eq!(json["params"][0], 7);
    assert_eq!(json["params"][1], "Alice");
    assert!(json.get("transactionId").is_none());
}
