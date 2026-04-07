mod common;

use common::{call_rpc_json, sqlite_state};
use nautilus_engine::handlers;
use nautilus_protocol::{
    RpcId, RpcRequest, RpcResponse, PROTOCOL_VERSION, QUERY_CREATE, QUERY_FIND_MANY,
};
use serde_json::json;
use tokio::sync::mpsc;

fn parse_result(response: RpcResponse) -> serde_json::Value {
    if let Some(error) = response.error {
        panic!(
            "streaming response failed ({}): {}",
            error.code, error.message
        );
    }

    serde_json::from_str(response.result.expect("missing rpc result").get())
        .expect("failed to parse rpc result")
}

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model User {
  id   Int    @id @default(autoincrement())
  name String
}
"#
}

#[tokio::test]
async fn find_many_chunk_size_emits_partial_responses_in_order() {
    let (state, temp_dir) = sqlite_state("streaming-find-many-tests", schema_source()).await;

    for name in ["Alice", "Bob", "Cara"] {
        let _ = call_rpc_json(
            &state,
            QUERY_CREATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "User",
                "data": {
                    "name": name
                }
            }),
        )
        .await;
    }

    let (tx, mut rx) = mpsc::channel(8);
    let final_response = handlers::handle_request(
        &state,
        RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(RpcId::String("chunked-find-many".to_string())),
            method: QUERY_FIND_MANY.to_string(),
            params: json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "User",
                "args": {
                    "orderBy": [
                        { "id": "asc" }
                    ]
                },
                "chunkSize": 1
            }),
        },
        tx,
    )
    .await;

    let first_partial = rx.recv().await.expect("missing first partial response");
    let second_partial = rx.recv().await.expect("missing second partial response");
    let final_json = parse_result(final_response);
    let first_json = parse_result(first_partial.clone());
    let second_json = parse_result(second_partial.clone());

    assert_eq!(first_partial.partial, Some(true));
    assert_eq!(second_partial.partial, Some(true));
    assert_eq!(
        first_partial.id,
        Some(RpcId::String("chunked-find-many".to_string()))
    );
    assert_eq!(
        second_partial.id,
        Some(RpcId::String("chunked-find-many".to_string()))
    );

    assert_eq!(
        first_json["data"]
            .as_array()
            .expect("first chunk rows")
            .len(),
        1
    );
    assert_eq!(
        second_json["data"]
            .as_array()
            .expect("second chunk rows")
            .len(),
        1
    );
    assert_eq!(
        final_json["data"]
            .as_array()
            .expect("final chunk rows")
            .len(),
        1
    );

    assert_eq!(first_json["data"][0]["User__name"], json!("Alice"));
    assert_eq!(second_json["data"][0]["User__name"], json!("Bob"));
    assert_eq!(final_json["data"][0]["User__name"], json!("Cara"));

    drop(state);
    drop(temp_dir);
}
