mod common;

use common::sqlite_state;
use nautilus_dialect::Sql;
use nautilus_engine::{handlers, EngineState};
use nautilus_protocol::{
    error::{ERR_INVALID_FILTER, ERR_RECORD_NOT_FOUND, ERR_UNIQUE_CONSTRAINT},
    RpcError, RpcId, RpcRequest, PROTOCOL_VERSION, QUERY_CREATE, QUERY_FIND_MANY,
    QUERY_FIND_UNIQUE_OR_THROW, TRANSACTION_BATCH,
};
use serde_json::json;
use tokio::sync::mpsc;

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model User {
  id    Int    @id @default(autoincrement())
  email String @unique
  name  String
}
"#
}

async fn count_users(state: &EngineState) -> usize {
    let sql = Sql {
        text: r#"SELECT "id" FROM "User""#.to_string(),
        params: vec![],
    };
    state
        .execute_query_on(&sql, "count users", None)
        .await
        .expect("count query should succeed")
        .len()
}

async fn transaction_batch_response(
    state: &EngineState,
    operations: serde_json::Value,
) -> nautilus_protocol::RpcResponse {
    let (tx, _rx) = mpsc::channel(4);
    handlers::handle_request(
        state,
        RpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(RpcId::Number(1)),
            method: TRANSACTION_BATCH.to_string(),
            params: json!({
                "protocolVersion": PROTOCOL_VERSION,
                "operations": operations,
            }),
        },
        tx,
    )
    .await
}

fn assert_batch_error_context(error: &RpcError, index: usize, method: &str) {
    let data = error
        .data
        .as_ref()
        .expect("transaction.batch failures should include error.data");
    assert_eq!(data["batchOperationIndex"], index);
    assert_eq!(data["batchOperationMethod"], method);
    assert_eq!(data["cause"]["code"], error.code);
    assert_eq!(data["cause"]["message"], error.message);
}

#[tokio::test]
async fn transaction_batch_preserves_invalid_filter_error_code_and_context() {
    let (state, temp_dir) = sqlite_state("transaction-batch-tests", schema_source()).await;

    let response = transaction_batch_response(
        &state,
        json!([
            {
                "method": QUERY_FIND_MANY,
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "model": "User",
                    "args": {
                        "where": true
                    }
                }
            }
        ]),
    )
    .await;

    let error = response.error.expect("batch should fail");
    assert_eq!(error.code, ERR_INVALID_FILTER);
    assert_batch_error_context(&error, 0, QUERY_FIND_MANY);

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn transaction_batch_preserves_record_not_found_error_code_and_context() {
    let (state, temp_dir) = sqlite_state("transaction-batch-tests", schema_source()).await;

    let response = transaction_batch_response(
        &state,
        json!([
            {
                "method": QUERY_FIND_UNIQUE_OR_THROW,
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "model": "User",
                    "filter": {
                        "id": 999
                    }
                }
            }
        ]),
    )
    .await;

    let error = response.error.expect("batch should fail");
    assert_eq!(error.code, ERR_RECORD_NOT_FOUND);
    assert_batch_error_context(&error, 0, QUERY_FIND_UNIQUE_OR_THROW);

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn transaction_batch_preserves_constraint_error_code_and_rolls_back() {
    let (state, temp_dir) = sqlite_state("transaction-batch-tests", schema_source()).await;

    let response = transaction_batch_response(
        &state,
        json!([
            {
                "method": QUERY_CREATE,
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "model": "User",
                    "data": {
                        "email": "alice@example.com",
                        "name": "Alice"
                    }
                }
            },
            {
                "method": QUERY_CREATE,
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "model": "User",
                    "data": {
                        "email": "alice@example.com",
                        "name": "Duplicate"
                    }
                }
            }
        ]),
    )
    .await;

    let error = response.error.expect("batch should fail");
    assert_eq!(error.code, ERR_UNIQUE_CONSTRAINT);
    assert_batch_error_context(&error, 1, QUERY_CREATE);
    assert_eq!(count_users(&state).await, 0);

    drop(state);
    drop(temp_dir);
}
