//! The single rule every Nautilus-emitted SQL identifier goes through.
//!
//! An identifier is delimited (`"name"` on PostgreSQL and SQLite, `` `name` ``
//! on MySQL) and any delimiter inside the name is doubled, so a physical name
//! containing a quote, a backtick, a space or a dot survives as exactly one
//! identifier. Callers that need a schema qualifier quote each part separately
//! and join them with an unquoted `.`, which is what keeps a literal dot inside
//! a name from being read as a qualifier.

use crate::table::TableName;

/// Append `name` to `sql` as a quoted identifier.
pub fn push_quoted_ident(sql: &mut String, name: &str, quote: char) {
    sql.push(quote);
    push_escaped_ident(sql, name, quote);
    sql.push(quote);
}

/// Quote `name` as an identifier.
pub fn quote_ident(name: &str, quote: char) -> String {
    let mut sql = String::with_capacity(name.len() + 2);
    push_quoted_ident(&mut sql, name, quote);
    sql
}

/// Append `segments` to `sql` as a single quoted identifier, as used by the
/// `table__column` aliases that keep joined columns apart.
pub fn push_quoted_ident_segments(sql: &mut String, segments: &[&str], quote: char) {
    sql.push(quote);
    for segment in segments {
        push_escaped_ident(sql, segment, quote);
    }
    sql.push(quote);
}

/// Append `table`.`column` to `sql`, each part quoted on its own.
pub fn push_qualified_ident(sql: &mut String, table: &str, column: &str, quote: char) {
    push_quoted_ident(sql, table, quote);
    sql.push('.');
    push_quoted_ident(sql, column, quote);
}

/// Append `table` to `sql` in a statement's table position, qualifying it with
/// its schema when it has one.
///
/// Column references keep using the bare table name: every supported provider
/// gives `schema.table` the bare `table` as its implicit alias.
pub fn push_table_name(sql: &mut String, table: &TableName, quote: char) {
    if let Some(schema) = table.schema() {
        push_quoted_ident(sql, schema, quote);
        sql.push('.');
    }
    push_quoted_ident(sql, &table.name, quote);
}

/// Quote `table` in a statement's table position.
pub fn quote_table_name(table: &TableName, quote: char) -> String {
    let mut sql = String::new();
    push_table_name(&mut sql, table, quote);
    sql
}

fn push_escaped_ident(sql: &mut String, name: &str, quote: char) {
    for ch in name.chars() {
        if ch == quote {
            sql.push(quote);
        }
        sql.push(ch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names every adapter of this rule is checked against.
    const CASES: &[&str] = &[
        "users",
        "user\"name",
        "user`name",
        "a\"b\"c",
        "a`b`c",
        "order details",
        "first.last",
        "",
        "quote\"and`tick",
    ];

    fn manual(name: &str, quote: char) -> String {
        let doubled: String = quote.to_string().repeat(2);
        format!(
            "{quote}{}{quote}",
            name.replace(quote, &doubled),
            quote = quote
        )
    }

    #[test]
    fn every_case_doubles_only_the_delimiter_in_use() {
        for quote in ['"', '`'] {
            for name in CASES {
                assert_eq!(quote_ident(name, quote), manual(name, quote), "{name}");
            }
        }
    }

    #[test]
    fn a_literal_dot_stays_inside_one_identifier() {
        assert_eq!(quote_ident("first.last", '"'), "\"first.last\"");
        assert_eq!(
            quote_table_name(&TableName::new("first.last"), '"'),
            "\"first.last\""
        );
    }

    #[test]
    fn a_schema_qualifier_quotes_each_part_apart() {
        assert_eq!(
            quote_table_name(&TableName::qualified("a\"b", "c.d"), '"'),
            "\"a\"\"b\".\"c.d\""
        );
        assert_eq!(
            quote_table_name(&TableName::qualified("a`b", "c.d"), '`'),
            "`a``b`.`c.d`"
        );
    }

    #[test]
    fn segments_and_qualified_parts_escape_the_same_way() {
        let mut alias = String::new();
        push_quoted_ident_segments(&mut alias, &["a\"b", "__", "c\"d"], '"');
        assert_eq!(alias, "\"a\"\"b__c\"\"d\"");

        let mut column = String::new();
        push_qualified_ident(&mut column, "a\"b", "c\"d", '"');
        assert_eq!(column, "\"a\"\"b\".\"c\"\"d\"");
    }
}
