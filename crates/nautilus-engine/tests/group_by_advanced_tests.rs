mod common;

use common::{call_rpc_json, sqlite_state};
use nautilus_engine::EngineState;
use nautilus_protocol::{PROTOCOL_VERSION, QUERY_CREATE, QUERY_GROUP_BY};
use serde_json::json;

fn schema_source() -> &'static str {
    r#"
datasource db {
  provider = "sqlite"
  url      = "sqlite::memory:"
}

model Metric {
  id     Int    @id @default(autoincrement()) @map("metric_id")
  bucket String @map("bucket_name")
  label  String @map("metric_label")
  points Int    @map("points_value")

  @@map("metrics")
}
"#
}

async fn seed_metrics(state: &EngineState) {
    for (bucket, label, points) in [
        ("gold", "gold-b", 20),
        ("gold", "gold-a", 50),
        ("silver", "silver-b", 10),
        ("silver", "silver-a", 30),
        ("bronze", "bronze-a", 5),
    ] {
        let _ = call_rpc_json(
            state,
            QUERY_CREATE,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "model": "Metric",
                "data": {
                    "bucket": bucket,
                    "label": label,
                    "points": points
                }
            }),
        )
        .await;
    }
}

#[tokio::test]
async fn group_by_orders_by_aggregate_and_returns_multi_aggregate_payloads() {
    let (state, temp_dir) = sqlite_state("group-by-advanced-tests", schema_source()).await;
    seed_metrics(&state).await;

    let grouped = call_rpc_json(
        &state,
        QUERY_GROUP_BY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Metric",
            "args": {
                "by": ["bucket"],
                "count": {
                    "_all": true,
                    "label": true
                },
                "sum": {
                    "points": true
                },
                "min": {
                    "label": true
                },
                "max": {
                    "points": true
                },
                "orderBy": [
                    { "_sum": { "points": "desc" } },
                    { "bucket": "asc" }
                ]
            }
        }),
    )
    .await;

    let rows = grouped["data"]
        .as_array()
        .expect("groupBy should return a data array");
    assert_eq!(rows.len(), 3);

    assert_eq!(rows[0]["bucket"], json!("gold"));
    assert_eq!(rows[0]["_count"]["_all"], json!(2));
    assert_eq!(rows[0]["_count"]["label"], json!(2));
    assert_eq!(rows[0]["_sum"]["points"], json!(70));
    assert_eq!(rows[0]["_min"]["label"], json!("gold-a"));
    assert_eq!(rows[0]["_max"]["points"], json!(50));

    assert_eq!(rows[1]["bucket"], json!("silver"));
    assert_eq!(rows[1]["_count"]["_all"], json!(2));
    assert_eq!(rows[1]["_sum"]["points"], json!(40));
    assert_eq!(rows[1]["_min"]["label"], json!("silver-a"));
    assert_eq!(rows[1]["_max"]["points"], json!(30));

    assert_eq!(rows[2]["bucket"], json!("bronze"));
    assert_eq!(rows[2]["_count"]["_all"], json!(1));
    assert_eq!(rows[2]["_sum"]["points"], json!(5));

    drop(state);
    drop(temp_dir);
}

#[tokio::test]
async fn group_by_having_filters_on_aggregate_values() {
    let (state, temp_dir) = sqlite_state("group-by-advanced-tests", schema_source()).await;
    seed_metrics(&state).await;

    let grouped = call_rpc_json(
        &state,
        QUERY_GROUP_BY,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "model": "Metric",
            "args": {
                "by": ["bucket"],
                "count": {
                    "_all": true
                },
                "sum": {
                    "points": true
                },
                "having": {
                    "_count": {
                        "_all": { "gt": 1 }
                    },
                    "_sum": {
                        "points": { "gte": 40 }
                    }
                },
                "orderBy": [
                    { "bucket": "asc" }
                ]
            }
        }),
    )
    .await;

    let rows = grouped["data"]
        .as_array()
        .expect("groupBy should return a data array");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["bucket"], json!("gold"));
    assert_eq!(rows[0]["_count"]["_all"], json!(2));
    assert_eq!(rows[0]["_sum"]["points"], json!(70));
    assert_eq!(rows[1]["bucket"], json!("silver"));
    assert_eq!(rows[1]["_count"]["_all"], json!(2));
    assert_eq!(rows[1]["_sum"]["points"], json!(40));

    drop(state);
    drop(temp_dir);
}
