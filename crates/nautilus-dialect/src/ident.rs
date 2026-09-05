//! Text written straight into a statement rather than bound as a parameter:
//! identifiers, column aliases, JSON path keys, string literals and integers.
//!
//! Quoting itself belongs to `nautilus_core::ident`, which every provider shares;
//! this module adds the shapes only a rendered statement needs.

/// Alias of the row-number column a [`nautilus_core::PartitionWindow`] adds to
/// the inner select. Never projected by the outer query, so callers see the same
/// columns they would without a window.
pub(crate) const WINDOW_ROW_NUMBER_ALIAS: &str = "__nautilus_rn";

/// Alias of the subquery a [`nautilus_core::PartitionWindow`] wraps the select in.
pub(crate) const WINDOW_SUBQUERY_ALIAS: &str = "__nautilus_win";

/// Identifier quoting, owned by [`nautilus_core::ident`] so that migrations,
/// introspection and rendered queries all delimit and escape a name the same way.
pub(crate) use nautilus_core::ident::{
    push_qualified_ident as push_qualified_identifier, push_quoted_ident as push_quoted_identifier,
    push_table_name,
};

/// Render a join-safe `table__column` alias directly into the SQL buffer.
pub(crate) fn push_column_alias(
    sql: &mut String,
    column: &nautilus_core::ColumnMarker,
    quote: char,
) {
    nautilus_core::ident::push_quoted_ident_segments(
        sql,
        &[column.table.as_ref(), "__", column.name.as_ref()],
        quote,
    );
}

/// Render an identifier reference that may use the `table__column` shorthand.
///
/// The split happens only on the first `__`, so mapped column names like
/// `users__profile__slug` still render as `users.profile__slug`.
pub(crate) fn push_identifier_reference(sql: &mut String, name: &str, quote: char) {
    if let Some((table, column)) = name.split_once("__") {
        push_qualified_identifier(sql, table, column, quote);
    } else {
        push_quoted_identifier(sql, name, quote);
    }
}

/// Render a PostgreSQL native composite field reference.
pub(crate) fn push_composite_field_reference(
    sql: &mut String,
    table: &str,
    column: &str,
    field: &str,
    quote: char,
) {
    sql.push('(');
    push_qualified_identifier(sql, table, column, quote);
    sql.push(')');
    sql.push('.');
    push_quoted_identifier(sql, field, quote);
}

fn push_json_path_key(sql: &mut String, key: &str) {
    sql.push_str("$.\"");
    for ch in key.chars() {
        match ch {
            '"' | '\\' => {
                sql.push('\\');
                sql.push(ch);
            }
            other => sql.push(other),
        }
    }
    sql.push('"');
}

/// Render a single-quoted JSON object path literal for a schema-known key.
pub(crate) fn push_json_object_path_literal(sql: &mut String, key: &str) {
    let mut path = String::with_capacity(key.len() + 4);
    push_json_path_key(&mut path, key);
    push_sql_string_literal(sql, &path);
}

/// Render a single-quoted SQL string literal directly into the SQL buffer.
pub(crate) fn push_sql_string_literal(sql: &mut String, value: &str) {
    sql.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            sql.push('\'');
        }
        sql.push(ch);
    }
    sql.push('\'');
}

/// Append a `u64` value directly into the SQL buffer.
pub(crate) fn push_u64(sql: &mut String, mut value: u64) {
    let mut digits = [0_u8; 20];
    let mut idx = digits.len();

    loop {
        idx -= 1;
        digits[idx] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }

    for digit in &digits[idx..] {
        sql.push(char::from(*digit));
    }
}

/// Append a `u32` value directly into the SQL buffer.
pub(crate) fn push_u32(sql: &mut String, value: u32) {
    push_u64(sql, u64::from(value));
}

/// Append a `usize` value directly into the SQL buffer.
pub(crate) fn push_usize(sql: &mut String, value: usize) {
    push_u64(sql, value as u64);
}
