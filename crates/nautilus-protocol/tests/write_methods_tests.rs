//! Contracts of the write methods: the row-returning mutations, the count-only
//! many-variants, and the result they share.

use nautilus_protocol::*;
use serde_json::json;

#[test]
fn test_write_method_names() {
    assert_eq!(QUERY_CREATE, "query.create");
    assert_eq!(QUERY_CREATE_MANY, "query.createMany");
    assert_eq!(QUERY_UPDATE, "query.update");
    assert_eq!(QUERY_UPDATE_MANY, "query.updateMany");
    assert_eq!(QUERY_DELETE, "query.delete");
    assert_eq!(QUERY_DELETE_MANY, "query.deleteMany");
    assert_eq!(QUERY_UPSERT, "query.upsert");
}

#[test]
fn test_create_params_serialization() {
    let params = CreateParams {
        protocol_version: 1,
        model: "Post".to_string(),
        data: json!({
            "title": "Hello World",
            "userId": "123"
        }),
        transaction_id: None,
        return_data: true,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["model"], "Post");
    assert_eq!(json["data"]["title"], "Hello World");
}

#[test]
fn test_create_many_params_serialization() {
    let params = CreateManyParams {
        protocol_version: 1,
        model: "User".to_string(),
        data: vec![json!({"name": "Alice"}), json!({"name": "Bob"})],
        transaction_id: None,
        return_data: true,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["model"], "User");
    assert_eq!(json["data"][0]["name"], "Alice");
    assert_eq!(json["data"][1]["name"], "Bob");

    let parsed: CreateManyParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.data.len(), 2);
}

#[test]
fn test_update_params_serialization() {
    let params = UpdateParams {
        protocol_version: 1,
        model: "User".to_string(),
        filter: json!({"id": 1}),
        data: json!({"name": "Updated"}),
        transaction_id: None,
        return_data: true,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["model"], "User");
    assert_eq!(json["filter"]["id"], 1);
    assert_eq!(json["data"]["name"], "Updated");

    let parsed: UpdateParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.model, "User");
}

#[test]
fn test_delete_params_serialization() {
    let params = DeleteParams {
        protocol_version: 1,
        model: "Post".to_string(),
        filter: json!({"id": 99}),
        transaction_id: None,
        return_data: true,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["model"], "Post");
    assert_eq!(json["filter"]["id"], 99);

    let parsed: DeleteParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.model, "Post");
}

#[test]
fn test_upsert_params_serialization() {
    let params = UpsertParams {
        protocol_version: 1,
        model: "User".to_string(),
        filter: json!({"email": "a@b.c"}),
        create: json!({"email": "a@b.c", "name": "Alice"}),
        update: json!({"name": "Alice"}),
        transaction_id: None,
        return_data: true,
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["filter"]["email"], "a@b.c");
    assert_eq!(json["create"]["name"], "Alice");
    assert_eq!(json["update"]["name"], "Alice");

    let parsed: UpsertParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.model, "User");
}

#[test]
fn test_row_returning_mutations_default_to_returning_their_rows() {
    let create: CreateParams =
        serde_json::from_value(json!({"protocolVersion": 1, "model": "User", "data": {}})).unwrap();
    assert!(create.return_data);

    let create_many: CreateManyParams =
        serde_json::from_value(json!({"protocolVersion": 1, "model": "User", "data": []})).unwrap();
    assert!(create_many.return_data);

    let update: UpdateParams = serde_json::from_value(
        json!({"protocolVersion": 1, "model": "User", "filter": {}, "data": {}}),
    )
    .unwrap();
    assert!(update.return_data);

    let delete: DeleteParams =
        serde_json::from_value(json!({"protocolVersion": 1, "model": "User", "filter": {}}))
            .unwrap();
    assert!(delete.return_data);

    let upsert: UpsertParams = serde_json::from_value(
        json!({"protocolVersion": 1, "model": "User", "filter": {}, "create": {}, "update": {}}),
    )
    .unwrap();
    assert!(upsert.return_data);
}

#[test]
fn test_many_variants_accept_return_data_and_leave_it_out_when_unset() {
    let update: UpdateManyParams = serde_json::from_value(json!({
        "protocolVersion": 1,
        "model": "User",
        "filter": {},
        "data": {},
        "returnData": true,
    }))
    .unwrap();
    assert!(update.return_data);

    let delete: DeleteManyParams =
        serde_json::from_value(json!({"protocolVersion": 1, "model": "User", "filter": {}}))
            .unwrap();
    assert!(!delete.return_data);
    let json = serde_json::to_value(&delete).unwrap();
    assert!(json.get("returnData").is_none());
}

#[test]
fn test_mutation_result_serialization() {
    let result = MutationResult {
        count: 5,
        data: Some(vec![json!({"id": 1}), json!({"id": 2})]),
    };

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["count"], 5);
    assert_eq!(json["data"][0]["id"], 1);
}

#[test]
fn test_mutation_result_without_rows_omits_data() {
    let json = serde_json::to_value(&MutationResult {
        count: 3,
        data: None,
    })
    .unwrap();

    assert_eq!(json["count"], 3);
    assert!(json.get("data").is_none());
}
