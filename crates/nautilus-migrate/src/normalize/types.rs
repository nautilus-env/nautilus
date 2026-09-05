//! Column types: the canonical spelling of a live column's SQL type, and
//! whether a live type and a target type describe the same column.

use super::predicates::strip_identifier_quotes;
use crate::ddl::DatabaseProvider;

/// Normalise a Postgres column type to the same canonical form that
/// `DdlGenerator::column_type_sql` produces (lower-cased).
pub(crate) fn normalize_pg_type(
    udt_name: &str,
    numeric_precision: Option<i32>,
    numeric_scale: Option<i32>,
    character_maximum_length: Option<i32>,
    formatted_type: Option<&str>,
) -> String {
    if let Some(formatted) = formatted_type.and_then(normalize_pgvector_formatted_type) {
        return formatted;
    }

    match udt_name.to_lowercase().as_str() {
        "int4" => "integer".to_string(),
        "int8" => "bigint".to_string(),
        "text" => "text".to_string(),
        "bool" => "boolean".to_string(),
        "timestamp" => "timestamp".to_string(),
        "float8" => "double precision".to_string(),
        "jsonb" => "jsonb".to_string(),
        "uuid" => "uuid".to_string(),
        "bytea" => "bytea".to_string(),
        "varchar" => character_maximum_length
            .map(|length| format!("varchar({length})"))
            .unwrap_or_else(|| "varchar".to_string()),
        "bpchar" => character_maximum_length
            .map(|length| format!("char({length})"))
            .unwrap_or_else(|| "char".to_string()),
        "numeric" => match (numeric_precision, numeric_scale) {
            (Some(p), Some(s)) => format!("decimal({p}, {s})"),
            _ => "decimal".to_string(),
        },
        udt if udt.starts_with('_') => {
            let base = normalize_pg_type(
                &udt[1..],
                numeric_precision,
                numeric_scale,
                character_maximum_length,
                None,
            );
            format!("{base}[]")
        }
        other => other.to_string(),
    }
}

fn normalize_pgvector_formatted_type(formatted_type: &str) -> Option<String> {
    let t = formatted_type.trim().to_lowercase();
    let array_suffix = t.strip_suffix("[]").map(|inner| (inner, "[]"));
    let (base, suffix) = array_suffix.unwrap_or((t.as_str(), ""));
    let base = base.rsplit('.').next().unwrap_or(base);

    if base == "vector" {
        return Some(format!("vector{}", suffix));
    }

    if base.starts_with("vector(") && base.ends_with(')') {
        return Some(format!("{}{}", base, suffix));
    }

    None
}

/// Normalise a `pg_catalog.format_type` output to the same canonical form that
/// `DdlGenerator::column_type_sql` produces so that live composite-type fields
/// can be compared against target schema fields without false positives.
///
/// `format_type` is the authoritative human-readable representation Postgres
/// uses for types, but it uses longer forms (`timestamp without time zone`,
/// `character varying(n)`) that differ from what the DDL generator emits.
pub(crate) fn normalize_pg_composite_field_type(s: &str) -> String {
    let t = s.trim().to_lowercase();
    if t == "timestamp without time zone" || t == "timestamp with time zone" {
        return "timestamp".to_string();
    }
    if let Some(vector) = normalize_pgvector_formatted_type(&t) {
        return vector;
    }
    if let Some(rest) = t.strip_prefix("character varying") {
        return format!("varchar{}", rest.trim());
    }
    if let Some(inner) = t.strip_prefix("character(") {
        return format!("char({}", inner);
    }
    t
}

/// Normalise a SQLite column type (PRAGMA table_info).
pub(crate) fn normalize_sqlite_type(type_str: &str) -> String {
    let s = type_str.to_lowercase();
    if let Some(pos) = s.find(" primary") {
        s[..pos].trim().to_string()
    } else {
        s.trim().to_string()
    }
}

/// Normalise a MySQL `column_type` value to the canonical form used by
/// `DdlGenerator::column_type_sql` (lower-cased).
pub(crate) fn normalize_mysql_type(column_type: &str) -> String {
    let s = crate::utils::lowercase_outside_quotes(column_type);
    if s == "tinyint(1)" {
        return "boolean".to_string();
    }
    let integer_prefixes = ["int(", "bigint(", "tinyint(", "smallint(", "mediumint("];
    for prefix in &integer_prefixes {
        if s.starts_with(prefix) {
            return prefix.trim_end_matches('(').to_string();
        }
    }
    s
}

/// Whether a live column type and the target type describe the same column.
///
/// The two sides spell the same type differently in ways that carry no
/// meaning, and comparing them literally makes every push after the first
/// report a destructive `TypeChanged` for a column that never changed:
///
/// - The DDL generator quotes user-defined type names (`"address"`) because it
///   writes them straight into statements; introspection reports `address`.
/// - The generator emits `decimal(6, 2)`; MySQL reports `decimal(6,2)`.
///
/// Case is deliberately preserved: a quoted PostgreSQL type name is
/// case-sensitive, so `"Address"` and `"address"` are genuinely different.
pub(crate) fn column_types_match(provider: DatabaseProvider, live: &str, target: &str) -> bool {
    if live == target || types_storage_equivalent(provider, live, target) {
        return true;
    }
    normalize_column_type(live) == normalize_column_type(target)
}

fn normalize_column_type(s: &str) -> String {
    strip_identifier_quotes(s)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Returns `true` when `a` and `b` are storage-equivalent for the given provider,
/// meaning no data migration is needed despite differing declared type names.
///
/// Handles the SQLite backwards-compatibility case: older Nautilus versions
/// used bare `TEXT` for `DateTime`, `Uuid`, and `Json` columns, while newer
/// versions use `DATETIME`, `CHAR(36)`, and `JSON` respectively.  All of these
/// are stored identically on disk by SQLite (TEXT/NUMERIC affinity - text
/// storage for non-numeric values), so they must not trigger a rebuild.
fn types_storage_equivalent(provider: DatabaseProvider, a: &str, b: &str) -> bool {
    if provider != DatabaseProvider::Sqlite {
        return false;
    }
    // Canonical set of SQLite text-storage types Nautilus may produce.
    // `text` is the old spelling; the others are the new descriptive names.
    fn sqlite_text_group(t: &str) -> bool {
        matches!(t, "text" | "datetime" | "json")
            || t == "char(36)"
            || t.starts_with("varchar")
            || t.starts_with("char(")
    }
    // Two decimal(p,s) spellings whose precision/scale may differ are NOT
    // equivalent — keep them as real type changes.
    fn sqlite_decimal_group(t: &str) -> bool {
        t.starts_with("decimal(")
    }
    // `TEXT` (old)  any descriptive text-affinity type (new) - equivalent.
    if sqlite_text_group(a) && sqlite_text_group(b) {
        return true;
    }
    // `TEXT` (old)  `DECIMAL(p,s)` (new) - equivalent (both stored as text).
    if (a == "text" && sqlite_decimal_group(b)) || (sqlite_decimal_group(a) && b == "text") {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_int4() {
        assert_eq!(normalize_pg_type("int4", None, None, None, None), "integer");
    }

    #[test]
    fn pg_int8() {
        assert_eq!(normalize_pg_type("int8", None, None, None, None), "bigint");
    }

    #[test]
    fn pg_text() {
        assert_eq!(normalize_pg_type("text", None, None, None, None), "text");
    }

    #[test]
    fn pg_bool() {
        assert_eq!(normalize_pg_type("bool", None, None, None, None), "boolean");
    }

    #[test]
    fn pg_float8() {
        assert_eq!(
            normalize_pg_type("float8", None, None, None, None),
            "double precision"
        );
    }

    #[test]
    fn pg_numeric_with_precision() {
        assert_eq!(
            normalize_pg_type("numeric", Some(10), Some(2), None, None),
            "decimal(10, 2)"
        );
    }

    #[test]
    fn pg_numeric_without_precision() {
        assert_eq!(
            normalize_pg_type("numeric", None, None, None, None),
            "decimal"
        );
    }

    #[test]
    fn pg_array_type() {
        assert_eq!(
            normalize_pg_type("_int4", None, None, None, None),
            "integer[]"
        );
        assert_eq!(normalize_pg_type("_text", None, None, None, None), "text[]");
    }

    #[test]
    fn pg_uuid() {
        assert_eq!(normalize_pg_type("uuid", None, None, None, None), "uuid");
    }

    #[test]
    fn pgvector_dimension_from_formatted_type() {
        assert_eq!(
            normalize_pg_type("vector", None, None, None, Some("vector(1536)")),
            "vector(1536)"
        );
        assert_eq!(
            normalize_pg_type("_vector", None, None, None, Some("vector(3)[]")),
            "vector(3)[]"
        );
    }

    #[test]
    fn pg_enum_passthrough() {
        assert_eq!(
            normalize_pg_type("my_custom_enum", None, None, None, None),
            "my_custom_enum"
        );
    }

    #[test]
    fn pg_varchar_with_length() {
        assert_eq!(
            normalize_pg_type("varchar", None, None, Some(30), None),
            "varchar(30)"
        );
    }

    #[test]
    fn pg_char_with_length() {
        assert_eq!(
            normalize_pg_type("bpchar", None, None, Some(12), None),
            "char(12)"
        );
    }

    #[test]
    fn sqlite_type_lowercases() {
        assert_eq!(normalize_sqlite_type("TEXT"), "text");
        assert_eq!(normalize_sqlite_type("INTEGER"), "integer");
    }

    #[test]
    fn sqlite_type_strips_pk_suffix() {
        assert_eq!(
            normalize_sqlite_type("INTEGER PRIMARY KEY AUTOINCREMENT"),
            "integer"
        );
    }

    #[test]
    fn mysql_tinyint1_is_boolean() {
        assert_eq!(normalize_mysql_type("tinyint(1)"), "boolean");
    }

    #[test]
    fn mysql_strips_int_display_width() {
        assert_eq!(normalize_mysql_type("int(11)"), "int");
        assert_eq!(normalize_mysql_type("bigint(20)"), "bigint");
    }

    #[test]
    fn mysql_keeps_varchar() {
        assert_eq!(normalize_mysql_type("varchar(255)"), "varchar(255)");
    }

    #[test]
    fn mysql_keeps_decimal() {
        assert_eq!(normalize_mysql_type("decimal(10,2)"), "decimal(10,2)");
    }
}
