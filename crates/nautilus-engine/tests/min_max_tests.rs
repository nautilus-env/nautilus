//! `MIN` and `MAX` of `Uuid` and `Boolean` fields, through `aggregate`,
//! `groupBy`, the grouped `orderBy` and `having`.
//!
//! PostgreSQL orders `uuid` and `boolean` values but has no `min` or `max` for
//! either, so these aggregates used to fail there with `function min(uuid)
//! does not exist`, while SQLite and MySQL, which store the two types as text
//! and integers, answered.

mod common;

use common::{call_rpc_json, parse_ir, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_migrate::{DatabaseProvider, DdlGenerator};
use nautilus_protocol::{PROTOCOL_VERSION, QUERY_AGGREGATE, QUERY_CREATE_MANY, QUERY_GROUP_BY};
use serde_json::{json, Value};

const SKU_LOW: &str = "0b6f5c3e-2a1d-4c8e-9f7a-1e2d3c4b5a60";
const SKU_MID: &str = "7d9e8f1a-3b2c-4d5e-8f6a-7b8c9d0e1f23";
const SKU_HIGH: &str = "c4a2e9d0-5f3b-4a7c-9e1d-2b8f6a0c3d51";

fn schema_source(provider: &str, table: &str) -> String {
    format!(
        r#"
datasource db {{
  provider = "{provider}"
  url = env("DATABASE_URL")
}}

model Order {{
  id     Int      @id
  region String
  sku    Uuid?
  paid   Boolean?
  @@map("{table}")
}}
"#
    )
}

async fn aggregate(state: &EngineState, args: Value) -> Value {
    let result = call_rpc_json(
        state,
        QUERY_AGGREGATE,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": "Order", "args": args }),
    )
    .await;
    result["data"][0].clone()
}

async fn group_by(state: &EngineState, args: Value) -> Value {
    let result = call_rpc_json(
        state,
        QUERY_GROUP_BY,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": "Order", "args": args }),
    )
    .await;
    result["data"].clone()
}

async fn min_and_max_of_uuid_and_boolean(state: &EngineState) {
    call_rpc_json(
        state,
        QUERY_CREATE_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Order",
            "data": [
                { "id": 1, "region": "north", "sku": SKU_MID, "paid": true },
                { "id": 2, "region": "north", "sku": SKU_LOW, "paid": false },
                { "id": 3, "region": "south", "sku": SKU_HIGH, "paid": true },
                { "id": 4, "region": "south", "sku": null, "paid": null },
                { "id": 5, "region": "west", "sku": null, "paid": null },
            ],
        }),
    )
    .await;

    let both = json!({ "sku": true, "paid": true });
    let totals = aggregate(state, json!({ "min": both, "max": both })).await;
    assert_eq!(totals["_min"], json!({ "sku": SKU_LOW, "paid": false }));
    assert_eq!(totals["_max"], json!({ "sku": SKU_HIGH, "paid": true }));

    // `true` asks for every field the aggregate applies to.
    let every = aggregate(state, json!({ "min": true, "max": true })).await;
    assert_eq!(
        every["_min"],
        json!({ "id": 1, "region": "north", "sku": SKU_LOW, "paid": false })
    );
    assert_eq!(
        every["_max"],
        json!({ "id": 5, "region": "west", "sku": SKU_HIGH, "paid": true })
    );

    let empty = aggregate(
        state,
        json!({ "where": { "region": "west" }, "min": both, "max": both }),
    )
    .await;
    assert_eq!(empty["_min"], json!({ "sku": null, "paid": null }));
    assert_eq!(empty["_max"], json!({ "sku": null, "paid": null }));

    let groups = group_by(
        state,
        json!({
            "by": ["region"],
            "where": { "region": { "in": ["north", "south"] } },
            "min": both,
            "max": both,
            "orderBy": [{ "_max": { "sku": "desc" } }],
        }),
    )
    .await;
    assert_eq!(
        groups,
        json!([
            {
                "region": "south",
                "_min": { "sku": SKU_HIGH, "paid": true },
                "_max": { "sku": SKU_HIGH, "paid": true },
            },
            {
                "region": "north",
                "_min": { "sku": SKU_LOW, "paid": false },
                "_max": { "sku": SKU_MID, "paid": true },
            },
        ])
    );

    let groups = group_by(
        state,
        json!({
            "by": ["region"],
            "where": { "region": { "in": ["north", "south"] } },
            "orderBy": [{ "_min": { "paid": "asc" } }],
        }),
    )
    .await;
    assert_eq!(
        groups,
        json!([{ "region": "north" }, { "region": "south" }])
    );

    // Both spellings of `having`: aggregate first and field first.
    let groups = group_by(
        state,
        json!({ "by": ["region"], "having": { "_min": { "paid": { "eq": false } } } }),
    )
    .await;
    assert_eq!(groups, json!([{ "region": "north" }]));
    let groups = group_by(
        state,
        json!({ "by": ["region"], "having": { "sku": { "_max": { "gt": SKU_MID } } } }),
    )
    .await;
    assert_eq!(groups, json!([{ "region": "south" }]));
}

async fn on_server(provider: DatabaseProvider, schema_provider: &str, url: String) {
    let table = format!("min_max_{}", uuid::Uuid::new_v4().simple());
    let schema = parse_ir(&schema_source(schema_provider, &table));
    let ddl = DdlGenerator::new(provider)
        .generate_create_tables(&schema)
        .unwrap();
    let state = EngineState::new(schema, url, None).await.unwrap();
    state.execute_ddl_sql(ddl).await.unwrap();
    min_and_max_of_uuid_and_boolean(&state).await;
    let quote = if schema_provider == "mysql" { '`' } else { '"' };
    state
        .execute_ddl_sql(vec![format!("DROP TABLE {quote}{table}{quote}")])
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_min_and_max_of_uuid_and_boolean() {
    let (state, _dir) = sqlite_state("min-max", &schema_source("sqlite", "orders")).await;
    min_and_max_of_uuid_and_boolean(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_min_and_max_of_uuid_and_boolean() {
    let url = std::env::var("NAUTILUS_TEST_POSTGRES_URL")
        .expect("set NAUTILUS_TEST_POSTGRES_URL for the PostgreSQL MIN and MAX tests");
    on_server(DatabaseProvider::Postgres, "postgresql", url).await;
}

#[tokio::test]
#[ignore = "requires MySQL; set NAUTILUS_TEST_MYSQL_URL"]
async fn mysql_min_and_max_of_uuid_and_boolean() {
    let url = std::env::var("NAUTILUS_TEST_MYSQL_URL")
        .expect("set NAUTILUS_TEST_MYSQL_URL for the MySQL MIN and MAX tests");
    on_server(DatabaseProvider::Mysql, "mysql", url).await;
}
