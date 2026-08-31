mod common;

use nautilus_migrate::live::{LiveColumn, LiveForeignKey, LiveSchema, LiveTable};
use nautilus_migrate::{serialize_live_schema, Change, DatabaseProvider, DdlGenerator, SchemaDiff};

const SCHEMA: &str = r#"
model Post {
  id    Int    @id @default(autoincrement())
  title String @unique
  tags  Tag[]
}

model Tag {
  id    Int    @id @default(autoincrement())
  label String @unique
  posts Post[]
}
"#;

fn create_tables(provider: DatabaseProvider) -> Vec<String> {
    let ir = common::parse(SCHEMA).unwrap();
    DdlGenerator::new(provider)
        .generate_create_tables(&ir)
        .unwrap()
}

fn join_table_ddl(provider: DatabaseProvider) -> String {
    create_tables(provider)
        .into_iter()
        .find(|sql| sql.contains("_PostToTag"))
        .expect("the join table is created")
}

#[test]
fn the_join_table_is_created_with_a_key_column_per_side() {
    for provider in [
        DatabaseProvider::Postgres,
        DatabaseProvider::Mysql,
        DatabaseProvider::Sqlite,
    ] {
        let sql = join_table_ddl(provider);
        assert!(sql.contains("\"A\"") || sql.contains("`A`"), "{sql}");
        assert!(sql.contains("\"B\"") || sql.contains("`B`"), "{sql}");
        assert!(sql.contains("PRIMARY KEY"), "{sql}");
        assert_eq!(sql.matches("FOREIGN KEY").count(), 2, "{sql}");
        assert_eq!(sql.matches("ON DELETE CASCADE").count(), 2, "{sql}");
    }
}

#[test]
fn an_empty_database_is_told_to_create_the_join_table_last() {
    let ir = common::parse(SCHEMA).unwrap();
    let changes = SchemaDiff::compute(&LiveSchema::default(), &ir, DatabaseProvider::Postgres);

    let created: Vec<&str> = changes
        .iter()
        .filter_map(|change| match change {
            Change::NewTable(model) => Some(model.db_name.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(created.len(), 3, "{created:?}");
    assert_eq!(
        created.last(),
        Some(&"_PostToTag"),
        "the join table references both models, so it comes after them: {created:?}"
    );
}

#[test]
fn the_join_table_is_indexed_on_the_side_the_primary_key_does_not_lead_with() {
    let statements = create_tables(DatabaseProvider::Postgres);

    assert!(
        statements
            .iter()
            .any(|sql| sql.contains("_PostToTag_B_index") && sql.contains("CREATE INDEX")),
        "{statements:?}"
    );
}

fn key_column(name: &str) -> LiveColumn {
    LiveColumn {
        name: name.to_string(),
        col_type: "integer".to_string(),
        nullable: false,
        default_value: None,
        generated_expr: None,
        computed_kind: None,
        check_expr: None,
        auto_increment: false,
    }
}

fn model_table(name: &str) -> LiveTable {
    LiveTable {
        name: name.to_string(),
        columns: vec![key_column("id")],
        primary_key: vec!["id".to_string()],
        indexes: vec![],
        check_constraints: vec![],
        foreign_keys: vec![],
    }
}

fn link(column: &str, referenced_table: &str) -> LiveForeignKey {
    LiveForeignKey {
        constraint_name: format!("fk_{}_{}", referenced_table, column),
        columns: vec![column.to_string()],
        referenced_table: referenced_table.to_string(),
        referenced_columns: vec!["id".to_string()],
        on_delete: Some("CASCADE".to_string()),
        on_update: Some("CASCADE".to_string()),
    }
}

fn join_table(name: &str, a_table: &str, b_table: &str) -> LiveTable {
    LiveTable {
        name: name.to_string(),
        columns: vec![key_column("A"), key_column("B")],
        primary_key: vec!["A".to_string(), "B".to_string()],
        indexes: vec![],
        check_constraints: vec![],
        foreign_keys: vec![link("A", a_table), link("B", b_table)],
    }
}

fn pulled(tables: Vec<LiveTable>) -> String {
    serialize_live_schema(
        &common::make_live_schema(tables),
        DatabaseProvider::Postgres,
        "postgres://localhost/test",
    )
}

#[test]
fn pull_recovers_the_relation_instead_of_the_join_table() {
    let source = pulled(vec![
        model_table("Post"),
        model_table("Tag"),
        join_table("_PostToTag", "Post", "Tag"),
    ]);

    assert!(!source.contains("model PostToTag"), "{source}");
    assert!(!source.contains("_PostToTag"), "{source}");
    assert!(source.contains("  tags  Tag[]"), "{source}");
    assert!(source.contains("  posts  Post[]"), "{source}");
}

#[test]
fn pull_names_a_relation_whose_join_table_is_not_the_default_one() {
    let source = pulled(vec![
        model_table("Post"),
        model_table("Tag"),
        join_table("_Labelling", "Post", "Tag"),
    ]);

    assert_eq!(
        source.matches("@relation(name: \"Labelling\")").count(),
        2,
        "{source}"
    );
}

#[test]
fn pull_leaves_a_link_table_that_is_not_one_of_ours_as_a_model() {
    let mut explicit = join_table("post_tags", "Post", "Tag");
    explicit.columns = vec![key_column("post_id"), key_column("tag_id")];
    explicit.primary_key = vec!["post_id".to_string(), "tag_id".to_string()];
    explicit.foreign_keys = vec![link("post_id", "Post"), link("tag_id", "Tag")];

    let source = pulled(vec![model_table("Post"), model_table("Tag"), explicit]);

    assert!(source.contains("@@map(\"post_tags\")"), "{source}");
}
