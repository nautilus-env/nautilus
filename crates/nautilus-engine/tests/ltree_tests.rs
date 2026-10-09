//! PostgreSQL ltree values must read back as paths, without wire format bytes.

#[allow(dead_code)]
mod common;

use common::{call_rpc_json, parse_ir};
use nautilus_engine::EngineState;
use nautilus_migrate::{DatabaseProvider, DdlGenerator};
use nautilus_protocol::PROTOCOL_VERSION;
use serde_json::{json, Value};

async fn call(state: &EngineState, method: &str, params: Value) -> Value {
    let mut params = params.as_object().unwrap().clone();
    params.insert("protocolVersion".into(), json!(PROTOCOL_VERSION));
    params.insert("model".into(), json!("Order"));
    call_rpc_json(state, method, Value::Object(params)).await
}

fn assert_paths(response: &Value, table: &str, path: Value, paths: Value) {
    assert_eq!(response["data"][0][format!("{table}__route_path")], path);
    assert_eq!(response["data"][0][format!("{table}__route_paths")], paths);
}

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_ltree_scalar_and_array_round_trip() {
    let table = format!("ltree_orders_{}", uuid::Uuid::new_v4().simple());
    let schema = parse_ir(&format!(
        r#"
datasource db {{
  provider = "postgresql"
  url = env("DATABASE_URL")
  extensions = [ltree]
}}
model Order {{
  id Int @id
  routePath Ltree? @map("route_path")
  routePaths Ltree[] @map("route_paths")
  @@map("{table}")
}}
"#
    ));
    let ddl = DdlGenerator::new(DatabaseProvider::Postgres)
        .generate_create_tables(&schema)
        .unwrap();
    let state = EngineState::new(
        schema,
        std::env::var("NAUTILUS_TEST_POSTGRES_URL").expect("PostgreSQL test URL"),
        None,
    )
    .await
    .unwrap();
    state.execute_ddl_sql(ddl).await.unwrap();

    let created = call(
        &state,
        "query.create",
        json!({"data": {"id": 1, "routePath": "top.c", "routePaths": []}}),
    )
    .await;
    assert_paths(&created, &table, json!("top.c"), json!([]));
    state
        .execute_ddl_sql(vec![format!(
            "UPDATE \"{table}\" SET route_paths = ARRAY['top.c', 'top.d', '']::ltree[] WHERE id = 1"
        )])
        .await
        .unwrap();
    for (method, params) in [
        ("query.findUnique", json!({"filter": {"id": 1}})),
        (
            "query.findMany",
            json!({"args": {"where": {"routePath": "top.c"}}}),
        ),
        (
            "query.findMany",
            json!({"args": {"select": {"routePath": true, "routePaths": true}}}),
        ),
    ] {
        let rows = call(&state, method, params).await;
        assert_paths(&rows, &table, json!("top.c"), json!(["top.c", "top.d", ""]));
    }
    let updated = call(
        &state,
        "query.update",
        json!({"filter": {"id": 1}, "data": {"routePath": "top.updated"}}),
    )
    .await;
    assert_paths(
        &updated,
        &table,
        json!("top.updated"),
        json!(["top.c", "top.d", ""]),
    );

    state
        .execute_ddl_sql(vec![format!(
            "INSERT INTO \"{table}\" (id, route_path, route_paths) VALUES \
             (2, 'top.sql', ARRAY['top.sql', '']::ltree[]), \
             (3, NULL, ARRAY[]::ltree[]), (4, '', ARRAY[NULL, 'top.null']::ltree[])"
        )])
        .await
        .unwrap();
    for (id, path, paths) in [
        (2, json!("top.sql"), json!(["top.sql", ""])),
        (3, Value::Null, json!([])),
        (4, json!(""), json!([null, "top.null"])),
    ] {
        let row = call(&state, "query.findUnique", json!({"filter": {"id": id}})).await;
        assert_paths(&row, &table, path, paths);
    }
    for path in [Value::Null, json!("")] {
        let updated = call(
            &state,
            "query.update",
            json!({"filter": {"id": 1}, "data": {"routePath": path, "routePaths": []}}),
        )
        .await;
        assert_paths(&updated, &table, path.clone(), json!([]));
        let row = call(&state, "query.findUnique", json!({"filter": {"id": 1}})).await;
        assert_paths(&row, &table, path, json!([]));
    }
    state
        .execute_ddl_sql(vec![format!("DROP TABLE \"{table}\"")])
        .await
        .unwrap();
}
