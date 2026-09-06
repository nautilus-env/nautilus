mod common;

use common::{call_rpc_json, call_rpc_response, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_protocol::{
    PROTOCOL_VERSION, QUERY_CREATE, QUERY_FIND_MANY, QUERY_UPDATE, QUERY_UPDATE_MANY,
};
use serde_json::json;

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model Post {
  id      Int            @id @default(autoincrement())
  title   String
  views   Int            @default(0)
  ratio   Float          @default(1.0)
  balance Decimal(12, 2) @default(0)
  tags    Json?
}
"#
}

async fn seed(state: &EngineState) {
    call_rpc_json(
        state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "data": { "title": "first", "views": 10, "ratio": 2.0, "balance": "10.00" }
        }),
    )
    .await;
}

async fn post(state: &EngineState) -> serde_json::Value {
    let result = call_rpc_json(
        state,
        QUERY_FIND_MANY,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": "Post", "args": {} }),
    )
    .await;
    result["data"][0].clone()
}

#[tokio::test]
async fn every_arithmetic_operator_applies_to_the_stored_value() {
    let (state, temp_dir) = sqlite_state("atomic-update-arithmetic", schema_source()).await;
    seed(&state).await;

    for (operator, operand, expected) in [
        ("increment", json!(5), 15),
        ("decrement", json!(3), 12),
        ("multiply", json!(4), 48),
        ("divide", json!(2), 24),
    ] {
        call_rpc_json(
            &state,
            QUERY_UPDATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Post",
                "filter": { "title": "first" },
                "data": { "views": { operator: operand } }
            }),
        )
        .await;
        assert_eq!(
            post(&state).await["Post__views"],
            json!(expected),
            "{operator} did not apply to the stored value"
        );
    }

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn an_increment_reads_the_row_it_updates() {
    let (state, temp_dir) = sqlite_state("atomic-update-concurrent", schema_source()).await;
    seed(&state).await;

    for _ in 0..3 {
        call_rpc_json(
            &state,
            QUERY_UPDATE_MANY,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Post",
                "filter": {},
                "data": { "views": { "increment": 7 } }
            }),
        )
        .await;
    }

    assert_eq!(post(&state).await["Post__views"], json!(31));

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn set_writes_the_operand_as_given() {
    let (state, temp_dir) = sqlite_state("atomic-update-set", schema_source()).await;
    seed(&state).await;

    call_rpc_json(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "filter": { "title": "first" },
            "data": { "views": { "set": 99 }, "title": { "set": "renamed" } }
        }),
    )
    .await;

    let row = post(&state).await;
    assert_eq!(row["Post__views"], json!(99));
    assert_eq!(row["Post__title"], json!("renamed"));

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn set_is_accepted_on_create_and_arithmetic_is_not() {
    let (state, temp_dir) = sqlite_state("atomic-update-create", schema_source()).await;

    call_rpc_json(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "data": { "title": { "set": "made" }, "views": { "set": 3 } }
        }),
    )
    .await;
    assert_eq!(post(&state).await["Post__views"], json!(3));

    let response = call_rpc_response(
        &state,
        QUERY_CREATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "data": { "title": "second", "views": { "increment": 1 } }
        }),
    )
    .await;
    let error = response.error.expect("increment on create should fail");
    assert!(
        error.message.contains("a create has none"),
        "{}",
        error.message
    );

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn arithmetic_is_refused_on_a_field_that_cannot_take_it() {
    let (state, temp_dir) = sqlite_state("atomic-update-domain", schema_source()).await;
    seed(&state).await;

    let response = call_rpc_response(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "filter": { "title": "first" },
            "data": { "title": { "increment": 1 } }
        }),
    )
    .await;
    let error = response.error.expect("increment on a String should fail");
    assert!(
        error.message.contains("does not support it"),
        "{}",
        error.message
    );

    let response = call_rpc_response(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "filter": { "title": "first" },
            "data": { "id": { "increment": 1 } }
        }),
    )
    .await;
    let error = response.error.expect("increment on the key should fail");
    assert!(
        error.message.contains("primary-key field"),
        "{}",
        error.message
    );

    let response = call_rpc_response(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "filter": { "title": "first" },
            "data": { "views": { "increment": "lots" } }
        }),
    )
    .await;
    let error = response.error.expect("a non-numeric operand should fail");
    assert!(
        error.message.contains("takes a number"),
        "{}",
        error.message
    );

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn a_json_field_keeps_an_operator_shaped_object_as_its_value() {
    let (state, temp_dir) = sqlite_state("atomic-update-json", schema_source()).await;
    seed(&state).await;

    call_rpc_json(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "filter": { "title": "first" },
            "data": { "tags": { "set": ["a", "b"] } }
        }),
    )
    .await;

    assert_eq!(
        post(&state).await["Post__tags"],
        json!({ "set": ["a", "b"] }),
        "a Json field stores the object it was given"
    );

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn float_and_decimal_columns_take_arithmetic_too() {
    let (state, temp_dir) = sqlite_state("atomic-update-numeric", schema_source()).await;
    seed(&state).await;

    call_rpc_json(
        &state,
        QUERY_UPDATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Post",
            "filter": { "title": "first" },
            "data": { "ratio": { "multiply": 2.5 }, "balance": { "increment": 5 } }
        }),
    )
    .await;

    let row = post(&state).await;
    assert_eq!(row["Post__ratio"], json!(5.0));
    // SQLite has no DECIMAL type, so the sum comes back without the column's
    // scale; the providers that do keep it are covered by the scenario.
    assert_eq!(
        row["Post__balance"]
            .as_str()
            .expect("a Decimal reads back as a string")
            .parse::<f64>()
            .expect("and as a number"),
        15.0
    );

    for (operator, operand, expected) in [
        ("increment", "0.25", "15.25"),
        ("decrement", "0.25", "15"),
        ("multiply", "2", "30"),
        ("divide", "2.5", "12"),
    ] {
        call_rpc_json(
            &state,
            QUERY_UPDATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Post",
                "filter": { "title": "first" },
                "data": { "balance": { (operator): operand } }
            }),
        )
        .await;
        let row = post(&state).await;
        assert_eq!(
            row["Post__balance"]
                .as_str()
                .unwrap()
                .parse::<rust_decimal::Decimal>()
                .unwrap(),
            expected.parse::<rust_decimal::Decimal>().unwrap(),
        );
    }

    for (field, operand) in [("balance", "lots"), ("views", "5"), ("ratio", "2.5")] {
        let response = call_rpc_response(
            &state,
            QUERY_UPDATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Post",
                "filter": { "title": "first" },
                "data": { (field): { "increment": operand } }
            }),
        )
        .await;
        let error = response
            .error
            .expect("unsupported string operand should fail");
        assert_eq!(
            error.code,
            nautilus_protocol::ProtocolError::InvalidParams(String::new()).code()
        );
        assert!(error.message.contains("takes a number"));
    }

    drop(state);
    drop(temp_dir);
}
