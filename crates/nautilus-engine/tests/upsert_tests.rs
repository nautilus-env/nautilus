mod common;

use common::{call_rpc_json, parse_ir, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_migrate::{DatabaseProvider, DdlGenerator};
use nautilus_protocol::{PROTOCOL_VERSION, QUERY_FIND_MANY, QUERY_UPSERT};
use serde_json::{json, Value};

fn schema_source(provider: &str, table: &str) -> String {
    format!(
        r#"
datasource db {{
  provider = "{provider}"
  url = env("DATABASE_URL")
}}

model Customer {{
  id BigInt @id @default(autoincrement())
  email String @unique @map("customer_email")
  note String
  updatedAt DateTime @updatedAt @map("updated_at")
  @@map("{table}")
}}

model Booking {{
  id Int @id @default(autoincrement())
  room String
  night Int
  note String
  @@unique([room, night])
  @@map("{table}_bookings")
}}
"#
    )
}

async fn upsert(
    state: &EngineState,
    model: &str,
    filter: Value,
    create: Value,
    update: Value,
    return_data: bool,
) -> Value {
    call_rpc_json(
        state,
        QUERY_UPSERT,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": model,
            "filter": filter,
            "create": create,
            "update": update,
            "returnData": return_data,
        }),
    )
    .await
}

async fn empty_update_returns_the_existing_row(state: &EngineState, table: &str) {
    let created = upsert(
        state,
        "Customer",
        json!({"email": "a@example.com"}),
        json!({
            "email": "a@example.com", "note": "original", "updatedAt": "2001-01-01T00:00:00Z",
        }),
        json!({}),
        true,
    )
    .await;
    let existing = upsert(
        state,
        "Customer",
        json!({"email": "a@example.com"}),
        json!({
            "email": "a@example.com", "note": "replacement", "updatedAt": "2020-01-01T00:00:00Z",
        }),
        json!({}),
        true,
    )
    .await;
    assert_eq!(created["count"], json!(1));
    assert_eq!(existing["count"], json!(1));
    assert_eq!(existing["data"], created["data"]);
    assert_eq!(
        existing["data"][0][format!("{table}__note")],
        json!("original")
    );

    let conflict = upsert(
        state,
        "Customer",
        json!({"email": "a@example.com"}),
        json!({
            "email": "a@example.com", "note": "replacement",
        }),
        json!({}),
        false,
    )
    .await;
    assert_eq!(conflict["count"], json!(0));
    let inserted = upsert(
        state,
        "Customer",
        json!({"email": "b@example.com"}),
        json!({
            "email": "b@example.com", "note": "second",
        }),
        json!({}),
        false,
    )
    .await;
    assert_eq!(inserted["count"], json!(1));

    let updated = upsert(
        state,
        "Customer",
        json!({"email": "a@example.com"}),
        json!({
            "email": "a@example.com", "note": "unused",
        }),
        json!({"note": "changed"}),
        true,
    )
    .await;
    assert_eq!(
        updated["data"][0][format!("{table}__id")],
        created["data"][0][format!("{table}__id")]
    );
    assert_eq!(
        updated["data"][0][format!("{table}__note")],
        json!("changed")
    );

    let key = json!({"night": 7, "room": "A"});
    let first = upsert(
        state,
        "Booking",
        key.clone(),
        json!({
            "room": "A", "night": 7, "note": "original",
        }),
        json!({}),
        true,
    )
    .await;
    let second = upsert(
        state,
        "Booking",
        key,
        json!({
            "room": "A", "night": 7, "note": "replacement",
        }),
        json!({}),
        true,
    )
    .await;
    assert_eq!(second["count"], json!(1));
    assert_eq!(second["data"], first["data"]);

    let rows = call_rpc_json(
        state,
        QUERY_FIND_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION, "model": "Booking", "args": {},
        }),
    )
    .await;
    assert_eq!(rows["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn sqlite_empty_update_returns_the_existing_row() {
    let (state, _dir) = sqlite_state("empty-upsert", &schema_source("sqlite", "customers")).await;
    empty_update_returns_the_existing_row(&state, "customers").await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_empty_update_returns_the_existing_row() {
    let url = std::env::var("NAUTILUS_TEST_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("set NAUTILUS_TEST_POSTGRES_URL for the PostgreSQL regression test");
    let table = format!("empty_upsert_{}", uuid::Uuid::new_v4().simple());
    let schema = parse_ir(&schema_source("postgresql", &table));
    let ddl = DdlGenerator::new(DatabaseProvider::Postgres)
        .generate_create_tables(&schema)
        .unwrap();
    let state = EngineState::new(schema, url, None).await.unwrap();
    state.execute_ddl_sql(ddl).await.unwrap();
    empty_update_returns_the_existing_row(&state, &table).await;
    state
        .execute_ddl_sql(vec![
            format!("DROP TABLE \"{table}_bookings\""),
            format!("DROP TABLE \"{table}\""),
        ])
        .await
        .unwrap();
}
