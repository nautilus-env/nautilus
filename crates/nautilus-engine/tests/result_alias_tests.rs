//! PostgreSQL result aliases preserve full field names and distinct values.

#[allow(dead_code)]
mod common;

use common::{call_rpc_json, parse_ir};
use nautilus_core::{Expr, OrderDir, Select, SelectItem};
use nautilus_dialect::{Dialect, PostgresDialect};
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
async fn postgres_long_result_aliases_round_trip() {
    for table in [
        format!("orders_{}aaaaaaaaaa", uuid::Uuid::new_v4().simple()),
        format!(
            "ordini_{}{}",
            "è".repeat(14),
            &uuid::Uuid::new_v4().simple().to_string()[..16]
        ),
    ] {
        let schema = parse_ir(&format!(
            r#"
datasource db {{ provider = "postgresql" url = env("DATABASE_URL") extensions = [citext] }}
model Order {{
  id Int @id @map("order_identifier")
  contactNames Citext[] @map("contact_names")
  contactNamex Citext[] @map("contact_namex")
  contactNotes String? @map("contact_notes")
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
        let lists = json!({"contactNames": ["Alice", "", "O'Reilly"],
            "contactNamex": ["Bob"], "contactNotes": "caffè"});
        let assert_row = |row: &Value, id: i32, expected: &Value| {
            assert_eq!(row[format!("{table}__order_identifier")], id);
            for (logical, column) in [
                ("contactNames", "contact_names"),
                ("contactNamex", "contact_namex"),
                ("contactNotes", "contact_notes"),
            ] {
                assert_eq!(row[format!("{table}__{column}")], expected[logical]);
            }
        };
        let mut data = lists.clone();
        data["id"] = json!(1);
        let created = call(&state, "query.create", json!({"data": data})).await;
        assert_row(&created["data"][0], 1, &lists);
        // Repeated requests exercise cached plans as well as freshly rendered ones.
        for _ in 0..2 {
            for (method, params) in [
                ("query.findUnique", json!({"filter": {"id": 1}})),
                ("query.findMany", json!({"args": {"where": {"id": 1}}})),
                (
                    "query.findMany",
                    json!({"args": {"where": {"id": 1}, "select": {
                    "contactNames": true, "contactNamex": true, "contactNotes": true}}}),
                ),
            ] {
                let row = call(&state, method, params).await;
                assert_row(&row["data"][0], 1, &lists);
            }
        }
        let empty = json!({"contactNames": [], "contactNamex": [], "contactNotes": null});
        let updated = call(
            &state,
            "query.update",
            json!({"filter": {"id": 1}, "data": empty}),
        )
        .await;
        assert_row(&updated["data"][0], 1, &empty);
        let batch = call(&state, "query.createMany", json!({"data": [
            {"id": 2, "contactNames": ["Alice"], "contactNamex": ["Bob"], "contactNotes": "north"},
            {"id": 3, "contactNames": [], "contactNamex": [], "contactNotes": null}], "returnData": true})).await;
        assert_eq!(batch["data"].as_array().unwrap().len(), 2);
        assert_eq!(
            batch["data"][0][format!("{table}__contact_names")],
            json!(["Alice"])
        );
        for id in [2, 4] {
            let mut create = lists.clone();
            create["id"] = json!(id);
            let row = call(
                &state,
                "query.upsert",
                json!({"filter": {"id": id},
                "create": create, "update": lists, "returnData": true}),
            )
            .await;
            assert_row(&row["data"][0], id, &lists);
        }
        let groups = call(
            &state,
            "query.groupBy",
            json!({"args": {
            "by": ["contactNotes"], "count": {"contactNotes": true}}}),
        )
        .await;
        assert_eq!(groups["data"].as_array().unwrap().len(), 2);
        assert!(groups["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["contactNotes"] == "caffè" && row["_count"]["contactNotes"] == 2));
        // SQL clauses referencing a computed output alias must use its bounded
        // name too, while physical table/column references remain unchanged.
        let computed = "è".repeat(32);
        let select = Select::from_table(&table)
            .item(SelectItem::computed(
                Expr::column(format!("{table}__order_identifier")),
                &computed,
            ))
            .distinct(vec![computed.clone()])
            .order_by(&computed, OrderDir::Asc)
            .build()
            .unwrap();
        let output = PostgresDialect.render_select_output_owned(select).unwrap();
        let rows = state
            .execute_query_on(&output.sql, "computed aliases", None)
            .await
            .unwrap();
        assert_eq!(rows.len(), 4);
        for row in rows {
            let row = row.restore_result_names(&output.aliases);
            assert!(row.get(&computed).is_some());
        }
        let deleted = call(
            &state,
            "query.delete",
            json!({"filter": {"id": 4}, "returnData": true}),
        )
        .await;
        assert_row(&deleted["data"][0], 4, &lists);
        state
            .execute_ddl_sql(vec![format!("DROP TABLE \"{table}\"")])
            .await
            .unwrap();
    }
}
