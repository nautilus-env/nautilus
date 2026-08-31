use nautilus_core::TableName;
mod common;

use nautilus_migrate::live::{LiveColumn, LiveSchema, LiveTable};
use nautilus_migrate::{serialize_live_schema, DatabaseProvider, DdlGenerator, SchemaDiff};

fn column(name: &str, col_type: &str, nullable: bool) -> LiveColumn {
    LiveColumn {
        name: name.to_string(),
        col_type: col_type.to_string(),
        nullable,
        default_value: None,
        generated_expr: None,
        computed_kind: None,
        check_expr: None,
        auto_increment: false,
    }
}

const SCHEMA_WITH_VIEW: &str = r#"
model User {
  id    Int    @id
  email String
}

view ActiveUser {
  id    Int    @id
  email String
  @@map("active_users")
}
"#;

#[test]
fn create_table_ddl_skips_views() {
    let ir = common::parse(SCHEMA_WITH_VIEW).unwrap();
    let statements = DdlGenerator::new(DatabaseProvider::Postgres)
        .generate_create_tables(&ir)
        .unwrap();

    assert!(
        statements.iter().any(|sql| sql.contains("\"User\"")),
        "expected the model's table: {statements:?}"
    );
    assert!(
        !statements.iter().any(|sql| sql.contains("active_users")),
        "a view must not reach DDL: {statements:?}"
    );
}

#[test]
fn diff_never_creates_a_view() {
    let ir = common::parse(SCHEMA_WITH_VIEW).unwrap();
    let changes = SchemaDiff::compute(&LiveSchema::default(), &ir, DatabaseProvider::Postgres);

    let created: Vec<&str> = changes
        .iter()
        .filter_map(|change| match change {
            nautilus_migrate::Change::NewTable(model) => Some(model.db_name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(created, vec!["User"]);
}

#[test]
fn diff_leaves_a_live_view_alone() {
    let ir = common::parse(SCHEMA_WITH_VIEW).unwrap();

    let mut live = LiveSchema::default();
    live.tables.insert(
        TableName::new("User".to_string()),
        LiveTable {
            name: TableName::new("User".to_string()),
            columns: vec![
                column("id", "integer", false),
                column("email", "text", false),
            ],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            check_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        },
    );
    live.views.insert(
        TableName::new("active_users".to_string()),
        LiveTable {
            name: TableName::new("active_users".to_string()),
            columns: vec![
                column("id", "integer", false),
                column("email", "text", false),
            ],
            primary_key: Vec::new(),
            indexes: Vec::new(),
            check_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        },
    );

    let changes = SchemaDiff::compute(&live, &ir, DatabaseProvider::Postgres);
    assert!(
        changes.is_empty(),
        "a declared view must produce no migration: {changes:?}"
    );
}

#[test]
fn db_pull_renders_a_view_block() {
    let mut live = LiveSchema::default();
    live.tables.insert(
        TableName::new("users".to_string()),
        LiveTable {
            name: TableName::new("users".to_string()),
            columns: vec![
                column("id", "integer", false),
                column("email", "text", false),
            ],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            check_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        },
    );
    live.views.insert(
        TableName::new("active_users".to_string()),
        LiveTable {
            name: TableName::new("active_users".to_string()),
            columns: vec![column("id", "integer", true), column("email", "text", true)],
            primary_key: Vec::new(),
            indexes: Vec::new(),
            check_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        },
    );

    let source = serialize_live_schema(&live, DatabaseProvider::Postgres, "env(\"DATABASE_URL\")");

    assert!(
        source.contains("view ActiveUsers {"),
        "expected a view block:\n{source}"
    );
    assert!(
        source.contains("@@map(\"active_users\")"),
        "expected the view's @@map:\n{source}"
    );

    let ir = common::parse(&source).expect("a pulled schema must parse back");
    assert!(ir.get_model("ActiveUsers").unwrap().is_view);
}
