mod common;

use common::{call_rpc_json, sqlite_state};
use nautilus_protocol::{PROTOCOL_VERSION, QUERY_CREATE, QUERY_DELETE, QUERY_UPDATE};
use serde_json::json;

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model User {
  id     Int     @id @default(autoincrement())
  email  String  @unique
  active Boolean @default(true)
}
"#
}

async fn create_user(state: &nautilus_engine::EngineState, email: &str) {
    let _ = call_rpc_json(
        state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "data": {
                "email": email
            },
            "returnData": true
        }),
    )
    .await;
}

#[tokio::test]
async fn update_accepts_wrapped_where_filter_payloads() {
    let (state, temp_dir) = sqlite_state("mutation-filter-shape", schema_source()).await;
    create_user(&state, "alice@example.com").await;

    let payload = call_rpc_json(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "filter": {
                "where": {
                    "email": "alice@example.com"
                }
            },
            "data": {
                "active": false
            },
            "returnData": true
        }),
    )
    .await;

    assert_eq!(payload["count"], json!(1));
    assert_eq!(
        payload["data"][0]["User__email"],
        json!("alice@example.com")
    );
    assert_eq!(payload["data"][0]["User__active"], json!(0));

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn delete_accepts_wrapped_where_filter_payloads() {
    let (state, temp_dir) = sqlite_state("mutation-filter-shape", schema_source()).await;
    create_user(&state, "alice@example.com").await;

    let payload = call_rpc_json(
        &state,
        QUERY_DELETE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "filter": {
                "where": {
                    "email": "alice@example.com"
                }
            },
            "returnData": true
        }),
    )
    .await;

    assert_eq!(payload["count"], json!(1));
    assert_eq!(
        payload["data"][0]["User__email"],
        json!("alice@example.com")
    );

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn bulk_update_treats_wrapped_empty_where_as_all_rows() {
    let (state, temp_dir) = sqlite_state("mutation-filter-shape", schema_source()).await;
    create_user(&state, "alice@example.com").await;
    create_user(&state, "bob@example.com").await;

    let payload = call_rpc_json(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "filter": {
                "where": {}
            },
            "data": {
                "active": false
            },
            "returnData": false
        }),
    )
    .await;

    assert_eq!(payload["count"], json!(2));

    drop(state);
    drop(temp_dir);
}
