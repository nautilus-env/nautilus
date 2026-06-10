//! Integration tests for `query.findFirst` / `query.findFirstOrThrow`.
//!
//! These cover the typed delegation into the shared `findMany` path: `take = 1`
//! must be injected both when `args` is present (merged into the existing
//! object) and when it is omitted entirely.

mod common;

use common::{call_rpc_json, call_rpc_response, sqlite_state};
use nautilus_protocol::{
    PROTOCOL_VERSION, QUERY_CREATE, QUERY_FIND_FIRST, QUERY_FIND_FIRST_OR_THROW,
};
use serde_json::json;

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model User {
  id    Int    @id @default(autoincrement())
  name  String
  score Int
}
"#
}

async fn seed_users(state: &nautilus_engine::EngineState) {
    for (name, score) in [("Alice", 10), ("Bob", 30), ("Carol", 20)] {
        let created = call_rpc_json(
            state,
            QUERY_CREATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "User",
                "data": { "name": name, "score": score }
            }),
        )
        .await;
        assert_eq!(created["count"], json!(1));
    }
}

#[tokio::test]
async fn find_first_without_args_returns_single_row() {
    let (state, _temp_dir) = sqlite_state("find-first-tests", schema_source()).await;
    seed_users(&state).await;

    let result = call_rpc_json(
        &state,
        QUERY_FIND_FIRST,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User"
        }),
    )
    .await;

    let data = result["data"].as_array().expect("data should be an array");
    assert_eq!(data.len(), 1, "findFirst must return at most one row");
}

#[tokio::test]
async fn find_first_injects_take_into_existing_args() {
    let (state, _temp_dir) = sqlite_state("find-first-tests", schema_source()).await;
    seed_users(&state).await;

    // The filter matches two users (Bob 30, Carol 20); orderBy picks Bob.
    let result = call_rpc_json(
        &state,
        QUERY_FIND_FIRST,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "args": {
                "where": { "score": { "gt": 15 } },
                "orderBy": [ { "score": "desc" } ]
            }
        }),
    )
    .await;

    let data = result["data"].as_array().expect("data should be an array");
    assert_eq!(data.len(), 1, "findFirst must return at most one row");
    assert_eq!(data[0]["User__name"], json!("Bob"));
}

#[tokio::test]
async fn find_first_returns_empty_data_when_nothing_matches() {
    let (state, _temp_dir) = sqlite_state("find-first-tests", schema_source()).await;
    seed_users(&state).await;

    let result = call_rpc_json(
        &state,
        QUERY_FIND_FIRST,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "args": { "where": { "score": { "gt": 1000 } } }
        }),
    )
    .await;

    let data = result["data"].as_array().expect("data should be an array");
    assert!(data.is_empty(), "no rows should match the filter");
}

#[tokio::test]
async fn find_first_or_throw_errors_when_nothing_matches() {
    let (state, _temp_dir) = sqlite_state("find-first-tests", schema_source()).await;
    seed_users(&state).await;

    let response = call_rpc_response(
        &state,
        QUERY_FIND_FIRST_OR_THROW,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "User",
            "args": { "where": { "score": { "gt": 1000 } } }
        }),
    )
    .await;

    let error = response
        .error
        .expect("findFirstOrThrow on an empty result must error");
    assert!(
        error.message.contains("findFirstOrThrow"),
        "unexpected error message: {}",
        error.message
    );
}

#[tokio::test]
async fn find_first_rejects_unsupported_protocol_version() {
    let (state, _temp_dir) = sqlite_state("find-first-tests", schema_source()).await;

    let response = call_rpc_response(
        &state,
        QUERY_FIND_FIRST,
        json!({
            "protocolVersion": 0,
            "model": "User"
        }),
    )
    .await;

    assert!(
        response.error.is_some(),
        "protocol version 0 must be rejected"
    );
}
