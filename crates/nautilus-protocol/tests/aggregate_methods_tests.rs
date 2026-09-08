//! Contracts of the aggregate methods: count, groupBy and aggregate.

use nautilus_protocol::*;
use serde_json::json;

#[test]
fn test_aggregate_method_names() {
    assert_eq!(QUERY_COUNT, "query.count");
    assert_eq!(QUERY_GROUP_BY, "query.groupBy");
    assert_eq!(QUERY_AGGREGATE, "query.aggregate");
}

#[test]
fn test_count_params_serialization() {
    let params = CountParams {
        protocol_version: 1,
        model: "User".to_string(),
        args: Some(json!({"where": {"active": true}})),
        transaction_id: Some("tx-1".to_string()),
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["model"], "User");
    assert_eq!(json["args"]["where"]["active"], true);
    assert_eq!(json["transactionId"], "tx-1");

    let parsed: CountParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.transaction_id.as_deref(), Some("tx-1"));
}

#[test]
fn test_group_by_params_carry_their_arguments_untouched() {
    let args = json!({
        "by": ["role"],
        "where": {"active": true},
        "having": {"count": {"id": {"gt": 1}}},
        "orderBy": [{"role": "asc"}],
        "count": {"id": true},
    });
    let params = GroupByParams {
        protocol_version: 1,
        model: "User".to_string(),
        args: Some(args.clone()),
        transaction_id: None,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["args"], args);
    assert!(json.get("transactionId").is_none());
}

#[test]
fn test_aggregate_params_omit_absent_arguments() {
    let params = AggregateParams {
        protocol_version: 1,
        model: "Post".to_string(),
        args: None,
        transaction_id: None,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["model"], "Post");
    assert!(json.get("args").is_none());

    let parsed: AggregateParams = serde_json::from_value(json).unwrap();
    assert!(parsed.args.is_none());
}

#[test]
fn test_aggregate_params_reject_an_unknown_field() {
    let parsed = serde_json::from_value::<AggregateParams>(json!({
        "protocolVersion": 1,
        "model": "Post",
        "by": ["role"],
    }));

    let message = parsed.unwrap_err().to_string();
    assert!(message.contains("unknown field"), "{message}");
}
