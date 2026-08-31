mod common;

use common::{call_rpc_json, call_rpc_response, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_protocol::{
    PROTOCOL_VERSION, QUERY_COUNT, QUERY_CREATE, QUERY_CREATE_MANY, QUERY_DELETE,
    QUERY_DELETE_MANY, QUERY_FIND_MANY, QUERY_UPDATE, QUERY_UPDATE_MANY, QUERY_UPSERT,
};
use serde_json::json;

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model Task {
  id       Int    @id @default(autoincrement())
  slug     String @unique
  status   String
  priority Int

  @@map("tasks")
}

view OpenTask {
  id       Int    @id
  slug     String
  priority Int

  @@map("open_tasks")
}
"#
}

async fn state_with_view() -> (EngineState, tempfile::TempDir) {
    let (state, dir) = sqlite_state("nautilus-view-", schema_source()).await;
    state
        .execute_ddl_sql(vec![
            "CREATE VIEW \"open_tasks\" AS \
             SELECT \"id\", \"slug\", \"priority\" FROM \"tasks\" WHERE \"status\" = 'open'"
                .to_string(),
        ])
        .await
        .expect("failed to create the view");

    for (slug, status, priority) in [
        ("task-1", "open", 10),
        ("task-2", "done", 20),
        ("task-3", "open", 30),
    ] {
        call_rpc_json(
            &state,
            QUERY_CREATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Task",
                "data": { "slug": slug, "status": status, "priority": priority },
                "returnData": false
            }),
        )
        .await;
    }

    (state, dir)
}

#[tokio::test]
async fn a_view_is_readable() {
    let (state, _dir) = state_with_view().await;

    let rows = call_rpc_json(
        &state,
        QUERY_FIND_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "OpenTask",
            "args": { "orderBy": [{ "priority": "asc" }] }
        }),
    )
    .await;

    let data = rows["data"].as_array().expect("rows");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["open_tasks__slug"], "task-1");
    assert_eq!(data[1]["open_tasks__slug"], "task-3");

    let counted = call_rpc_json(
        &state,
        QUERY_COUNT,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "OpenTask",
            "args": {}
        }),
    )
    .await;
    assert_eq!(counted["count"], 2);
}

#[tokio::test]
async fn every_write_method_refuses_a_view() {
    let (state, _dir) = state_with_view().await;

    let writes = [
        (
            QUERY_CREATE,
            json!({ "protocolVersion": PROTOCOL_VERSION, "model": "OpenTask", "data": { "id": 9, "slug": "x", "priority": 1 } }),
        ),
        (
            QUERY_CREATE_MANY,
            json!({ "protocolVersion": PROTOCOL_VERSION, "model": "OpenTask", "data": [{ "id": 9, "slug": "x", "priority": 1 }] }),
        ),
        (
            QUERY_UPDATE,
            json!({ "protocolVersion": PROTOCOL_VERSION, "model": "OpenTask", "data": { "slug": "x" }, "filter": { "id": 1 } }),
        ),
        (
            QUERY_UPDATE_MANY,
            json!({ "protocolVersion": PROTOCOL_VERSION, "model": "OpenTask", "data": { "slug": "x" }, "filter": { "id": 1 } }),
        ),
        (
            QUERY_DELETE,
            json!({ "protocolVersion": PROTOCOL_VERSION, "model": "OpenTask", "filter": { "id": 1 } }),
        ),
        (
            QUERY_DELETE_MANY,
            json!({ "protocolVersion": PROTOCOL_VERSION, "model": "OpenTask", "filter": { "id": 1 } }),
        ),
        (
            QUERY_UPSERT,
            json!({ "protocolVersion": PROTOCOL_VERSION, "model": "OpenTask", "filter": { "id": 1 }, "create": { "id": 1, "slug": "x", "priority": 1 }, "update": { "slug": "x" } }),
        ),
    ];

    for (method, params) in writes {
        let response = call_rpc_response(&state, method, params).await;
        let error = response
            .error
            .unwrap_or_else(|| panic!("{method} on a view should fail"));
        assert!(
            error
                .message
                .contains("'OpenTask' is a view and is read-only"),
            "{method} reported: {}",
            error.message
        );
    }
}
