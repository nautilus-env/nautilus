//! PostgreSQL XML writes and the query operations XML actually supports.

#[allow(dead_code)]
mod common;

use common::{call_rpc_json, call_rpc_response, parse_ir};
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

#[tokio::test]
#[ignore = "requires PostgreSQL; set NAUTILUS_TEST_POSTGRES_URL"]
async fn postgres_xml_writes_reads_null_checks_and_counts() {
    let table = format!("xml_orders_{}", uuid::Uuid::new_v4().simple());
    let schema = parse_ir(&format!(
        r#"
datasource db {{ provider = "postgresql" url = env("DATABASE_URL") }}
model Order {{
  id Int @id
  region String
  invoice Xml? @map("invoice_xml")
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

    let document = "<invoice><customer>caffè &amp; tea</customer></invoice>";
    let column = format!("{table}__invoice_xml");
    let created = call(
        &state,
        "query.create",
        json!({"data": {
            "id": 1, "region": "north", "invoice": document
        }}),
    )
    .await;
    assert_eq!(created["data"][0][&column], document);
    let row = call(&state, "query.findUnique", json!({"filter": {"id": 1}})).await;
    assert_eq!(row["data"][0][&column], document);
    for args in [
        json!({"where": {"id": 1}}),
        json!({"where": {"id": 1}, "select": {"invoice": true}}),
    ] {
        let row = call(&state, "query.findMany", json!({"args": args})).await;
        assert_eq!(row["data"][0][&column], document);
    }
    let updated = call(
        &state,
        "query.update",
        json!({
            "filter": {"id": 1}, "data": {"invoice": "<invoice>updated</invoice>"}
        }),
    )
    .await;
    assert_eq!(updated["data"][0][&column], "<invoice>updated</invoice>");
    call(
        &state,
        "query.createMany",
        json!({"data": [
            {"id": 2, "region": "north", "invoice": null},
            {"id": 3, "region": "south", "invoice": "<invoice>third</invoice>"}
        ]}),
    )
    .await;
    call(
        &state,
        "query.updateMany",
        json!({"filter": {"id": 3},
        "data": {"invoice": "<invoice>batch update</invoice>"}}),
    )
    .await;
    call(
        &state,
        "query.upsert",
        json!({"filter": {"id": 3},
            "create": {"id": 3, "region": "south", "invoice": "<invoice/>"},
            "update": {"invoice": "<invoice>upsert</invoice>"}
        }),
    )
    .await;
    let row = call(&state, "query.findUnique", json!({"filter": {"id": 3}})).await;
    assert_eq!(row["data"][0][&column], "<invoice>upsert</invoice>");
    for field in ["invoice", "invoice_xml"] {
        for (predicate, length) in [
            (json!({"isNull": true}), 1),
            (json!({"isNull": false}), 2),
            (json!({"isNotNull": true}), 2),
            (json!({"isNotNull": false}), 1),
            (Value::Null, 1),
            (json!({"eq": null}), 1),
            (json!({"not": null}), 2),
        ] {
            let rows = call(
                &state,
                "query.findMany",
                json!({"args": {"where": {field: predicate}}}),
            )
            .await;
            assert_eq!(rows["data"].as_array().unwrap().len(), length);
        }
    }
    let count = call(
        &state,
        "query.aggregate",
        json!({"args": {"count": {"invoice": true}}}),
    )
    .await;
    assert_eq!(count["data"][0]["_count"]["invoice"], 2);
    let groups = call(
        &state,
        "query.groupBy",
        json!({"args": {"by": ["region"],
        "count": {"invoice": true}, "having": {"_count": {"invoice": {"gt": 0}}},
        "orderBy": [{"_count": {"invoice": "asc"}}]}}),
    )
    .await;
    assert_eq!(groups["data"].as_array().unwrap().len(), 2);
    call(
        &state,
        "query.update",
        json!({"filter": {"id": 1}, "data": {"invoice": null}}),
    )
    .await;
    let rows = call(
        &state,
        "query.findMany",
        json!({"args": {"where": {"invoice": {"isNull": true}}}}),
    )
    .await;
    assert_eq!(rows["data"].as_array().unwrap().len(), 2);

    for field in ["invoice", "invoice_xml"] {
        for (method, args) in [
            ("query.findMany", json!({"where": {field: document}})),
            ("query.findMany", json!({"orderBy": [{field: "asc"}]})),
            ("query.findMany", json!({"distinct": [field]})),
            ("query.groupBy", json!({"by": [field]})),
            (
                "query.groupBy",
                json!({"by": ["region"], "orderBy": [{field: "asc"}]}),
            ),
            (
                "query.groupBy",
                json!({"by": ["region"], "having": {field: {"eq": document}}}),
            ),
            (
                "query.groupBy",
                json!({"by": ["region"], "having": {"_min": {field: {"eq": document}}}}),
            ),
            (
                "query.groupBy",
                json!({"by": ["region"], "orderBy": [{"_max": {field: "asc"}}]}),
            ),
        ] {
            let response = call_rpc_response(
                &state,
                method,
                json!({"protocolVersion": PROTOCOL_VERSION, "model": "Order", "args": args}),
            )
            .await;
            let error = response.error.expect("unsupported XML operation must fail");
            assert!(error.message.contains("Xml"), "{}", error.message);
            assert_eq!(error.code, 2001, "{}", error.message);
        }
    }
    state
        .execute_ddl_sql(vec![format!("DROP TABLE \"{table}\"")])
        .await
        .unwrap();
}
