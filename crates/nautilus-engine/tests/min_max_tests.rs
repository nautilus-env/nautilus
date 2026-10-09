//! `MIN` and `MAX` of `Uuid`, `Boolean`, `Char` and `VarChar` fields, through
//! `aggregate`, `groupBy`, the grouped `orderBy` and `having`.
//!
//! PostgreSQL orders `uuid` and `boolean` values but has no `min` or `max` for
//! either, so these aggregates used to fail there with `function min(uuid)
//! does not exist`, while SQLite and MySQL, which store the two types as text
//! and integers, answered. The engine used to refuse `Char` and `VarChar`
//! fields, which the generated `_min` and `_max` inputs list, and must still
//! refuse `Xml` ones, which PostgreSQL cannot aggregate.

mod common;

use common::{call_rpc_json, call_rpc_response, parse_ir, sqlite_state};
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

/// `Char` and `VarChar` exist on PostgreSQL and MySQL, `Xml` on PostgreSQL
/// only.
fn character_schema_source(provider: &str, table: &str) -> String {
    let xml = if provider == "postgresql" {
        "\n  invoice Xml?"
    } else {
        ""
    };
    format!(
        r#"
datasource db {{
  provider = "{provider}"
  url = env("DATABASE_URL")
}}

model Order {{
  id    Int        @id
  code  VarChar(8)
  grade Char(2)?{xml}
  @@map("{table}")
}}
"#
    )
}

async fn min_and_max_of_character_types(state: &EngineState) {
    call_rpc_json(
        state,
        QUERY_CREATE_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Order",
            "data": [
                { "id": 1, "code": "B-200", "grade": "AA" },
                { "id": 2, "code": "A-100", "grade": "BB" },
                { "id": 3, "code": "C-300", "grade": "AA" },
                { "id": 4, "code": "D-400", "grade": null },
            ],
        }),
    )
    .await;

    let both = json!({ "code": true, "grade": true });
    let totals = aggregate(state, json!({ "min": both, "max": both })).await;
    assert_eq!(totals["_min"], json!({ "code": "A-100", "grade": "AA" }));
    assert_eq!(totals["_max"], json!({ "code": "D-400", "grade": "BB" }));

    // `true` asks for every field the aggregate applies to, which an `Xml`
    // field is not.
    let every = aggregate(state, json!({ "min": true, "max": true })).await;
    assert_eq!(
        every["_min"],
        json!({ "id": 1, "code": "A-100", "grade": "AA" })
    );
    assert_eq!(
        every["_max"],
        json!({ "id": 4, "code": "D-400", "grade": "BB" })
    );

    let groups = group_by(
        state,
        json!({
            "by": ["grade"],
            "min": { "code": true },
            "max": { "code": true },
            "orderBy": [{ "_max": { "code": "desc" } }],
        }),
    )
    .await;
    assert_eq!(
        groups,
        json!([
            { "grade": null, "_min": { "code": "D-400" }, "_max": { "code": "D-400" } },
            { "grade": "AA", "_min": { "code": "B-200" }, "_max": { "code": "C-300" } },
            { "grade": "BB", "_min": { "code": "A-100" }, "_max": { "code": "A-100" } },
        ])
    );

    let groups = group_by(
        state,
        json!({ "by": ["grade"], "having": { "_min": { "code": { "lt": "B-000" } } } }),
    )
    .await;
    assert_eq!(groups, json!([{ "grade": "BB" }]));
}

/// PostgreSQL has no `min` or `max` for `xml`, so the engine refuses the field
/// before it reaches the database.
async fn min_and_max_refuse_xml(state: &EngineState) {
    for key in ["min", "max"] {
        let response = call_rpc_response(
            state,
            QUERY_AGGREGATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Order",
                "args": { key: { "invoice": true } },
            }),
        )
        .await;
        let error = response.error.expect("an Xml field should be refused");
        assert!(
            error.message.contains(&format!(
                "{key} needs an orderable field, and 'invoice' is not one"
            )),
            "{}",
            error.message
        );
    }
}

/// An engine on a new table built from `schema_source`, and the statement that
/// drops the table.
async fn on_server(
    provider: DatabaseProvider,
    schema_source: impl Fn(&str) -> String,
    url: String,
) -> (EngineState, String) {
    let table = format!("min_max_{}", uuid::Uuid::new_v4().simple());
    let schema = parse_ir(&schema_source(&table));
    let ddl = DdlGenerator::new(provider)
        .generate_create_tables(&schema)
        .unwrap();
    let state = EngineState::new(schema, url, None).await.unwrap();
    state.execute_ddl_sql(ddl).await.unwrap();
    let quote = if provider == DatabaseProvider::Mysql {
        '`'
    } else {
        '"'
    };
    (state, format!("DROP TABLE {quote}{table}{quote}"))
}

fn postgres_url() -> String {
    std::env::var("NAUTILUS_TEST_POSTGRES_URL")
        .expect("set NAUTILUS_TEST_POSTGRES_URL for the PostgreSQL MIN and MAX tests")
}

fn mysql_url() -> String {
    std::env::var("NAUTILUS_TEST_MYSQL_URL")
        .expect("set NAUTILUS_TEST_MYSQL_URL for the MySQL MIN and MAX tests")
}

#[tokio::test]
async fn sqlite_min_and_max_of_uuid_and_boolean() {
    let (state, _dir) = sqlite_state("min-max", &schema_source("sqlite", "orders")).await;
    min_and_max_of_uuid_and_boolean(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_min_and_max_of_uuid_and_boolean() {
    let (state, drop_table) = on_server(
        DatabaseProvider::Postgres,
        |table| schema_source("postgresql", table),
        postgres_url(),
    )
    .await;
    min_and_max_of_uuid_and_boolean(&state).await;
    state.execute_ddl_sql(vec![drop_table]).await.unwrap();
}

#[tokio::test]
#[ignore = "requires MySQL; set NAUTILUS_TEST_MYSQL_URL"]
async fn mysql_min_and_max_of_uuid_and_boolean() {
    let (state, drop_table) = on_server(
        DatabaseProvider::Mysql,
        |table| schema_source("mysql", table),
        mysql_url(),
    )
    .await;
    min_and_max_of_uuid_and_boolean(&state).await;
    state.execute_ddl_sql(vec![drop_table]).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_min_and_max_of_character_types() {
    let (state, drop_table) = on_server(
        DatabaseProvider::Postgres,
        |table| character_schema_source("postgresql", table),
        postgres_url(),
    )
    .await;
    min_and_max_of_character_types(&state).await;
    min_and_max_refuse_xml(&state).await;
    state.execute_ddl_sql(vec![drop_table]).await.unwrap();
}

#[tokio::test]
#[ignore = "requires MySQL; set NAUTILUS_TEST_MYSQL_URL"]
async fn mysql_min_and_max_of_character_types() {
    let (state, drop_table) = on_server(
        DatabaseProvider::Mysql,
        |table| character_schema_source("mysql", table),
        mysql_url(),
    )
    .await;
    min_and_max_of_character_types(&state).await;
    state.execute_ddl_sql(vec![drop_table]).await.unwrap();
}
