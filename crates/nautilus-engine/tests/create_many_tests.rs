mod common;

use common::{call_rpc_response, sqlite_state};
use nautilus_protocol::{PROTOCOL_VERSION, QUERY_CREATE_MANY};
use serde_json::json;

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model User {
  id        Int      @id @default(autoincrement())
  email     String
  nickname  String?
  createdAt DateTime @default(now()) @map("created_at")
}
"#
}

fn updated_at_schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model User {
  id        Int      @id @default(autoincrement())
  email     String
  updatedAt DateTime @updatedAt @map("updated_at")
}
"#
}

#[tokio::test]
async fn create_many_rejects_staggered_keys_instead_of_dropping_later_values() {
    let (state, temp_dir) = sqlite_state("create-many-tests", schema_source()).await;

    let response = call_rpc_response(
        &state,
        QUERY_CREATE_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "data": [
                { "email": "alice@example.com" },
                { "email": "bob@example.com", "nickname": "Bobby" }
            ]
        }),
    )
    .await;

    let error = response
        .error
        .expect("createMany should reject staggered key sets");
    assert!(
        error
            .message
            .contains("same key set after omitting server defaults"),
        "unexpected error message: {}",
        error.message
    );
    assert!(
        error.message.contains("nickname"),
        "error should identify the staggered field: {}",
        error.message
    );

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn create_many_treats_null_function_defaults_like_omitted_fields() {
    let (state, temp_dir) = sqlite_state("create-many-tests", schema_source()).await;

    let response = call_rpc_response(
        &state,
        QUERY_CREATE_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "returnData": true,
            "data": [
                { "email": "alice@example.com" },
                { "email": "bob@example.com", "createdAt": null }
            ]
        }),
    )
    .await;

    if let Some(error) = response.error {
        panic!(
            "createMany unexpectedly failed ({}): {}",
            error.code, error.message
        );
    }

    let payload: serde_json::Value =
        serde_json::from_str(response.result.expect("missing rpc result").get())
            .expect("failed to parse rpc result");

    assert_eq!(payload["count"], json!(2));
    let rows = payload["data"]
        .as_array()
        .expect("createMany should return inserted rows");
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert!(
            row["User__created_at"].is_string(),
            "created_at should be populated by the server default: {row:?}"
        );
    }

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn create_many_populates_updated_at_for_missing_and_null_values() {
    let (state, temp_dir) = sqlite_state("create-many-tests", updated_at_schema_source()).await;

    let response = call_rpc_response(
        &state,
        QUERY_CREATE_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "returnData": true,
            "data": [
                { "email": "alice@example.com" },
                { "email": "bob@example.com", "updatedAt": null }
            ]
        }),
    )
    .await;

    if let Some(error) = response.error {
        panic!(
            "createMany unexpectedly failed ({}): {}",
            error.code, error.message
        );
    }

    let payload: serde_json::Value =
        serde_json::from_str(response.result.expect("missing rpc result").get())
            .expect("failed to parse rpc result");

    assert_eq!(payload["count"], json!(2));
    let rows = payload["data"]
        .as_array()
        .expect("createMany should return inserted rows");
    assert_eq!(rows.len(), 2);
    for row in rows {
        let updated_at = row["User__updated_at"]
            .as_str()
            .expect("updated_at should be returned as a string");
        chrono::DateTime::parse_from_rfc3339(updated_at)
            .expect("updated_at should be a valid RFC3339 timestamp");
    }

    drop(state);
    drop(temp_dir);
}
