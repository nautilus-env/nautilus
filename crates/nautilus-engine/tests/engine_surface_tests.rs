mod common;

use common::{call_rpc_json, call_rpc_response, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_protocol::{
    ENGINE_METRICS, PROTOCOL_VERSION, QUERY_AGGREGATE, QUERY_COUNT, QUERY_CREATE,
    QUERY_DELETE_MANY, QUERY_EXPLAIN, QUERY_UPDATE_MANY,
};
use serde_json::json;

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model Task {
  id       Int    @id @default(autoincrement()) @map("task_id")
  slug     String @unique @map("task_slug")
  status   String @map("task_status")
  priority Int    @map("priority_value")

  @@map("tasks")
}
"#
}

async fn seed_tasks(state: &EngineState) {
    for (slug, status, priority) in [
        ("task-1", "open", 10),
        ("task-2", "open", 20),
        ("task-3", "done", 30),
        ("task-4", "done", 40),
    ] {
        call_rpc_json(
            state,
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
}

async fn count_with_status(state: &EngineState, status: &str) -> u64 {
    let counted = call_rpc_json(
        state,
        QUERY_COUNT,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Task",
            "args": { "where": { "status": status } }
        }),
    )
    .await;

    counted["count"].as_u64().expect("count is a number")
}

#[tokio::test]
async fn update_many_reports_affected_rows_without_returning_them() {
    let (state, temp_dir) = sqlite_state("engine-surface-update-many", schema_source()).await;
    seed_tasks(&state).await;

    let updated = call_rpc_json(
        &state,
        QUERY_UPDATE_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Task",
            "filter": { "where": { "status": "open" } },
            "data": { "status": "archived" }
        }),
    )
    .await;

    assert_eq!(updated["count"], json!(2));
    assert!(
        updated.get("data").is_none(),
        "updateMany must not return rows: {updated}"
    );
    assert_eq!(count_with_status(&state, "archived").await, 2);
    assert_eq!(count_with_status(&state, "done").await, 2);
    assert_eq!(count_with_status(&state, "open").await, 0);

    drop(temp_dir);
}

#[tokio::test]
async fn update_many_with_an_empty_filter_touches_every_row() {
    let (state, temp_dir) = sqlite_state("engine-surface-update-all", schema_source()).await;
    seed_tasks(&state).await;

    let updated = call_rpc_json(
        &state,
        QUERY_UPDATE_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Task",
            "filter": { "where": {} },
            "data": { "status": "closed" }
        }),
    )
    .await;

    assert_eq!(updated["count"], json!(4));
    assert_eq!(count_with_status(&state, "closed").await, 4);

    drop(temp_dir);
}

#[tokio::test]
async fn delete_many_reports_affected_rows() {
    let (state, temp_dir) = sqlite_state("engine-surface-delete-many", schema_source()).await;
    seed_tasks(&state).await;

    let deleted = call_rpc_json(
        &state,
        QUERY_DELETE_MANY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Task",
            "filter": { "where": { "priority": { "gte": 30 } } }
        }),
    )
    .await;

    assert_eq!(deleted["count"], json!(2));
    assert!(deleted.get("data").is_none());
    assert_eq!(count_with_status(&state, "open").await, 2);
    assert_eq!(count_with_status(&state, "done").await, 0);

    drop(temp_dir);
}

#[tokio::test]
async fn aggregate_computes_one_row_over_the_filtered_set() {
    let (state, temp_dir) = sqlite_state("engine-surface-aggregate", schema_source()).await;
    seed_tasks(&state).await;

    let aggregated = call_rpc_json(
        &state,
        QUERY_AGGREGATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Task",
            "args": {
                "where": { "priority": { "gte": 20 } },
                "count": { "_all": true },
                "avg": { "priority": true },
                "sum": { "priority": true },
                "min": { "priority": true },
                "max": { "priority": true }
            }
        }),
    )
    .await;

    let rows = aggregated["data"].as_array().expect("aggregate data array");
    assert_eq!(rows.len(), 1, "aggregate returns exactly one row: {rows:?}");

    let row = &rows[0];
    assert_eq!(row["_count"]["_all"], json!(3));
    assert_eq!(row["_sum"]["priority"], json!(90));
    assert_eq!(row["_min"]["priority"], json!(20));
    assert_eq!(row["_max"]["priority"], json!(40));
    assert_eq!(row["_avg"]["priority"].as_f64(), Some(30.0));

    drop(temp_dir);
}

#[tokio::test]
async fn aggregate_over_an_empty_match_still_returns_a_row() {
    let (state, temp_dir) = sqlite_state("engine-surface-aggregate-empty", schema_source()).await;
    seed_tasks(&state).await;

    let aggregated = call_rpc_json(
        &state,
        QUERY_AGGREGATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Task",
            "args": {
                "where": { "status": "missing" },
                "count": { "_all": true },
                "sum": { "priority": true }
            }
        }),
    )
    .await;

    let row = &aggregated["data"].as_array().expect("aggregate data")[0];
    assert_eq!(row["_count"]["_all"], json!(0));
    assert_eq!(row["_sum"]["priority"], json!(null));

    drop(temp_dir);
}

#[tokio::test]
async fn aggregate_without_any_aggregate_argument_is_rejected() {
    let (state, temp_dir) = sqlite_state("engine-surface-aggregate-bare", schema_source()).await;

    let response = call_rpc_response(
        &state,
        QUERY_AGGREGATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Task",
            "args": { "where": {} }
        }),
    )
    .await;

    let error = response.error.expect("bare aggregate must fail");
    assert!(
        error.message.contains("at least one of count"),
        "unexpected message: {}",
        error.message
    );

    drop(temp_dir);
}

#[tokio::test]
async fn explain_returns_the_rendered_statement_and_a_plan() {
    let (state, temp_dir) = sqlite_state("engine-surface-explain", schema_source()).await;
    seed_tasks(&state).await;

    let explained = call_rpc_json(
        &state,
        QUERY_EXPLAIN,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Task",
            "args": {
                "where": { "status": "open" },
                "orderBy": [{ "priority": "desc" }],
                "take": 2
            }
        }),
    )
    .await;

    let sql = explained["sql"].as_str().expect("explain sql");
    assert!(sql.starts_with("SELECT"), "unexpected sql: {sql}");
    assert!(sql.contains("tasks"), "unexpected sql: {sql}");
    assert!(
        !sql.to_ascii_uppercase().starts_with("EXPLAIN"),
        "explain must report the explained statement, not the EXPLAIN wrapper: {sql}"
    );
    assert_eq!(explained["params"], json!(["open"]));
    assert!(
        !explained["plan"]
            .as_array()
            .expect("explain plan array")
            .is_empty(),
        "plan should carry at least one row: {explained}"
    );

    drop(temp_dir);
}

#[tokio::test]
async fn engine_metrics_counts_dispatched_methods_and_resets_on_request() {
    let (state, temp_dir) = sqlite_state("engine-surface-metrics", schema_source()).await;
    seed_tasks(&state).await;

    let metrics = call_rpc_json(
        &state,
        ENGINE_METRICS,
        json!({ "protocolVersion": PROTOCOL_VERSION }),
    )
    .await;

    let creates = metrics["methods"]
        .as_array()
        .expect("methods array")
        .iter()
        .find(|entry| entry["method"] == json!(QUERY_CREATE))
        .expect("query.create should be counted");
    assert_eq!(creates["calls"], json!(4));
    assert_eq!(creates["errors"], json!(0));
    assert_eq!(metrics["activeTransactions"], json!(0));
    assert!(metrics["planCache"]["capacity"].as_u64().unwrap_or(0) > 0);

    let after_reset = call_rpc_json(
        &state,
        ENGINE_METRICS,
        json!({ "protocolVersion": PROTOCOL_VERSION, "reset": true }),
    )
    .await;
    assert!(after_reset["methods"]
        .as_array()
        .expect("methods array")
        .iter()
        .any(|entry| entry["method"] == json!(QUERY_CREATE)));

    let cleared = call_rpc_json(
        &state,
        ENGINE_METRICS,
        json!({ "protocolVersion": PROTOCOL_VERSION }),
    )
    .await;
    let creates = cleared["methods"]
        .as_array()
        .expect("methods array")
        .iter()
        .find(|entry| entry["method"] == json!(QUERY_CREATE))
        .expect("query.create key survives a reset");
    assert_eq!(creates["calls"], json!(0));

    drop(temp_dir);
}

#[tokio::test]
async fn engine_metrics_records_failed_requests_as_errors() {
    let (state, temp_dir) = sqlite_state("engine-surface-metrics-errors", schema_source()).await;

    let response = call_rpc_response(
        &state,
        QUERY_AGGREGATE,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Nope",
            "args": { "count": true }
        }),
    )
    .await;
    assert!(response.error.is_some());

    let metrics = call_rpc_json(
        &state,
        ENGINE_METRICS,
        json!({ "protocolVersion": PROTOCOL_VERSION }),
    )
    .await;
    let aggregate = metrics["methods"]
        .as_array()
        .expect("methods array")
        .iter()
        .find(|entry| entry["method"] == json!(QUERY_AGGREGATE))
        .expect("query.aggregate should be counted");
    assert_eq!(aggregate["calls"], json!(1));
    assert_eq!(aggregate["errors"], json!(1));

    drop(temp_dir);
}
