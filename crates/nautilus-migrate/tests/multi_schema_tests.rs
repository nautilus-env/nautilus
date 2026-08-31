mod common;

use nautilus_core::TableName;
use nautilus_migrate::live::{LiveColumn, LiveSchema, LiveTable};
use nautilus_migrate::{
    serialize_live_schema, Change, DatabaseProvider, DdlGenerator, DiffApplier, SchemaDiff,
};

const SCHEMA: &str = r#"
datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
  schemas  = ["public", "analytics"]
}

model User {
  id     Int     @id
  email  String  @unique
  events Event[]
  @@map("users")
  @@schema("public")
}

model Event {
  id     Int    @id
  kind   String
  userId Int    @map("user_id")
  user   User   @relation(fields: [userId], references: [id])
  @@index([kind])
  @@map("events")
  @@schema("analytics")
}
"#;

fn column(name: &str, col_type: &str) -> LiveColumn {
    LiveColumn {
        name: name.to_string(),
        col_type: col_type.to_string(),
        nullable: false,
        default_value: None,
        generated_expr: None,
        computed_kind: None,
        check_expr: None,
        auto_increment: false,
    }
}

fn table(schema: &str, name: &str, columns: Vec<LiveColumn>) -> LiveTable {
    LiveTable {
        name: TableName::qualified(schema, name),
        primary_key: vec![columns[0].name.clone()],
        columns,
        indexes: Vec::new(),
        check_constraints: Vec::new(),
        foreign_keys: Vec::new(),
    }
}

#[test]
fn create_table_ddl_qualifies_every_table_and_creates_the_schemas() {
    let ir = common::parse(SCHEMA).unwrap();
    let statements = DdlGenerator::new(DatabaseProvider::Postgres)
        .generate_create_tables(&ir)
        .unwrap();
    let all = statements.join("\n");

    assert!(
        statements.contains(&"CREATE SCHEMA IF NOT EXISTS \"public\"".to_string()),
        "{all}"
    );
    assert!(
        statements.contains(&"CREATE SCHEMA IF NOT EXISTS \"analytics\"".to_string()),
        "{all}"
    );
    assert!(
        all.contains("CREATE TABLE IF NOT EXISTS \"public\".\"users\""),
        "{all}"
    );
    assert!(
        all.contains("CREATE TABLE IF NOT EXISTS \"analytics\".\"events\""),
        "{all}"
    );

    // A cross-schema foreign key has to name the schema of the table it points at.
    assert!(all.contains("REFERENCES \"public\".\"users\""), "{all}");
    // An index lives in its table's schema, so only the table is qualified.
    assert!(all.contains("ON \"analytics\".\"events\""), "{all}");
    assert!(!all.contains("\"analytics\".\"idx_"), "{all}");
}

#[test]
fn schemas_are_created_before_the_tables_that_need_them() {
    let ir = common::parse(SCHEMA).unwrap();
    let statements = DdlGenerator::new(DatabaseProvider::Postgres)
        .generate_create_tables(&ir)
        .unwrap();

    let first_table = statements
        .iter()
        .position(|s| s.starts_with("CREATE TABLE"))
        .expect("a CREATE TABLE statement");
    let last_schema = statements
        .iter()
        .rposition(|s| s.starts_with("CREATE SCHEMA"))
        .expect("a CREATE SCHEMA statement");

    assert!(last_schema < first_table);
}

#[test]
fn a_missing_schema_is_diffed_as_create_schema() {
    let ir = common::parse(SCHEMA).unwrap();
    let mut live = LiveSchema::default();
    live.schemas.insert("public".to_string());

    let changes = SchemaDiff::compute(&live, &ir, DatabaseProvider::Postgres);
    let created: Vec<&String> = changes
        .iter()
        .filter_map(|c| match c {
            Change::CreateSchema { name } => Some(name),
            _ => None,
        })
        .collect();

    assert_eq!(created, vec!["analytics"]);

    let ddl = DdlGenerator::new(DatabaseProvider::Postgres);
    let applier = DiffApplier::new(DatabaseProvider::Postgres, &ddl, &ir, &live);
    let sql = applier
        .sql_for(&Change::CreateSchema {
            name: "analytics".to_string(),
        })
        .unwrap();
    assert_eq!(sql, vec!["CREATE SCHEMA IF NOT EXISTS \"analytics\""]);
}

#[test]
fn a_table_present_in_both_schemas_is_matched_per_schema() {
    let ir = common::parse(SCHEMA).unwrap();
    let mut live = LiveSchema::default();
    live.schemas.insert("public".to_string());
    live.schemas.insert("analytics".to_string());
    live.tables.insert(
        TableName::qualified("public", "users"),
        table(
            "public",
            "users",
            vec![column("id", "integer"), column("email", "text")],
        ),
    );
    live.tables.insert(
        TableName::qualified("analytics", "events"),
        table(
            "analytics",
            "events",
            vec![
                column("id", "integer"),
                column("kind", "text"),
                column("user_id", "integer"),
            ],
        ),
    );

    let changes = SchemaDiff::compute(&live, &ir, DatabaseProvider::Postgres);

    assert!(
        !changes
            .iter()
            .any(|c| matches!(c, Change::NewTable(_) | Change::DroppedTable { .. })),
        "{changes:#?}"
    );
}

#[test]
fn the_same_table_name_in_another_schema_is_not_a_match() {
    let ir = common::parse(SCHEMA).unwrap();
    let mut live = LiveSchema::default();
    live.schemas.insert("public".to_string());
    live.schemas.insert("analytics".to_string());
    // `events` sits in the wrong schema: the diff has to create the one the
    // schema asks for and drop the stray one, not treat them as one table.
    live.tables.insert(
        TableName::qualified("public", "events"),
        table("public", "events", vec![column("id", "integer")]),
    );

    let changes = SchemaDiff::compute(&live, &ir, DatabaseProvider::Postgres);

    assert!(
        changes.iter().any(|c| matches!(
            c,
            Change::NewTable(model)
                if model.db_name == "events" && model.schema.as_deref() == Some("analytics")
        )),
        "{changes:#?}"
    );
    assert!(
        changes.iter().any(|c| matches!(
            c,
            Change::DroppedTable { name } if *name == TableName::qualified("public", "events")
        )),
        "{changes:#?}"
    );
}

#[test]
fn pull_round_trips_the_schemas_list_and_the_attribute() {
    let mut live = LiveSchema::default();
    live.tables.insert(
        TableName::qualified("public", "users"),
        table(
            "public",
            "users",
            vec![column("id", "integer"), column("email", "text")],
        ),
    );
    live.tables.insert(
        TableName::qualified("analytics", "events"),
        table("analytics", "events", vec![column("id", "integer")]),
    );

    let source = serialize_live_schema(&live, DatabaseProvider::Postgres, "env(\"DATABASE_URL\")");

    assert!(
        source.contains("schemas  = [\"analytics\", \"public\"]"),
        "{source}"
    );
    assert!(source.contains("@@map(\"users\")"), "{source}");
    assert!(source.contains("@@schema(\"public\")"), "{source}");
    assert!(source.contains("@@schema(\"analytics\")"), "{source}");
    assert!(!source.contains("@@map(\"analytics.events\")"), "{source}");

    // The pulled schema has to be a schema Nautilus accepts.
    let ir = common::parse(&source).expect("pulled schema re-parses");
    assert_eq!(
        ir.datasource.as_ref().unwrap().schemas,
        vec!["analytics".to_string(), "public".to_string()]
    );
}

#[test]
fn a_single_schema_pull_stays_unqualified() {
    let mut live = LiveSchema::default();
    live.tables.insert(
        TableName::new("users"),
        LiveTable {
            name: TableName::new("users"),
            columns: vec![column("id", "integer")],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            check_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        },
    );

    let source = serialize_live_schema(&live, DatabaseProvider::Postgres, "env(\"DATABASE_URL\")");
    assert!(!source.contains("schemas"), "{source}");
    assert!(!source.contains("@@schema"), "{source}");
}
