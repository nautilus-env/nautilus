use nautilus_protocol::*;
use serde_json::json;

#[test]
fn test_handshake_params_serialization() {
    let params = HandshakeParams {
        protocol_version: 1,
        client_name: Some("nautilus-js".to_string()),
        client_version: Some("0.1.0".to_string()),
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["clientName"], "nautilus-js");
    assert_eq!(json["clientVersion"], "0.1.0");

    let parsed: HandshakeParams = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.protocol_version, 1);
    assert_eq!(parsed.client_name, Some("nautilus-js".to_string()));
}

#[test]
fn test_handshake_result_serialization() {
    let result = HandshakeResult {
        engine_version: "0.1.0".to_string(),
        protocol_version: 1,
    };

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["engineVersion"], "0.1.0");
    assert_eq!(json["protocolVersion"], 1);
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
fn test_schema_validate_params() {
    let params = SchemaValidateParams {
        protocol_version: 1,
        schema: "model User { id Int @id }".to_string(),
    };

    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["protocolVersion"], 1);
    assert!(json["schema"].as_str().unwrap().contains("User"));
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
fn test_schema_validate_result_serialization() {
    let result = SchemaValidateResult {
        valid: false,
        errors: Some(vec!["Unknown type 'Foo'".to_string()]),
    };

    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["valid"], false);
    assert_eq!(json["errors"][0], "Unknown type 'Foo'");

    let parsed: SchemaValidateResult = serde_json::from_value(json).unwrap();
    assert!(!parsed.valid);
    assert_eq!(parsed.errors.unwrap().len(), 1);
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
fn test_isolation_level_as_sql() {
    assert_eq!(IsolationLevel::ReadUncommitted.as_sql(), "READ UNCOMMITTED");
    assert_eq!(IsolationLevel::ReadCommitted.as_sql(), "READ COMMITTED");
    assert_eq!(IsolationLevel::RepeatableRead.as_sql(), "REPEATABLE READ");
    assert_eq!(IsolationLevel::Serializable.as_sql(), "SERIALIZABLE");
}

#[test]
fn test_isolation_level_rejects_unknown_snapshot_variant() {
    let parsed = serde_json::from_value::<IsolationLevel>(json!("snapshot"));
    assert!(parsed.is_err());
}

#[test]
fn test_readme_tracks_current_protocol_version() {
    let readme = std::fs::read_to_string(format!("{}/README.md", env!("CARGO_MANIFEST_DIR")))
        .expect("failed to read crate README");

    assert!(readme.contains(&format!("Current version: **{}**", PROTOCOL_VERSION)));
    assert!(readme.contains("All client requests must include `protocolVersion: 1`"));
    assert!(!readme.contains("When protocol version 2 is released:"));
}
