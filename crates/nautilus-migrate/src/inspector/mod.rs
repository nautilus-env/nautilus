//! Live-schema inspector — queries a running database to build a [`LiveSchema`].

mod mysql;
mod postgres;
mod postgres_indexes;
mod sqlite;

pub(super) use postgres_indexes::group_pg_indexes;

use crate::ddl::DatabaseProvider;
use crate::error::Result;
use crate::live::{LiveForeignKey, LiveSchema};
use crate::normalize::predicates::normalize_sqlite_check_expr;
use nautilus_core::TableName;

/// Inspects a live database and returns a snapshot of its current schema.
pub struct SchemaInspector {
    provider: DatabaseProvider,
    url: String,
    schemas: Vec<String>,
}

impl SchemaInspector {
    /// Create a new inspector for the given provider and connection URL.
    pub fn new(provider: DatabaseProvider, url: impl Into<String>) -> Self {
        Self {
            provider,
            url: url.into(),
            schemas: Vec::new(),
        }
    }

    /// Restrict introspection to the PostgreSQL schemas the datasource declares.
    ///
    /// An empty list keeps the single-schema behaviour: only `current_schema()`
    /// is scanned and its tables come back unqualified. With a list, every named
    /// schema is scanned and each table carries its schema, so two tables of the
    /// same name in different schemas stay distinct.
    #[must_use]
    pub fn with_schemas(mut self, schemas: Vec<String>) -> Self {
        self.schemas = schemas;
        self
    }

    /// Connect to the database and return the current [`LiveSchema`].
    pub async fn inspect(&self) -> Result<LiveSchema> {
        match self.provider {
            DatabaseProvider::Postgres => self.inspect_postgres().await,
            DatabaseProvider::Sqlite => self.inspect_sqlite().await,
            DatabaseProvider::Mysql => self.inspect_mysql().await,
        }
    }
}

/// Parse generation expressions from a SQLite CREATE TABLE statement.
///
/// Returns a map of lower-cased column name -> expression body with original
/// expression casing preserved.
/// Looks for patterns like `col_name TYPE AS (expr) STORED` or `... VIRTUAL`.
fn parse_sqlite_generated_exprs(create_sql: &str) -> std::collections::HashMap<String, String> {
    let mut result = std::collections::HashMap::new();
    let lower = create_sql.to_lowercase();

    let start = match lower.find('(') {
        Some(i) => i + 1,
        None => return result,
    };

    let bytes = create_sql.as_bytes();
    let mut depth = 0i32;
    let mut seg_start = start;

    for i in start..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' if depth == 0 => {
                let seg = create_sql[seg_start..i].trim();
                if let Some((name, expr)) = extract_generated_col(seg) {
                    result.insert(name.to_lowercase(), expr);
                }
                break;
            }
            b')' => depth -= 1,
            b',' if depth == 0 => {
                let seg = create_sql[seg_start..i].trim();
                if let Some((name, expr)) = extract_generated_col(seg) {
                    result.insert(name.to_lowercase(), expr);
                }
                seg_start = i + 1;
            }
            _ => {}
        }
    }

    result
}

/// Extract the generation expression from a single column definition segment.
///
/// Looks for `... AS (expr) STORED` or `... AS (expr) VIRTUAL` (case-insensitive).
/// Returns `(column_name, expression)` with original casing preserved.
fn extract_generated_col(col_def: &str) -> Option<(String, String)> {
    let col_lower = col_def.to_lowercase();
    let as_idx = col_lower.find(" as (")?;
    let name = col_def
        .split_whitespace()
        .next()?
        .trim_matches('"')
        .to_string();

    let expr_start = as_idx + 5;
    let mut depth = 1i32;
    for (i, ch) in col_def[expr_start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let expr = col_def[expr_start..expr_start + i].trim().to_string();
                    return Some((name, expr));
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse CHECK constraints from a SQLite CREATE TABLE statement.
///
/// Returns `(column_checks, table_checks)` where `column_checks` maps
/// lower-cased column name -> normalized expression body and `table_checks`
/// holds normalized table-level expressions. Original expression casing is
/// preserved.
fn parse_sqlite_check_constraints(
    create_sql: &str,
) -> (std::collections::HashMap<String, String>, Vec<String>) {
    let mut column_checks: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut table_checks: Vec<String> = Vec::new();

    let lower = create_sql.to_lowercase();
    let start = match lower.find('(') {
        Some(i) => i + 1,
        None => return (column_checks, table_checks),
    };

    let bytes = create_sql.as_bytes();
    let mut depth = 0i32;
    let mut seg_start = start;
    let mut segments: Vec<String> = Vec::new();

    for i in start..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' if depth == 0 => {
                segments.push(create_sql[seg_start..i].trim().to_string());
                break;
            }
            b')' => depth -= 1,
            b',' if depth == 0 => {
                segments.push(create_sql[seg_start..i].trim().to_string());
                seg_start = i + 1;
            }
            _ => {}
        }
    }

    for seg in &segments {
        if seg.is_empty() {
            continue;
        }
        let first_keyword: String = seg
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_alphabetic())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let is_table_constraint = matches!(
            first_keyword.as_str(),
            "check" | "constraint" | "primary" | "unique" | "foreign"
        );

        if is_table_constraint {
            if let Some(expr) = extract_sqlite_check_expr(seg) {
                table_checks.push(normalize_sqlite_check_expr(&expr));
            }
        } else {
            let col_name = seg
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('"')
                .to_string();
            if let Some(expr) = extract_sqlite_check_expr(seg) {
                column_checks.insert(col_name.to_lowercase(), normalize_sqlite_check_expr(&expr));
            }
        }
    }

    (column_checks, table_checks)
}

/// Extract the expression body from the first `CHECK (…)` or `CHECK(…)` pattern in `seg`.
///
/// The `CHECK` keyword is matched case-insensitively; the returned expression
/// body preserves the original casing from `seg`. Both `CHECK (` (with space)
/// and `CHECK(` (without space) are accepted, since SQLite stores the CREATE
/// TABLE SQL verbatim as the user wrote it.
fn extract_sqlite_check_expr(seg: &str) -> Option<String> {
    let seg_lower = seg.to_lowercase();
    let (check_pos, content_offset) = if let Some(p) = seg_lower.find("check (") {
        (p, 7usize)
    } else {
        let p = seg_lower.find("check(")?;
        (p, 6usize)
    };
    let after = &seg[check_pos + content_offset..];
    let mut depth = 1i32;
    for (i, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(after[..i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Group raw PostgreSQL FK rows (one row per FK column) into
/// [`LiveForeignKey`] values.
///
/// `qualified` mirrors how the tables themselves are keyed: a single-schema
/// inspection leaves every name bare, so stamping the referenced table with its
/// schema there would make it point at a table the snapshot does not hold.
fn group_pg_foreign_keys(rows: Vec<sqlx::postgres::PgRow>, qualified: bool) -> Vec<LiveForeignKey> {
    use sqlx::Row as _;

    let mut ordered: Vec<String> = Vec::new();
    let mut map: std::collections::HashMap<String, LiveForeignKey> =
        std::collections::HashMap::new();

    for row in rows {
        let con_name: String = row.try_get("constraint_name").unwrap_or_default();
        let col: String = row.try_get("column_name").unwrap_or_default();
        let ref_table: String = row.try_get("referenced_table").unwrap_or_default();
        let ref_schema: Option<String> = qualified
            .then(|| row.try_get("referenced_schema").ok().flatten())
            .flatten();
        let ref_col: String = row.try_get("referenced_column").unwrap_or_default();
        let del_type: String = row.try_get("delete_type").unwrap_or_default();
        let upd_type: String = row.try_get("update_type").unwrap_or_default();

        if !ordered.contains(&con_name) {
            ordered.push(con_name.clone());
        }

        let entry = map
            .entry(con_name.clone())
            .or_insert_with(|| LiveForeignKey {
                constraint_name: con_name,
                columns: Vec::new(),
                referenced_table: TableName::with_schema(ref_schema, ref_table),
                referenced_columns: Vec::new(),
                on_delete: pg_fk_action(&del_type),
                on_update: pg_fk_action(&upd_type),
            });
        entry.columns.push(col);
        entry.referenced_columns.push(ref_col);
    }

    ordered
        .into_iter()
        .filter_map(|name| map.remove(&name))
        .collect()
}

/// Decode a PostgreSQL single-character FK action code into a SQL string.
fn pg_fk_action(code: &str) -> Option<String> {
    match code {
        "a" => None,
        "r" => Some("RESTRICT".to_string()),
        "c" => Some("CASCADE".to_string()),
        "n" => Some("SET NULL".to_string()),
        "d" => Some("SET DEFAULT".to_string()),
        _ => None,
    }
}

/// Group SQLite `PRAGMA foreign_key_list` rows into [`LiveForeignKey`] values.
fn group_sqlite_foreign_keys(
    table_name: &str,
    rows: Vec<sqlx::sqlite::SqliteRow>,
) -> Vec<LiveForeignKey> {
    use sqlx::Row as _;

    type SqliteFkColumn = (i64, String, String);
    type SqliteFkGroup = (String, Vec<SqliteFkColumn>, String, String);

    let mut ordered: Vec<i64> = Vec::new();
    let mut map: std::collections::HashMap<i64, SqliteFkGroup> = std::collections::HashMap::new();

    for row in rows {
        let id: i64 = row.try_get("id").unwrap_or(0);
        let seq: i64 = row.try_get("seq").unwrap_or(0);
        let ref_table: String = row.try_get("table").unwrap_or_default();
        let from_col: String = row.try_get("from").unwrap_or_default();
        let to_col: String = row.try_get("to").unwrap_or_default();
        let on_update: String = row.try_get("on_update").unwrap_or_default();
        let on_delete: String = row.try_get("on_delete").unwrap_or_default();

        if !ordered.contains(&id) {
            ordered.push(id);
        }
        let entry = map
            .entry(id)
            .or_insert_with(|| (ref_table, Vec::new(), on_delete, on_update));
        entry.1.push((seq, from_col, to_col));
    }

    let mut result: Vec<LiveForeignKey> = Vec::new();
    for fk_id in ordered {
        if let Some((ref_table, mut cols, on_delete, on_update)) = map.remove(&fk_id) {
            cols.sort_by_key(|(seq, _, _)| *seq);
            let fk_cols: Vec<String> = cols.iter().map(|(_, f, _)| f.clone()).collect();
            let ref_cols: Vec<String> = cols.iter().map(|(_, _, t)| t.clone()).collect();
            let constraint_name = fk_auto_name(table_name, &fk_cols);
            result.push(LiveForeignKey {
                constraint_name,
                columns: fk_cols,
                referenced_table: TableName::new(ref_table),
                referenced_columns: ref_cols,
                on_delete: sqlite_fk_action(&on_delete),
                on_update: sqlite_fk_action(&on_update),
            });
        }
    }
    result
}

/// Normalise a SQLite FK action string.
fn sqlite_fk_action(s: &str) -> Option<String> {
    match s.to_uppercase().as_str() {
        "NO ACTION" | "" => None,
        other => Some(other.to_string()),
    }
}

/// Group MySQL FK rows into [`LiveForeignKey`] values.
fn group_mysql_foreign_keys(rows: Vec<sqlx::mysql::MySqlRow>) -> Vec<LiveForeignKey> {
    use sqlx::Row as _;

    let mut ordered: Vec<String> = Vec::new();
    let mut map: std::collections::HashMap<String, LiveForeignKey> =
        std::collections::HashMap::new();

    for row in rows {
        let con_name: String = row.try_get("constraint_name").unwrap_or_default();
        let col: String = row.try_get("column_name").unwrap_or_default();
        let ref_table: String = row.try_get("referenced_table_name").unwrap_or_default();
        let ref_col: String = row.try_get("referenced_column_name").unwrap_or_default();
        let del_rule: String = row.try_get("delete_rule").unwrap_or_default();
        let upd_rule: String = row.try_get("update_rule").unwrap_or_default();

        if !ordered.contains(&con_name) {
            ordered.push(con_name.clone());
        }
        let entry = map
            .entry(con_name.clone())
            .or_insert_with(|| LiveForeignKey {
                constraint_name: con_name,
                columns: Vec::new(),
                referenced_table: TableName::new(ref_table),
                referenced_columns: Vec::new(),
                on_delete: mysql_fk_action(&del_rule),
                on_update: mysql_fk_action(&upd_rule),
            });
        entry.columns.push(col);
        entry.referenced_columns.push(ref_col);
    }

    ordered
        .into_iter()
        .filter_map(|name| map.remove(&name))
        .collect()
}

/// Normalise a MySQL FK action rule string.
fn mysql_fk_action(s: &str) -> Option<String> {
    match s.to_uppercase().as_str() {
        "NO ACTION" | "" => None,
        other => Some(other.to_string()),
    }
}

/// Derive an auto-generated FK constraint name from table and FK column list.
fn fk_auto_name(table: &str, columns: &[String]) -> String {
    format!("fk_{}_{}", table, columns.join("_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_generated_expr_preserves_expression_casing() {
        let exprs = parse_sqlite_generated_exprs(
            r#"CREATE TABLE users (
  "Id" INTEGER PRIMARY KEY,
  "FullName" TEXT AS ("FirstName" || ' ' || "LastName") STORED
)"#,
        );

        assert_eq!(
            exprs.get("fullname"),
            Some(&r#""FirstName" || ' ' || "LastName""#.to_string())
        );
    }

    #[test]
    fn sqlite_check_parser_preserves_literals_and_uses_lowercase_keys() {
        let (column_checks, table_checks) = parse_sqlite_check_constraints(
            r#"CREATE TABLE users (
  "Status" TEXT CHECK ("Status" IN ('Draft', 'PUBLISHED')),
  "Role" TEXT,
  CHECK ("Role" IN ('ADMIN', 'User'))
)"#,
        );

        assert_eq!(
            column_checks.get("status"),
            Some(&r#""Status" IN ['Draft', 'PUBLISHED']"#.to_string())
        );
        assert_eq!(
            table_checks,
            vec![r#""Role" IN ['ADMIN', 'User']"#.to_string()]
        );
    }

    #[test]
    fn sqlite_check_no_space_before_paren() {
        let (column_checks, table_checks) = parse_sqlite_check_constraints(
            r#"CREATE TABLE users (
  "Status" TEXT CHECK("Status" IN ('Draft', 'PUBLISHED')),
  CHECK("age" > 0)
)"#,
        );

        assert_eq!(
            column_checks.get("status"),
            Some(&r#""Status" IN ['Draft', 'PUBLISHED']"#.to_string())
        );
        assert_eq!(table_checks, vec![r#""age" > 0"#.to_string()]);
    }

    #[test]
    fn sqlite_check_constraint_no_space_table_level() {
        let (column_checks, table_checks) = parse_sqlite_check_constraints(
            r#"CREATE TABLE orders (
  id INTEGER PRIMARY KEY,
  quantity INTEGER,
  CONSTRAINT chk_qty CHECK(quantity > 0)
)"#,
        );

        assert!(column_checks.is_empty());
        assert_eq!(table_checks, vec!["quantity > 0".to_string()]);
    }
}
