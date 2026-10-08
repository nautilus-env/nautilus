mod common;

use common::{call_rpc_json, parse_ir, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_migrate::{DatabaseProvider, DdlGenerator};
use nautilus_protocol::{PROTOCOL_VERSION, QUERY_CREATE_MANY, QUERY_FIND_MANY};
use serde_json::{json, Value};

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

async fn on_server(provider: DatabaseProvider, schema_provider: &str, url: String) {
    let table = format!("cursor_keys_{}", uuid::Uuid::new_v4().simple());
    let schema = parse_ir(&schema_source(schema_provider, &table));
    let ddl = DdlGenerator::new(provider)
        .generate_create_tables(&schema)
        .unwrap();
    let state = EngineState::new(schema, url, None).await.unwrap();
    state.execute_ddl_sql(ddl).await.unwrap();
    cursors_bind_as_the_key_type(&state, &table).await;
    let quote = if schema_provider == "mysql" { '`' } else { '"' };
    state
        .execute_ddl_sql(
            [
                table.clone(),
                format!("{table}_tickets"),
                format!("{table}_readings"),
            ]
            .iter()
            .map(|name| format!("DROP TABLE {quote}{name}{quote}"))
            .collect(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn sqlite_cursors_bind_as_the_key_type() {
    let (state, _dir) = sqlite_state("cursor-keys", &schema_source("sqlite", "tokens")).await;
    cursors_bind_as_the_key_type(&state, "tokens").await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_cursors_bind_as_the_key_type() {
    let url = std::env::var("NAUTILUS_TEST_POSTGRES_URL")
        .expect("set NAUTILUS_TEST_POSTGRES_URL for the PostgreSQL cursor test");
    on_server(DatabaseProvider::Postgres, "postgresql", url).await;
}

#[tokio::test]
#[ignore = "requires MySQL; set NAUTILUS_TEST_MYSQL_URL"]
async fn mysql_cursors_bind_as_the_key_type() {
    let url = std::env::var("NAUTILUS_TEST_MYSQL_URL")
        .expect("set NAUTILUS_TEST_MYSQL_URL for the MySQL cursor test");
    on_server(DatabaseProvider::Mysql, "mysql", url).await;
}
