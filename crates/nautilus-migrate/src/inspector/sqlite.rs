use super::{
    group_sqlite_foreign_keys, normalize_sqlite_check_expr, normalize_sqlite_default,
    normalize_sqlite_type, parse_sqlite_check_constraints, parse_sqlite_generated_exprs,
    SchemaInspector,
};
use crate::error::{MigrationError, Result};
use crate::live::{ComputedKind, LiveColumn, LiveIndex, LiveIndexKind, LiveSchema, LiveTable};
use nautilus_core::ident::quote_ident;
use nautilus_core::TableName;

impl SchemaInspector {
    pub(super) async fn inspect_sqlite(&self) -> Result<LiveSchema> {
        use sqlx::Row as _;

        let opts: sqlx::sqlite::SqliteConnectOptions = self
            .url
            .parse::<sqlx::sqlite::SqliteConnectOptions>()
            .map_err(|e| MigrationError::Database(e.to_string()))?
            .create_if_missing(false);

        let pool = sqlx::SqlitePool::connect_with(opts)
            .await
            .map_err(|e| MigrationError::Database(format!("SQLite connection failed: {e}")))?;

        let table_rows = sqlx::query(
            "SELECT name, COALESCE(sql, '') AS create_sql FROM sqlite_master \
             WHERE type = 'table' \
               AND name NOT LIKE 'sqlite_%' \
               AND name NOT LIKE '_nautilus_%' \
             ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| MigrationError::Database(e.to_string()))?;

        let tables: Vec<(String, String)> = table_rows
            .into_iter()
            .map(|r| {
                let table_name = r
                    .try_get::<String, _>("name")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let create_sql = r
                    .try_get::<String, _>("create_sql")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                Ok((table_name, create_sql))
            })
            .collect::<Result<_>>()?;

        let mut live = LiveSchema::default();

        for (table_name, create_sql) in tables {
            let pragma_sql = format!("PRAGMA table_xinfo({})", quote_ident(&table_name, '"'));
            let col_rows = sqlx::query(&pragma_sql)
                .fetch_all(&pool)
                .await
                .map_err(|e| MigrationError::Database(e.to_string()))?;

            // SQLite records AUTOINCREMENT only in the table's CREATE statement;
            // `PRAGMA table_xinfo` does not report it, so `db pull` would drop
            // the `@default(autoincrement())` the column was created with.
            let has_autoincrement = create_sql.to_ascii_uppercase().contains("AUTOINCREMENT");

            let gen_exprs = parse_sqlite_generated_exprs(&create_sql);
            let (column_check_map, table_check_constraints) =
                parse_sqlite_check_constraints(&create_sql);

            let mut columns = Vec::new();
            let mut primary_key = Vec::new();

            for row in &col_rows {
                let col_name: String = row
                    .try_get("name")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let type_str: String = row
                    .try_get("type")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let notnull: i64 = row
                    .try_get("notnull")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let dflt_value: Option<String> = row
                    .try_get("dflt_value")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let pk_seq: i64 = row
                    .try_get("pk")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let hidden: i64 = row
                    .try_get("hidden")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;

                let col_type = normalize_sqlite_type(&type_str);
                let is_pk = pk_seq > 0;
                let nullable = notnull == 0 && !is_pk;

                if is_pk {
                    primary_key.push((pk_seq as i32, col_name.clone()));
                }

                let generated_expr = if hidden >= 2 {
                    gen_exprs.get(&col_name.to_lowercase()).cloned()
                } else {
                    None
                };
                let computed_kind = generated_expr.as_ref().map(|_| {
                    if hidden == 2 {
                        ComputedKind::Virtual
                    } else {
                        ComputedKind::Stored
                    }
                });

                columns.push(LiveColumn {
                    name: col_name.clone(),
                    col_type,
                    nullable,
                    default_value: dflt_value.map(|s| normalize_sqlite_default(&s)),
                    generated_expr,
                    computed_kind,
                    check_expr: column_check_map.get(&col_name.to_lowercase()).cloned(),
                    auto_increment: is_pk && has_autoincrement,
                    self_updating: false,
                });
            }

            primary_key.sort_by_key(|(seq, _)| *seq);
            let primary_key = primary_key.into_iter().map(|(_, name)| name).collect();

            let index_sql_rows = sqlx::query(
                "SELECT name, COALESCE(sql, '') AS create_sql FROM sqlite_master \
                 WHERE type = 'index' AND tbl_name = ?",
            )
            .bind(&table_name)
            .fetch_all(&pool)
            .await
            .map_err(|e| MigrationError::Database(e.to_string()))?;

            let mut index_predicates: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for row in &index_sql_rows {
                let name: String = row
                    .try_get("name")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let create_sql: String = row
                    .try_get("create_sql")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                if let Some(predicate) = parse_sqlite_index_predicate(&create_sql) {
                    index_predicates.insert(name, normalize_sqlite_check_expr(&predicate));
                }
            }

            let index_list_sql = format!("PRAGMA index_list({})", quote_ident(&table_name, '"'));
            let idx_list_rows = sqlx::query(&index_list_sql)
                .fetch_all(&pool)
                .await
                .map_err(|e| MigrationError::Database(e.to_string()))?;

            let mut indexes = Vec::new();
            for idx_row in &idx_list_rows {
                let idx_name: String = idx_row
                    .try_get("name")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let unique_val: i64 = idx_row
                    .try_get("unique")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let origin: String = idx_row
                    .try_get("origin")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;

                if origin == "pk" {
                    continue;
                }

                let index_info_sql = format!("PRAGMA index_info({})", quote_ident(&idx_name, '"'));
                let idx_info_rows = sqlx::query(&index_info_sql)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| MigrationError::Database(e.to_string()))?;

                let mut idx_cols = Vec::new();
                for irow in &idx_info_rows {
                    let seqno: i64 = irow
                        .try_get("seqno")
                        .map_err(|e| MigrationError::Database(e.to_string()))?;
                    let col: String = irow
                        .try_get("name")
                        .map_err(|e| MigrationError::Database(e.to_string()))?;
                    idx_cols.push((seqno, col));
                }
                idx_cols.sort_by_key(|(seq, _)| *seq);

                let predicate = index_predicates.get(&idx_name).cloned();
                indexes.push(LiveIndex {
                    name: idx_name,
                    columns: idx_cols.into_iter().map(|(_, col)| col).collect(),
                    unique: unique_val != 0,
                    kind: LiveIndexKind::Unknown(None),
                    predicate,
                });
            }

            let fk_pragma_sql =
                format!("PRAGMA foreign_key_list({})", quote_ident(&table_name, '"'));
            let fk_pragma_rows = sqlx::query(&fk_pragma_sql)
                .fetch_all(&pool)
                .await
                .map_err(|e| MigrationError::Database(e.to_string()))?;

            let foreign_keys = group_sqlite_foreign_keys(&table_name, fk_pragma_rows);

            live.tables.insert(
                TableName::new(table_name.clone()),
                LiveTable {
                    name: TableName::new(table_name),
                    columns,
                    primary_key,
                    indexes,
                    check_constraints: table_check_constraints,
                    foreign_keys,
                },
            );
        }

        let view_rows = sqlx::query(
            "SELECT name FROM sqlite_master              WHERE type = 'view'                AND name NOT LIKE 'sqlite_%'                AND name NOT LIKE '_nautilus_%'              ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .map_err(|e| MigrationError::Database(e.to_string()))?;

        for row in &view_rows {
            let view_name: String = row
                .try_get("name")
                .map_err(|e| MigrationError::Database(e.to_string()))?;

            let pragma_sql = format!("PRAGMA table_info({})", quote_ident(&view_name, '"'));
            let col_rows = sqlx::query(&pragma_sql)
                .fetch_all(&pool)
                .await
                .map_err(|e| MigrationError::Database(e.to_string()))?;

            let mut columns = Vec::with_capacity(col_rows.len());
            for col_row in &col_rows {
                let col_name: String = col_row
                    .try_get("name")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let type_str: String = col_row
                    .try_get("type")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
                let notnull: i64 = col_row
                    .try_get("notnull")
                    .map_err(|e| MigrationError::Database(e.to_string()))?;

                columns.push(LiveColumn {
                    name: col_name,
                    col_type: normalize_sqlite_type(&type_str),
                    nullable: notnull == 0,
                    default_value: None,
                    generated_expr: None,
                    computed_kind: None,
                    check_expr: None,
                    auto_increment: false,
                    self_updating: false,
                });
            }

            live.views.insert(
                TableName::new(view_name.clone()),
                LiveTable {
                    name: TableName::new(view_name),
                    columns,
                    primary_key: Vec::new(),
                    indexes: Vec::new(),
                    check_constraints: Vec::new(),
                    foreign_keys: Vec::new(),
                },
            );
        }

        Ok(live)
    }
}

/// Extracts the `WHERE` predicate of a partial index from the `CREATE INDEX`
/// text SQLite stores verbatim in `sqlite_master`.
///
/// `PRAGMA index_list` reports only that an index *is* partial, never the
/// predicate itself, so the statement text is the only source. The key column
/// list is always parenthesised and the predicate always follows the closing
/// paren, so scanning for the first `WHERE` at paren depth zero is enough.
fn parse_sqlite_index_predicate(create_sql: &str) -> Option<String> {
    let bytes = create_sql.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut i = 0usize;

    while i < bytes.len() {
        let ch = bytes[i] as char;
        if in_string {
            if ch == '\'' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match ch {
            '\'' => in_string = true,
            '(' => depth += 1,
            ')' => depth -= 1,
            'w' | 'W' if depth == 0 => {
                let rest = &create_sql[i..];
                if rest.len() >= 5
                    && rest[..5].eq_ignore_ascii_case("where")
                    && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
                    && rest[5..].starts_with(|c: char| c.is_whitespace() || c == '(')
                {
                    let predicate = rest[5..].trim().trim_end_matches(';').trim();
                    if predicate.is_empty() {
                        return None;
                    }
                    return Some(predicate.to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::parse_sqlite_index_predicate;

    #[test]
    fn full_index_has_no_predicate() {
        assert_eq!(
            parse_sqlite_index_predicate("CREATE INDEX \"i\" ON \"t\" (\"a\", \"b\")"),
            None
        );
    }

    #[test]
    fn partial_index_predicate_is_extracted() {
        assert_eq!(
            parse_sqlite_index_predicate(
                "CREATE INDEX \"i\" ON \"t\" (\"a\") WHERE \"active\" = 1"
            )
            .as_deref(),
            Some("\"active\" = 1")
        );
    }

    #[test]
    fn where_inside_a_string_literal_is_not_a_predicate() {
        assert_eq!(
            parse_sqlite_index_predicate("CREATE INDEX \"i\" ON \"t\" (\"a\") WHERE b <> 'where'")
                .as_deref(),
            Some("b <> 'where'")
        );
    }

    #[test]
    fn column_named_where_prefix_is_not_a_predicate() {
        assert_eq!(
            parse_sqlite_index_predicate("CREATE INDEX i ON t (whereabouts)"),
            None
        );
    }
}
