//! Contracts of the read methods: the four `find` shapes, `explain`, and the
//! row payload they answer with.

use nautilus_protocol::*;
use serde_json::json;

#[test]
fn test_read_method_names() {
    assert_eq!(QUERY_FIND_MANY, "query.findMany");
    assert_eq!(QUERY_FIND_FIRST, "query.findFirst");
    assert_eq!(QUERY_FIND_UNIQUE, "query.findUnique");
    assert_eq!(QUERY_FIND_UNIQUE_OR_THROW, "query.findUniqueOrThrow");
    assert_eq!(QUERY_FIND_FIRST_OR_THROW, "query.findFirstOrThrow");
    assert_eq!(QUERY_EXPLAIN, "query.explain");
}

#[test]
fn test_find_many_params_serialization() {
    let params = FindManyParams {
        protocol_version: 1,
        model: "User".to_string(),
        args: Some(json!({
            "where": { "email": { "contains": "test" } },
            "take": 10
        })),
        transaction_id: None,
        chunk_size: None,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["model"], "User");
    assert_eq!(json["args"]["take"], 10);
}

#[test]
fn test_find_many_carries_streaming_and_transaction_by_their_wire_names() {
    let params: FindManyParams = serde_json::from_value(json!({
        "protocolVersion": 1,
        "model": "User",
        "transactionId": "tx-1",
        "chunkSize": 500,
    }))
    .unwrap();

    assert_eq!(params.transaction_id.as_deref(), Some("tx-1"));
    assert_eq!(params.chunk_size, Some(500));
}

#[test]
fn test_find_many_rejects_an_unknown_field() {
    let parsed = serde_json::from_value::<FindManyParams>(json!({
        "protocolVersion": 1,
        "model": "User",
        "chunk_size": 500,
    }));

    let message = parsed.unwrap_err().to_string();
    assert!(message.contains("unknown field"), "{message}");
}

#[test]
fn test_find_first_params_serialization() {
    let params = FindFirstParams {
        protocol_version: 1,
        model: "User".to_string(),
        args: Some(json!({"where": {"active": true}})),
        transaction_id: None,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["model"], "User");
    assert_eq!(json["args"]["where"]["active"], true);

    let parsed: FindFirstParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.model, "User");
}

#[test]
fn test_find_unique_params_serialization() {
    let params = FindUniqueParams {
        protocol_version: 1,
        model: "Post".to_string(),
        filter: json!({"id": 42}),
        transaction_id: None,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["model"], "Post");
    assert_eq!(json["filter"]["id"], 42);

    let parsed: FindUniqueParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.model, "Post");
}

#[test]
fn test_or_throw_aliases_are_compatible() {
    let params: FindUniqueOrThrowParams = FindUniqueParams {
        protocol_version: 1,
        model: "User".to_string(),
        filter: json!({"id": 1}),
        transaction_id: None,
    };
    let json = serde_json::to_value(&params).unwrap();
    let _: FindUniqueParams = serde_json::from_value(json).unwrap();

    let params: FindFirstOrThrowParams = FindFirstParams {
        protocol_version: 1,
        model: "User".to_string(),
        args: None,
        transaction_id: None,
    };
    let json = serde_json::to_value(&params).unwrap();
    assert!(json.get("args").is_none());
    let _: FindFirstParams = serde_json::from_value(json).unwrap();
}

#[test]
fn test_explain_params_default_to_not_running_the_statement() {
    let params: ExplainParams = serde_json::from_value(json!({
        "protocolVersion": 1,
        "model": "User",
    }))
    .unwrap();
    assert!(!params.analyze);

    let json = serde_json::to_value(&ExplainParams {
        protocol_version: 1,
        model: "User".to_string(),
        args: Some(json!({"take": 1})),
        analyze: true,
        transaction_id: Some("tx-1".to_string()),
    })
    .unwrap();
    assert_eq!(json["analyze"], true);
    assert_eq!(json["transactionId"], "tx-1");
}

#[test]
fn test_explain_result_serialization() {
    let result = ExplainResult {
        sql: "SELECT 1".to_string(),
        params: vec![json!(1)],
        plan: vec![json!({"detail": "SCAN User"})],
    };

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["sql"], "SELECT 1");
    assert_eq!(json["params"][0], 1);
    assert_eq!(json["plan"][0]["detail"], "SCAN User");
}

#[test]
fn test_query_result_serialization() {
    let result = QueryResult {
        data: vec![
            json!({"id": 1, "name": "Alice"}),
            json!({"id": 2, "name": "Bob"}),
        ],
    };

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["data"][0]["name"], "Alice");
    assert_eq!(json["data"][1]["name"], "Bob");
}
