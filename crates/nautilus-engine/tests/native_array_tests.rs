//! List fields written and read back: native arrays on PostgreSQL, JSON text
//! on SQLite.
//!
//! On PostgreSQL a `Decimal`, `Uuid` or `DateTime` list used to be bound as
//! `text[]`, which is assigned to no `numeric[]`, `uuid[]` or `timestamp[]`
//! column, and an empty list of any type was bound the same way; reading such a
//! column back failed on its element type. A `String` list holding a
//! UUID-shaped string was written as JSON text, quotes included. An enum list
//! was bound as `text[]` too, which an enum array column does not accept, and
//! read back it failed on its element type. SQLite keeps each list as the JSON
//! the client sent.

mod common;

use common::{call_rpc_json, parse_ir, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_migrate::{DatabaseProvider, DdlGenerator};
use nautilus_protocol::{
    PROTOCOL_VERSION, QUERY_CREATE, QUERY_CREATE_MANY, QUERY_FIND_MANY, QUERY_FIND_UNIQUE,
    QUERY_UPDATE, QUERY_UPSERT,
};
use serde_json::{json, Value};

const PARCEL_A: &str = "0b6f5c3e-2a1d-4c8e-9f7a-1e2d3c4b5a60";
const PARCEL_B: &str = "7d9e8f1a-3b2c-4d5e-8f6a-7b8c9d0e1f23";

/// The schema of one run. On PostgreSQL the enum becomes a type of the database
/// rather than of the table, so each run names its own.
fn schema_source(provider: &str, table: &str, status_enum: &str) -> String {
    // SQLite has no native arrays, so its lists are stored as JSON text.
    let store = if provider == "sqlite" {
        " @store(json)"
    } else {
        ""
    };
    format!(
        r#"
datasource db {{
  provider = "{provider}"
  url = env("DATABASE_URL")
}}

enum {status_enum} {{
  PENDING
  SHIPPED
  DELIVERED
}}

model Order {{
  id         Int              @id
  unitPrices Decimal(10, 2)[] @map("unit_prices"){store}
  parcelIds  Uuid[]           @map("parcel_ids"){store}
  shippedAt  DateTime[]       @map("shipped_at"){store}
  notes      String[]{store}
  history    {status_enum}[]{store}
  @@map("{table}")
}}
"#
    )
}

/// The array columns of one returned row, by field name.
fn arrays(row: &Value, table: &str) -> Value {
    json!({
        "unitPrices": row[format!("{table}__unit_prices")],
        "parcelIds": row[format!("{table}__parcel_ids")],
        "shippedAt": row[format!("{table}__shipped_at")],
        "notes": row[format!("{table}__notes")],
        "history": row[format!("{table}__history")],
    })
}

async fn stored(state: &EngineState, table: &str, id: i32) -> Value {
    let found = call_rpc_json(
        state,
        QUERY_FIND_UNIQUE,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": "Order", "filter": { "id": id } }),
    )
    .await;
    arrays(&found["data"][0], table)
}

async fn arrays_round_trip(state: &EngineState, table: &str) {
    let full = json!({
        "unitPrices": ["1.50", "20.25"],
        "parcelIds": [PARCEL_A, PARCEL_B],
        "shippedAt": ["2026-01-02T03:04:05Z", "2026-02-03T04:05:06Z"],
        "notes": ["fragile", PARCEL_A],
        "history": ["PENDING", "SHIPPED"],
    });
    let empty = json!({
        "unitPrices": [],
        "parcelIds": [],
        "shippedAt": [],
        "notes": [],
        "history": [],
    });

    let mut data = full.clone();
    data["id"] = json!(1);
    let created = call_rpc_json(
        state,
        QUERY_CREATE,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": "Order", "data": data }),
    )
    .await;
    assert_eq!(arrays(&created["data"][0], table), full);
    assert_eq!(stored(state, table, 1).await, full);

    let mut data = empty.clone();
    data["id"] = json!(2);
    let created = call_rpc_json(
        state,
        QUERY_CREATE,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": "Order", "data": data }),
    )
    .await;
    assert_eq!(arrays(&created["data"][0], table), empty);
    assert_eq!(stored(state, table, 2).await, empty);

    for (id, data, expected) in [(1, &empty, &empty), (2, &full, &full)] {
        let updated = call_rpc_json(
            state,
            QUERY_UPDATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Order",
                "filter": { "id": id },
                "data": data,
            }),
        )
        .await;
        assert_eq!(arrays(&updated["data"][0], table), *expected);
        assert_eq!(stored(state, table, id).await, *expected);
    }

    // One statement whose rows hold an empty and a filled list in each column.
    let mut first = full.clone();
    first["id"] = json!(3);
    let mut second = empty.clone();
    second["id"] = json!(4);
    call_rpc_json(
        state,
        QUERY_CREATE_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Order",
            "data": [first, second],
        }),
    )
    .await;
    assert_eq!(stored(state, table, 3).await, full);
    assert_eq!(stored(state, table, 4).await, empty);

    // Row 3 holds filled lists and row 4 empty ones, so each upsert updates an
    // existing row to the other shape.
    for (id, update) in [(3, &empty), (4, &full)] {
        let mut create = empty.clone();
        create["id"] = json!(id);
        let upserted = call_rpc_json(
            state,
            QUERY_UPSERT,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Order",
                "filter": { "id": id },
                "create": create,
                "update": update,
                "returnData": true,
            }),
        )
        .await;
        assert_eq!(arrays(&upserted["data"][0], table), *update);
        assert_eq!(stored(state, table, id).await, *update);
    }

    let rows = call_rpc_json(
        state,
        QUERY_FIND_MANY,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": "Order", "args": {} }),
    )
    .await;
    assert_eq!(rows["data"].as_array().expect("rows missing").len(), 4);
}

#[tokio::test]
async fn sqlite_json_arrays_round_trip() {
    let (state, _dir) = sqlite_state(
        "native-arrays",
        &schema_source("sqlite", "orders", "OrderStatus"),
    )
    .await;
    arrays_round_trip(&state, "orders").await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_native_arrays_round_trip() {
    let url = std::env::var("NAUTILUS_TEST_POSTGRES_URL")
        .expect("set NAUTILUS_TEST_POSTGRES_URL for the PostgreSQL array tests");
    let run = uuid::Uuid::new_v4().simple().to_string();
    let table = format!("native_arrays_{run}");
    let status_enum = format!("OrderStatus{run}");
    let schema = parse_ir(&schema_source("postgresql", &table, &status_enum));
    let ddl = DdlGenerator::new(DatabaseProvider::Postgres)
        .generate_create_tables(&schema)
        .unwrap();
    let state = EngineState::new(schema, url, None).await.unwrap();
    state.execute_ddl_sql(ddl).await.unwrap();
    arrays_round_trip(&state, &table).await;
    state
        .execute_ddl_sql(vec![
            format!("DROP TABLE \"{table}\""),
            format!("DROP TYPE \"{}\"", status_enum.to_lowercase()),
        ])
        .await
        .unwrap();
}
