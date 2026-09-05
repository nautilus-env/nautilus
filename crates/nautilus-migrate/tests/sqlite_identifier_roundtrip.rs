//! Real-database check that a name carrying the identifier delimiter survives
//! `db pull`: SQLite introspection quotes it into every `PRAGMA`, and the
//! serializer escapes it back into a schema that reparses.

mod common;

use nautilus_core::TableName;
use nautilus_migrate::{serialize_live_schema, DatabaseProvider, DdlGenerator, SchemaInspector};

const SOURCE: &str = r#"
model Weird {
  id    Int    @id
  label String @map("we\"ird`col")

  @@map("we\"ird`table")
}
"#;

#[tokio::test]
async fn a_delimiter_in_a_name_survives_creation_and_introspection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roundtrip.db");
    let url = format!(
        "sqlite://{}?mode=rwc",
        path.to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/")
    );

    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let ir = common::parse(SOURCE).unwrap();
    for statement in DdlGenerator::new(DatabaseProvider::Sqlite)
        .generate_create_tables(&ir)
        .unwrap()
    {
        sqlx::query(&statement).execute(&pool).await.unwrap();
    }
    pool.close().await;

    let live = SchemaInspector::new(DatabaseProvider::Sqlite, &url)
        .inspect()
        .await
        .unwrap();

    let table = live
        .tables
        .get(&TableName::new("we\"ird`table"))
        .expect("introspection should find the table under its exact name");
    assert!(
        table.columns.iter().any(|c| c.name == "we\"ird`col"),
        "PRAGMA output was empty, so the table name did not survive quoting: {:?}",
        table.columns
    );
    assert_eq!(table.primary_key, vec!["id".to_string()]);

    let pulled = serialize_live_schema(&live, DatabaseProvider::Sqlite, &url);
    let reparsed = common::parse(&pulled).expect("pulled schema should reparse");
    assert!(reparsed
        .models
        .values()
        .any(|m| m.db_name == "we\"ird`table"));
}
