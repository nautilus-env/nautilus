mod common;

use common::{call_rpc_json, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_protocol::{PROTOCOL_VERSION, QUERY_COUNT, QUERY_CREATE, QUERY_RAW_STMT};
use serde_json::json;

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model Entry {
  id       Int    @id @default(autoincrement()) @map("entry_id")
  slug     String @unique @map("entry_slug")
  title    String @map("entry_title")
  priority Int    @map("priority_value")

  @@map("entries")
}
"#
}

async fn seed_entries(state: &EngineState) {
    for (slug, title, priority) in [
        ("entry-1", "Entry 1", 10),
        ("entry-2", "Entry 2", 20),
        ("entry-3", "Entry 3", 30),
        ("entry-4", "Entry 4", 40),
    ] {
        let _ = call_rpc_json(
            state,
            QUERY_CREATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Entry",
                "data": {
                    "slug": slug,
                    "title": title,
                    "priority": priority
                }
            }),
        )
        .await;
    }
}

#[tokio::test]
async fn count_uses_logical_field_names_and_pagination_window() {
    let (state, temp_dir) = sqlite_state("aggregation-and-raw-sql-tests", schema_source()).await;
    seed_entries(&state).await;

    let counted = call_rpc_json(
        &state,
        QUERY_COUNT,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Entry",
            "args": {
                "where": {
                    "priority": { "gte": 20 }
                },
                "take": 2,
                "skip": 1
            }
        }),
    )
    .await;

    assert_eq!(counted["count"], json!(2));

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn raw_stmt_query_binds_params_and_returns_row_objects() {
    let (state, temp_dir) = sqlite_state("aggregation-and-raw-sql-tests", schema_source()).await;
    seed_entries(&state).await;

    let rows = call_rpc_json(
        &state,
        QUERY_RAW_STMT,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "sql": r#"SELECT "entry_slug" AS slug, "priority_value" AS priority FROM "entries" WHERE "priority_value" > ? ORDER BY "entry_id" ASC"#,
            "params": [20]
        }),
    )
    .await;

    let data = rows["data"]
        .as_array()
        .expect("rawStmtQuery should return a data array");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["slug"], json!("entry-3"));
    assert_eq!(data[0]["priority"], json!(30));
    assert_eq!(data[1]["slug"], json!("entry-4"));
    assert_eq!(data[1]["priority"], json!(40));

    drop(state);
    drop(temp_dir);
}
