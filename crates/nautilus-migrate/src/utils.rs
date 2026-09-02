//! Shared low-level utilities for `nautilus-migrate`.

/// Strip a single balanced outer layer of parentheses from `s`, if present.
///
/// Used when comparing DEFAULT expressions: SQLite versions differ on whether
/// they wrap expressions in parentheses in `PRAGMA table_info` output, and
/// PostgreSQL sometimes adds them too.  By stripping exactly one balanced
/// layer in both the live value and the schema-generated value, we avoid
/// false-positive [`crate::diff::Change::DefaultChanged`] detections.
///
/// Only strips if `s` begins with `(`, ends with `)`, and the opening paren
/// is closed by that final paren rather than by an earlier one: `(a)` becomes
/// `a`, while `(a) OR (b)` and `((a)(b))` are left alone. Parentheses inside
/// single-quoted literals are not counted.
pub(crate) fn strip_outer_parens(s: &str) -> String {
    if !(s.starts_with('(') && s.ends_with(')')) {
        return s.to_string();
    }

    let inner = &s[1..s.len() - 1];
    let mut depth: i32 = 0;
    let mut in_quotes = false;
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if in_quotes => {
                chars.next();
            }
            '\'' => in_quotes = !in_quotes,
            '(' if !in_quotes => depth += 1,
            ')' if !in_quotes => {
                depth -= 1;
                if depth < 0 {
                    return s.to_string();
                }
            }
            _ => {}
        }
    }

    if depth == 0 && !in_quotes {
        return inner.to_string();
    }
    s.to_string()
}

/// Lower-case a SQL fragment, leaving single-quoted literals untouched.
///
/// A MySQL `enum('DRAFT','PUBLISHED')` is reported by the server with the
/// variants spelled as declared, so folding their case would make the type
/// compare unequal to the one the DDL generator emits.
pub(crate) fn lowercase_outside_quotes(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut in_quotes = false;
    for ch in sql.chars() {
        if ch == '\'' {
            in_quotes = !in_quotes;
            out.push(ch);
        } else if in_quotes {
            out.push(ch);
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_simple_parens() {
        assert_eq!(strip_outer_parens("(42)"), "42");
    }

    #[test]
    fn strips_nested_inner_parens() {
        assert_eq!(strip_outer_parens("((a)(b))"), "(a)(b)");
    }

    #[test]
    fn no_strip_unbalanced() {
        assert_eq!(strip_outer_parens("(a(b)"), "(a(b)");
    }

    #[test]
    fn no_strip_when_the_first_paren_closes_early() {
        assert_eq!(
            strip_outer_parens("(notes IS NULL) OR (char_length(notes) > 3)"),
            "(notes IS NULL) OR (char_length(notes) > 3)"
        );
    }

    #[test]
    fn parens_inside_string_literals_do_not_count() {
        assert_eq!(strip_outer_parens("(code LIKE 'a)b')"), "code LIKE 'a)b'");
    }

    #[test]
    fn no_strip_without_parens() {
        assert_eq!(strip_outer_parens("hello"), "hello");
    }

    #[test]
    fn no_strip_empty() {
        assert_eq!(strip_outer_parens(""), "");
    }
}

/// Re-render a boolean expression written in the Nautilus schema dialect as
/// plain SQL.
///
/// The inspector stores index predicates and CHECK expressions in the schema
/// dialect (`status IN ['A', 'B']`, `active = TRUE`) so they can be written
/// straight back out by `db pull`. Down-migrations rebuild the same index from
/// that snapshot and need real SQL again, which is what
/// [`nautilus_schema::bool_expr::BoolExpr::to_sql`] produces. Expressions the
/// parser does not understand are passed through unchanged.
pub(crate) fn schema_bool_expr_to_sql(expr: &str) -> String {
    let mut lexer = nautilus_schema::Lexer::new(expr);
    let mut tokens = Vec::new();
    loop {
        match lexer.next_token() {
            Ok(token) if matches!(token.kind, nautilus_schema::TokenKind::Eof) => break,
            Ok(token) => tokens.push(token),
            Err(_) => return expr.to_string(),
        }
    }

    nautilus_schema::bool_expr::parse_bool_expr(&tokens, nautilus_schema::Span::new(0, expr.len()))
        .map(|parsed| parsed.to_sql())
        .unwrap_or_else(|_| expr.to_string())
}

#[cfg(test)]
mod bool_expr_tests {
    use super::schema_bool_expr_to_sql;

    #[test]
    fn bracket_in_list_becomes_sql_in_list() {
        assert_eq!(
            schema_bool_expr_to_sql("status IN [ACTIVE, PENDING]"),
            "status IN ('ACTIVE', 'PENDING')"
        );
    }

    #[test]
    fn boolean_comparison_round_trips() {
        assert_eq!(schema_bool_expr_to_sql("active = true"), "active = TRUE");
    }

    #[test]
    fn unparseable_expression_is_passed_through() {
        assert_eq!(
            schema_bool_expr_to_sql("lower(name) LIKE 'a%'"),
            "lower(name) LIKE 'a%'"
        );
    }
}

/// Whether a DDL statement has to run on its own, outside the migration's
/// transaction.
///
/// PostgreSQL refuses to use an enum value that was added by the transaction
/// still in flight ("unsafe use of new value"), so adding a variant and making
/// it a column default — one ordinary schema edit — can never be applied in one
/// go. `ALTER TYPE … ADD VALUE` is therefore committed first and separately.
/// The check is textual because the migration-file path only ever sees SQL
/// text; no other provider emits this statement.
pub fn requires_own_transaction(sql: &str) -> bool {
    let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    let upper = normalized.to_uppercase();
    upper.starts_with("ALTER TYPE ") && upper.contains(" ADD VALUE ")
}

#[cfg(test)]
mod own_transaction_tests {
    use super::requires_own_transaction;

    #[test]
    fn enum_add_value_runs_alone() {
        assert!(requires_own_transaction(
            "ALTER TYPE \"shade\" ADD VALUE IF NOT EXISTS 'blue'"
        ));
    }

    #[test]
    fn other_statements_stay_in_the_transaction() {
        assert!(!requires_own_transaction(
            "ALTER TABLE \"Box\" ALTER COLUMN \"shade\" SET DEFAULT 'blue'"
        ));
        assert!(!requires_own_transaction(
            "ALTER TYPE \"shade\" RENAME TO \"shade_old\""
        ));
    }
}
