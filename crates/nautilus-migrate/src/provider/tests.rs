use super::*;
use nautilus_schema::ir::{BasicIndexType, IndexKind};

#[test]
fn postgres_hash_index_uses_if_not_exists_and_using_clause() {
    let strategy = ProviderStrategy::new(DatabaseProvider::Postgres);
    let columns = vec!["email".to_string()];
    let kind = IndexKind::Basic(BasicIndexType::Hash);

    let sql = strategy.create_index_sql(CreateIndex {
        table: &TableName::new("User"),
        name: "email_hash_idx",
        columns: &columns,
        unique: false,
        kind: &kind,
        if_not_exists: true,
        predicate: None,
    });

    assert_eq!(
        sql,
        "CREATE INDEX IF NOT EXISTS \"email_hash_idx\" ON \"User\" USING HASH (\"email\")"
    );
}

#[test]
fn mysql_fulltext_index_uses_native_syntax() {
    let strategy = ProviderStrategy::new(DatabaseProvider::Mysql);
    let columns = vec!["body".to_string()];
    let kind = IndexKind::Basic(BasicIndexType::FullText);

    let sql = strategy.create_index_sql(CreateIndex {
        table: &TableName::new("Post"),
        name: "body_search",
        columns: &columns,
        unique: false,
        kind: &kind,
        if_not_exists: true,
        predicate: None,
    });

    assert_eq!(
        sql,
        "CREATE FULLTEXT INDEX `body_search` ON `Post` (`body`)"
    );
}

#[test]
fn postgres_drop_table_can_include_cascade() {
    let strategy = ProviderStrategy::new(DatabaseProvider::Postgres);

    assert_eq!(
        strategy.drop_table_sql(&TableName::new("users"), true),
        "DROP TABLE IF EXISTS \"users\" CASCADE"
    );
    assert_eq!(
        strategy.drop_table_sql(&TableName::new("users"), false),
        "DROP TABLE IF EXISTS \"users\""
    );
}

#[test]
fn postgres_not_null_with_default_backfills_before_constraint() {
    let strategy = ProviderStrategy::new(DatabaseProvider::Postgres);

    let plan = strategy
        .alter_column_nullability_sql(AlterColumnNullability {
            table: &TableName::new("User"),
            column: "email",
            now_required: true,
            is_generated: false,
            default_sql: Some("'unknown@example.com'"),
            full_column_definition: None,
        })
        .unwrap();

    let ProviderSqlPlan::Statements(sql) = plan else {
        panic!("expected postgres statements plan");
    };

    assert_eq!(
        sql,
        vec![
            "ALTER TABLE \"User\" ALTER COLUMN \"email\" SET DEFAULT 'unknown@example.com'"
                .to_string(),
            "UPDATE \"User\" SET \"email\" = 'unknown@example.com' WHERE \"email\" IS NULL"
                .to_string(),
            "ALTER TABLE \"User\" ALTER COLUMN \"email\" SET NOT NULL".to_string(),
        ]
    );
}

#[test]
fn mysql_type_rewrite_uses_modify_column() {
    let strategy = ProviderStrategy::new(DatabaseProvider::Mysql);

    let plan = strategy
        .alter_column_type_sql(AlterColumnType {
            table: &TableName::new("User"),
            column: "email",
            target_type: "VARCHAR(255)",
            full_column_definition: Some("`email` VARCHAR(255) NOT NULL"),
        })
        .unwrap();

    let ProviderSqlPlan::Statements(sql) = plan else {
        panic!("expected mysql statements plan");
    };

    assert_eq!(
        sql,
        vec!["ALTER TABLE `User` MODIFY COLUMN `email` VARCHAR(255) NOT NULL".to_string()]
    );
}

#[test]
fn sqlite_column_changes_require_rebuild() {
    let strategy = ProviderStrategy::new(DatabaseProvider::Sqlite);

    let type_plan = strategy
        .alter_column_type_sql(AlterColumnType {
            table: &TableName::new("User"),
            column: "email",
            target_type: "TEXT",
            full_column_definition: None,
        })
        .unwrap();
    assert!(matches!(type_plan, ProviderSqlPlan::RequiresTableRebuild));

    let default_plan = strategy
        .alter_column_default_sql(AlterColumnDefault {
            table: &TableName::new("User"),
            column: "email",
            new_default: Some("'x'"),
            preserve_implicit_default: false,
            full_column_definition: None,
        })
        .unwrap();
    assert!(matches!(
        default_plan,
        ProviderSqlPlan::RequiresTableRebuild
    ));
}

#[test]
fn postgres_implicit_serial_default_is_preserved() {
    let strategy = ProviderStrategy::new(DatabaseProvider::Postgres);

    let plan = strategy
        .alter_column_default_sql(AlterColumnDefault {
            table: &TableName::new("User"),
            column: "id",
            new_default: None,
            preserve_implicit_default: true,
            full_column_definition: None,
        })
        .unwrap();

    let ProviderSqlPlan::Statements(sql) = plan else {
        panic!("expected postgres statements plan");
    };
    assert!(sql.is_empty());
}

#[test]
fn mysql_json_storage_and_udt_support_are_provider_aware() {
    let mysql = ProviderStrategy::new(DatabaseProvider::Mysql);
    let sqlite = ProviderStrategy::new(DatabaseProvider::Sqlite);
    let postgres = ProviderStrategy::new(DatabaseProvider::Postgres);

    assert_eq!(
        mysql.array_storage_sql(Some(StorageStrategy::Json)),
        Some("JSON")
    );
    assert_eq!(
        sqlite.composite_storage_sql(Some(StorageStrategy::Json)),
        Some("TEXT")
    );
    assert!(postgres.supports_user_defined_types());
    assert!(!mysql.supports_user_defined_types());
}
