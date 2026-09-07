use crate::ddl::DatabaseProvider;
use crate::live::{LiveIndex, LiveTable};
use crate::provider::{CreateIndex, ProviderStrategy};
use nautilus_core::TableName;

/// Reconstruct a dropped table and its indexes from the live snapshot.
/// SQLite inlines a single INTEGER primary key with AUTOINCREMENT.
pub(super) fn create_table_sql_from_live(
    table: &LiveTable,
    provider: DatabaseProvider,
) -> Vec<String> {
    let q = |name: &str| provider.quote_identifier(name);
    let strategy = ProviderStrategy::new(provider);
    let sqlite_inline_pk = provider == DatabaseProvider::Sqlite
        && table.primary_key.len() == 1
        && table
            .columns
            .iter()
            .any(|c| c.name == table.primary_key[0] && c.col_type.to_lowercase() == "integer");

    let mut col_lines: Vec<String> = Vec::new();
    for col in &table.columns {
        let is_pk = table.primary_key.contains(&col.name);
        if sqlite_inline_pk && is_pk {
            col_lines.push(format!(
                "  {} INTEGER PRIMARY KEY AUTOINCREMENT",
                q(&col.name)
            ));
        } else {
            let type_upper = col.col_type.to_uppercase();
            let mut parts = vec![q(&col.name), type_upper];
            if !col.nullable {
                parts.push("NOT NULL".to_string());
            }
            if let Some(default) = &col.default_value {
                parts.push(format!("DEFAULT {}", default));
            }
            col_lines.push(format!("  {}", parts.join(" ")));
        }
    }

    if !sqlite_inline_pk && !table.primary_key.is_empty() {
        let pk_cols = table
            .primary_key
            .iter()
            .map(|c| q(c))
            .collect::<Vec<_>>()
            .join(", ");
        col_lines.push(format!("  PRIMARY KEY ({})", pk_cols));
    }

    let mut stmts = vec![format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n)",
        strategy.quote_table(&table.name),
        col_lines.join(",\n"),
    )];

    for idx in &table.indexes {
        stmts.push(create_index_sql_from_live(&table.name, idx, provider));
    }

    stmts
}

pub(super) fn create_index_sql_from_live(
    table_name: &TableName,
    index: &LiveIndex,
    provider: DatabaseProvider,
) -> String {
    let kind = index.kind.to_index_kind();
    let predicate = index
        .predicate
        .as_deref()
        .map(crate::utils::schema_bool_expr_to_sql);
    ProviderStrategy::new(provider).create_index_sql(CreateIndex {
        table: table_name,
        name: &index.name,
        columns: &index.columns,
        unique: index.unique,
        kind: &kind,
        if_not_exists: true,
        predicate: predicate.as_deref(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::LiveColumn;

    fn live_table_with_index(table: &str, column: &str, index: LiveIndex) -> LiveTable {
        LiveTable {
            name: TableName::new(table),
            columns: vec![
                LiveColumn {
                    name: "id".to_string(),
                    col_type: "integer".to_string(),
                    nullable: false,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                    auto_increment: false,
                    self_updating: false,
                },
                LiveColumn {
                    name: column.to_string(),
                    col_type: "text".to_string(),
                    nullable: false,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                    auto_increment: false,
                    self_updating: false,
                },
            ],
            primary_key: vec!["id".to_string()],
            indexes: vec![index],
            check_constraints: vec![],
            foreign_keys: vec![],
        }
    }

    #[test]
    fn dropped_table_down_sql_preserves_postgres_index_name_and_method() {
        let stmts = create_table_sql_from_live(
            &live_table_with_index(
                "User",
                "email",
                LiveIndex {
                    name: "email_hash_idx".to_string(),
                    columns: vec!["email".to_string()],
                    unique: false,
                    kind: crate::live::LiveIndexKind::Basic(
                        nautilus_schema::ir::BasicIndexType::Hash,
                    ),
                    predicate: None,
                },
            ),
            DatabaseProvider::Postgres,
        );

        assert!(
            stmts.iter().any(|sql| {
                sql.contains("CREATE INDEX IF NOT EXISTS \"email_hash_idx\"")
                    && sql.contains("ON \"User\" USING HASH (\"email\")")
            }),
            "down SQL must preserve the live physical name and USING HASH method: {:?}",
            stmts
        );
        assert!(
            !stmts.iter().any(|sql| sql.contains("idx_User_email")),
            "down SQL must not fall back to auto-generated index names: {:?}",
            stmts
        );
    }

    #[test]
    fn dropped_table_down_sql_preserves_mysql_fulltext_index_name() {
        let stmts = create_table_sql_from_live(
            &live_table_with_index(
                "Post",
                "body",
                LiveIndex {
                    name: "body_search".to_string(),
                    columns: vec!["body".to_string()],
                    unique: false,
                    kind: crate::live::LiveIndexKind::Basic(
                        nautilus_schema::ir::BasicIndexType::FullText,
                    ),
                    predicate: None,
                },
            ),
            DatabaseProvider::Mysql,
        );

        assert!(
            stmts.iter().any(|sql| {
                sql.contains("CREATE FULLTEXT INDEX `body_search`")
                    && sql.contains("ON `Post` (`body`)")
            }),
            "down SQL must preserve MySQL FULLTEXT index metadata: {:?}",
            stmts
        );
        assert!(
            !stmts.iter().any(|sql| sql.contains("idx_Post_body")),
            "down SQL must not fall back to auto-generated index names: {:?}",
            stmts
        );
    }
}
