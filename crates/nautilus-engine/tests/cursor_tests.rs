mod common;

use common::{call_rpc_json, parse_ir, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_migrate::{DatabaseProvider, DdlGenerator};
use nautilus_protocol::{PROTOCOL_VERSION, QUERY_COUNT, QUERY_CREATE_MANY, QUERY_FIND_MANY};
use serde_json::{json, Value};

/// The tables of `schema_source`, as suffixes of the name it is given.
const TABLE_SUFFIXES: [&str; 5] = ["", "_tickets", "_readings", "_items", "_order_lines"];

fn schema_source(provider: &str, table: &str) -> String {
    format!(
        r#"
datasource db {{
  provider = "{provider}"
  url = env("DATABASE_URL")
}}

model Token {{
  id    String @id
  label String
  @@map("{table}")
}}

model Ticket {{
  id    Uuid   @id
  label String
  @@map("{table}_tickets")
}}

model Reading {{
  takenAt DateTime @id @map("taken_at")
  label   String
  @@map("{table}_readings")
}}

model Item {{
  id    Int    @id
  label String
  @@map("{table}_items")
}}

model OrderLine {{
  orderId Int    @map("order_id")
  lineNo  Int    @map("line_no")
  label   String
  @@id([orderId, lineNo])
  @@map("{table}_order_lines")
}}
"#
    )
}

async fn create(state: &EngineState, model: &str, rows: Value) {
    call_rpc_json(
        state,
        QUERY_CREATE_MANY,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": model, "data": rows }),
    )
    .await;
}

/// The labels of the page `take` reads from `cursor`, in the order returned.
async fn page(
    state: &EngineState,
    model: &str,
    table: &str,
    cursor: Value,
    take: i32,
) -> Vec<String> {
    let rows = call_rpc_json(
        state,
        QUERY_FIND_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": model,
            "args": { "cursor": cursor, "take": take },
        }),
    )
    .await;
    rows["data"]
        .as_array()
        .expect("rows missing")
        .iter()
        .map(|row| row[format!("{table}__label")].as_str().unwrap().to_string())
        .collect()
}

/// A cursor binds as the type of the key it names. UUID-shaped text on a
/// `String` key used to bind as a UUID: PostgreSQL refused `text >= uuid`, and
/// elsewhere the UUID came back as lowercase hyphenated text, so the page
/// started at the wrong row. A `DateTime` key got text, and a `Uuid` key must
/// still get a UUID.
async fn cursors_bind_as_the_key_type(state: &EngineState, table: &str) {
    // Lowercase as a UUID prints, uppercase, and 32 digits without hyphens.
    let lower = "1aaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let upper = "2BBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB";
    let simple = "4ddddddddddd4ddd8ddddddddddddddd";
    create(
        state,
        "Token",
        json!([
            { "id": lower, "label": "first" },
            { "id": upper, "label": "second" },
            { "id": "3ccccccc-cccc-4ccc-8ccc-cccccccccccc", "label": "third" },
            { "id": simple, "label": "fourth" },
        ]),
    )
    .await;
    assert_eq!(
        page(state, "Token", table, json!({ "id": lower }), 2).await,
        ["first", "second"]
    );
    assert_eq!(
        page(state, "Token", table, json!({ "id": upper }), 2).await,
        ["second", "third"]
    );
    assert_eq!(
        page(state, "Token", table, json!({ "id": simple }), -2).await,
        ["third", "fourth"]
    );

    let tickets = format!("{table}_tickets");
    let ticket = "22222222-2222-4222-8222-222222222222";
    create(
        state,
        "Ticket",
        json!([
            { "id": "11111111-1111-4111-8111-111111111111", "label": "first" },
            { "id": ticket, "label": "second" },
            { "id": "33333333-3333-4333-8333-333333333333", "label": "third" },
        ]),
    )
    .await;
    assert_eq!(
        page(state, "Ticket", &tickets, json!({ "id": ticket }), 2).await,
        ["second", "third"]
    );

    let readings = format!("{table}_readings");
    create(
        state,
        "Reading",
        json!([
            { "takenAt": "2026-10-08T10:00:00Z", "label": "first" },
            { "takenAt": "2026-10-08T11:00:00Z", "label": "second" },
            { "takenAt": "2026-10-08T12:00:00Z", "label": "third" },
        ]),
    )
    .await;
    assert_eq!(
        page(
            state,
            "Reading",
            &readings,
            json!({ "takenAt": "2026-10-08T11:00:00Z" }),
            2
        )
        .await,
        ["second", "third"]
    );
}

/// How many rows `findMany` returns for `args`, and the `count` of the same
/// arguments.
async fn found_and_counted(state: &EngineState, model: &str, args: &Value) -> (usize, usize) {
    let rows = call_rpc_json(
        state,
        QUERY_FIND_MANY,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": model, "args": args }),
    )
    .await;
    let counted = call_rpc_json(
        state,
        QUERY_COUNT,
        json!({ "protocolVersion": PROTOCOL_VERSION, "model": model, "args": args }),
    )
    .await;
    (
        rows["data"].as_array().expect("rows missing").len(),
        counted["count"].as_u64().expect("count missing") as usize,
    )
}

/// `count` counts the rows `findMany` returns for the same `cursor`, `take`
/// and `skip`, forward and backward. It used to drop the cursor and count from
/// the first row: from the third of four rows, `take: 10` found two rows and
/// counted four.
async fn count_follows_the_cursor(state: &EngineState) {
    create(
        state,
        "Item",
        json!([
            { "id": 1, "label": "first" },
            { "id": 2, "label": "second" },
            { "id": 3, "label": "third" },
            { "id": 4, "label": "fourth" },
        ]),
    )
    .await;
    for (args, expected) in [
        (json!({ "cursor": { "id": 3 } }), 2),
        (json!({ "cursor": { "id": 3 }, "take": 10 }), 2),
        (json!({ "cursor": { "id": 3 }, "take": 1 }), 1),
        (json!({ "cursor": { "id": 3 }, "skip": 1 }), 1),
        (json!({ "cursor": { "id": 3 }, "take": 10, "skip": 1 }), 1),
        (json!({ "cursor": { "id": 3 }, "take": -10 }), 3),
        (json!({ "cursor": { "id": 3 }, "take": -2 }), 2),
        (json!({ "cursor": { "id": 3 }, "take": -10, "skip": 1 }), 2),
        (
            json!({ "cursor": { "id": 3 }, "take": 10, "where": { "id": { "lt": 4 } } }),
            1,
        ),
    ] {
        assert_eq!(
            found_and_counted(state, "Item", &args).await,
            (expected, expected),
            "{args}"
        );
    }

    create(
        state,
        "OrderLine",
        json!([
            { "orderId": 1, "lineNo": 1, "label": "first" },
            { "orderId": 1, "lineNo": 2, "label": "second" },
            { "orderId": 2, "lineNo": 1, "label": "third" },
            { "orderId": 2, "lineNo": 2, "label": "fourth" },
        ]),
    )
    .await;
    let cursor = json!({ "orderId": 1, "lineNo": 2 });
    for (args, expected) in [
        (json!({ "cursor": cursor }), 3),
        (json!({ "cursor": cursor, "take": 10 }), 3),
        (json!({ "cursor": cursor, "take": 2, "skip": 2 }), 1),
        (json!({ "cursor": cursor, "take": -10 }), 2),
    ] {
        assert_eq!(
            found_and_counted(state, "OrderLine", &args).await,
            (expected, expected),
            "{args}"
        );
    }
}

/// The scenarios a server database runs, each on tables of its own.
#[derive(Clone, Copy)]
enum Scenario {
    KeyTypes,
    Count,
}

async fn on_server(
    provider: DatabaseProvider,
    schema_provider: &str,
    url: String,
    scenario: Scenario,
) {
    let table = format!("cursor_keys_{}", uuid::Uuid::new_v4().simple());
    let schema = parse_ir(&schema_source(schema_provider, &table));
    let ddl = DdlGenerator::new(provider)
        .generate_create_tables(&schema)
        .unwrap();
    let state = EngineState::new(schema, url, None).await.unwrap();
    state.execute_ddl_sql(ddl).await.unwrap();
    match scenario {
        Scenario::KeyTypes => cursors_bind_as_the_key_type(&state, &table).await,
        Scenario::Count => count_follows_the_cursor(&state).await,
    }
    let quote = if schema_provider == "mysql" { '`' } else { '"' };
    state
        .execute_ddl_sql(
            TABLE_SUFFIXES
                .iter()
                .map(|suffix| format!("DROP TABLE {quote}{table}{suffix}{quote}"))
                .collect(),
        )
        .await
        .unwrap();
}

fn postgres_url() -> String {
    std::env::var("NAUTILUS_TEST_POSTGRES_URL")
        .expect("set NAUTILUS_TEST_POSTGRES_URL for the PostgreSQL cursor tests")
}

fn mysql_url() -> String {
    std::env::var("NAUTILUS_TEST_MYSQL_URL")
        .expect("set NAUTILUS_TEST_MYSQL_URL for the MySQL cursor tests")
}

#[tokio::test]
async fn sqlite_cursors_bind_as_the_key_type() {
    let (state, _dir) = sqlite_state("cursor-keys", &schema_source("sqlite", "tokens")).await;
    cursors_bind_as_the_key_type(&state, "tokens").await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_cursors_bind_as_the_key_type() {
    on_server(
        DatabaseProvider::Postgres,
        "postgresql",
        postgres_url(),
        Scenario::KeyTypes,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires MySQL; set NAUTILUS_TEST_MYSQL_URL"]
async fn mysql_cursors_bind_as_the_key_type() {
    on_server(
        DatabaseProvider::Mysql,
        "mysql",
        mysql_url(),
        Scenario::KeyTypes,
    )
    .await;
}

#[tokio::test]
async fn sqlite_count_follows_the_cursor() {
    let (state, _dir) = sqlite_state("cursor-count", &schema_source("sqlite", "tokens")).await;
    count_follows_the_cursor(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_count_follows_the_cursor() {
    on_server(
        DatabaseProvider::Postgres,
        "postgresql",
        postgres_url(),
        Scenario::Count,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires MySQL; set NAUTILUS_TEST_MYSQL_URL"]
async fn mysql_count_follows_the_cursor() {
    on_server(
        DatabaseProvider::Mysql,
        "mysql",
        mysql_url(),
        Scenario::Count,
    )
    .await;
}
