//! Contracts of the schema validation method.

use nautilus_protocol::*;
use serde_json::json;

#[test]
fn test_schema_method_name() {
    assert_eq!(SCHEMA_VALIDATE, "schema.validate");
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
fn test_valid_schema_result_omits_the_error_list() {
    let json = serde_json::to_value(&SchemaValidateResult {
        valid: true,
        errors: None,
    })
    .unwrap();

    assert_eq!(json["valid"], true);
    assert!(json.get("errors").is_none());

    let parsed: SchemaValidateResult = serde_json::from_value(json!({"valid": true})).unwrap();
    assert!(parsed.valid);
    assert!(parsed.errors.is_none());
}
