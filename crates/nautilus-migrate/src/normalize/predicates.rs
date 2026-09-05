//! Boolean SQL expressions: `CHECK` constraints, partial-index predicates and
//! generated-column expressions.

use nautilus_schema::{Lexer, Span, TokenKind};

use super::sql_text::{
    collapse_whitespace, convert_any_array_to_in, convert_in_parens_to_brackets,
    strip_all_outer_parens, strip_casts, strip_numeric_paren_literals,
};

/// Extract and normalise the expression body from a PostgreSQL CHECK constraint definition.
pub(crate) fn normalize_pg_check_expr(constraint_def: &str) -> String {
    let s = constraint_def.trim();
    let s_lower = s.to_lowercase();

    let s = if let Some(inner_lower) = s_lower.strip_prefix("check (") {
        let inner = &s[7..];
        if inner_lower.ends_with(')') {
            &inner[..inner.len() - 1]
        } else {
            inner
        }
    } else {
        s
    };

    let s = strip_casts(s.trim());
    let s = strip_numeric_paren_literals(&s);
    let s = strip_all_outer_parens(s.trim());

    let s = collapse_whitespace(&s);
    let s = convert_any_array_to_in(&s);
    convert_in_parens_to_brackets(&s)
}

/// Normalise a MySQL CHECK expression into a form the Nautilus schema parser accepts.
///
/// MySQL stores `CHECK_CLAUSE` in `information_schema` with backtick-quoted
/// identifiers (e.g. `` `status` in ('Draft', 'PUBLISHED') ``), redundant outer
/// parentheses, and lowercased SQL keywords. Normalisation strips the backtick
/// quoting, removes outer parentheses, collapses whitespace, and converts SQL
/// `IN (...)` syntax to the Nautilus bracket form `IN [...]`.
pub(crate) fn normalize_mysql_check_expr(expr: &str) -> String {
    let s = unescape_mysql_check_clause(expr.trim());
    let s = strip_mysql_charset_introducers(&s);
    let s = strip_mysql_backtick_quotes(&s);
    let s = strip_all_outer_parens(&s);
    let s = collapse_whitespace(&s);
    convert_in_parens_to_brackets(&s)
}

/// Undo the extra layer of backslash escaping MySQL applies to a stored
/// `CHECK_CLAUSE`.
///
/// `information_schema` reports `upper(`code`) like _utf8mb4\'DEV-%\'` — the
/// quotes delimiting the literal are escaped, and so is every backslash inside
/// it. That text is not valid SQL, so a constraint pulled from MySQL could not
/// be pushed back until the escaping is removed.
fn unescape_mysql_check_clause(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some('\'' | '\\') = chars.peek() {
                out.push(chars.next().expect("peeked character"));
                continue;
            }
        }
        out.push(ch);
    }
    out
}

/// Drop the charset introducer MySQL prefixes to every string literal in a
/// stored `CHECK_CLAUSE` (`_utf8mb4'DEV-%'` -> `'DEV-%'`).
///
/// The introducer names the charset the server resolved the literal to, not
/// anything the schema declared, and it stops the expression parser from seeing
/// a string at all — an `IN` list of literals would otherwise never round-trip
/// as an expression.
fn strip_mysql_charset_introducers(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_literal = false;

    while i < chars.len() {
        let ch = chars[i];

        if in_literal {
            out.push(ch);
            if ch == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if ch == '\'' {
                in_literal = false;
            }
            i += 1;
            continue;
        }

        if ch == '_' {
            let mut end = i + 1;
            while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            if end > i + 1 && chars.get(end) == Some(&'\'') {
                i = end;
                continue;
            }
        }

        if ch == '\'' {
            in_literal = true;
        }
        out.push(ch);
        i += 1;
    }

    out
}

/// Strip MySQL backtick-quoted identifiers, keeping the bare identifier name.
///
/// MySQL wraps column and table names in backticks when storing expressions in
/// `information_schema` (e.g. `` `status` `` → `status`). Backtick-quoted names
/// are passed through verbatim without any further escaping because Nautilus
/// schema identifiers are unquoted plain names.
fn strip_mysql_backtick_quotes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '`' {
            for inner in chars.by_ref() {
                if inner == '`' {
                    break;
                }
                result.push(inner);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Normalise a SQLite CHECK expression into a form the Nautilus schema parser accepts.
pub(crate) fn normalize_sqlite_check_expr(expr: &str) -> String {
    let s = strip_all_outer_parens(expr.trim());
    let s = collapse_whitespace(&s);
    convert_in_parens_to_brackets(&s)
}

pub(crate) fn normalize_check_expr(s: &str) -> String {
    let s = strip_identifier_quotes(s.trim());
    let s = strip_casts(&s);
    let s = strip_numeric_paren_literals(&s);
    let s = strip_all_outer_parens(s.trim());
    let s = collapse_whitespace(&s);
    let s = convert_any_array_to_in(&s);
    let s = convert_in_parens_to_brackets(&s);
    canonicalize_check_bool_expr(&s).unwrap_or(s)
}

pub(super) fn strip_identifier_quotes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_single = false;
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\'' {
            out.push(ch);
            if in_single && chars.peek() == Some(&'\'') {
                out.push('\'');
                chars.next();
                continue;
            }
            in_single = !in_single;
            continue;
        }

        if !in_single && (ch == '`' || ch == '"') {
            continue;
        }

        out.push(ch);
    }

    out
}

fn canonicalize_check_bool_expr(s: &str) -> Option<String> {
    let mut lexer = Lexer::new(s);
    let mut tokens = Vec::new();

    loop {
        let token = lexer.next_token().ok()?;
        match token.kind {
            TokenKind::Eof => break,
            TokenKind::Newline => {}
            _ => tokens.push(token),
        }
    }

    if tokens.is_empty() {
        return Some(String::new());
    }

    nautilus_schema::bool_expr::parse_bool_expr(&tokens, Span::new(0, 0))
        .ok()
        .map(|expr| expr.to_string())
}

/// Normalise a generation expression for comparison.
///
/// Databases reformat `GENERATED ALWAYS AS (...)` expressions aggressively:
///   - PostgreSQL adds type casts (`::text`, `::integer`) and extra parens
///   - MySQL lower-cases and may rewrite operator spacing
///   - SQLite preserves the original expression mostly as-is
///
/// We canonicalise by: lower-casing, stripping all `::type` casts (PG),
/// collapsing whitespace, and stripping balanced outer parentheses.
pub(crate) fn normalize_generated_expr(s: &str) -> String {
    let s = strip_casts(&s.to_lowercase());
    let s = collapse_whitespace(&s);
    strip_all_outer_parens(s.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_check_in_list_preserves_casing() {
        assert_eq!(
            normalize_mysql_check_expr("status IN ('Draft', 'PUBLISHED')"),
            "status IN ['Draft', 'PUBLISHED']"
        );
    }

    #[test]
    fn mysql_check_strips_backtick_quotes() {
        assert_eq!(
            normalize_mysql_check_expr("(`status` in ('Draft', 'PUBLISHED'))"),
            "status IN ['Draft', 'PUBLISHED']"
        );
    }

    #[test]
    fn mysql_check_strips_backtick_quotes_numeric() {
        assert_eq!(
            normalize_mysql_check_expr("(`quantity` > 0)"),
            "quantity > 0"
        );
    }

    #[test]
    fn mysql_check_unescapes_literals_and_drops_charset_introducers() {
        assert_eq!(
            normalize_mysql_check_expr(r"(`status` in (_utf8mb4\'Draft\',_utf8mb4\'PUBLISHED\'))"),
            "status IN ['Draft','PUBLISHED']"
        );
    }

    #[test]
    fn mysql_check_keeps_an_escaped_quote_inside_a_literal() {
        assert_eq!(
            normalize_mysql_check_expr(r"(`code` <> _latin1\'O\\\'Reilly\')"),
            r"code <> 'O\'Reilly'"
        );
    }

    #[test]
    fn mysql_check_keeps_an_unparseable_predicate_executable() {
        assert_eq!(
            normalize_mysql_check_expr(r"(upper(`code`) like _utf8mb4\'DEV-%\')"),
            "upper(code) like 'DEV-%'"
        );
    }

    #[test]
    fn sqlite_check_in_list_preserves_casing() {
        assert_eq!(
            normalize_sqlite_check_expr(r#""Role" IN ('ADMIN', 'User')"#),
            r#""Role" IN ['ADMIN', 'User']"#
        );
    }

    #[test]
    fn pg_check_simple_integer() {
        assert_eq!(
            normalize_pg_check_expr("CHECK ((quantity > 0))"),
            "quantity > 0"
        );
    }

    #[test]
    fn pg_check_numeric_literal_cast() {
        assert_eq!(
            normalize_pg_check_expr("CHECK ((price > (0)::numeric))"),
            "price > 0"
        );
    }

    #[test]
    fn pg_check_gte_numeric() {
        assert_eq!(
            normalize_pg_check_expr("CHECK ((stock >= (0)::numeric))"),
            "stock >= 0"
        );
    }

    #[test]
    fn pg_check_no_cast() {
        assert_eq!(
            normalize_pg_check_expr("CHECK ((total_amount > 0))"),
            "total_amount > 0"
        );
    }

    #[test]
    fn pg_check_in_list_preserves_casing() {
        assert_eq!(
            normalize_pg_check_expr("CHECK ((status IN ('DRAFT', 'PUBLISHED')))"),
            "status IN ['DRAFT', 'PUBLISHED']"
        );
    }

    #[test]
    fn pg_check_any_array_to_in() {
        assert_eq!(
            normalize_pg_check_expr(
                "CHECK ((status = ANY (ARRAY['DRAFT'::character varying, 'PUBLISHED'::character varying])))"
            ),
            "status IN ['DRAFT', 'PUBLISHED']"
        );
    }

    #[test]
    fn pg_check_any_array_no_cast() {
        assert_eq!(
            normalize_pg_check_expr("CHECK ((status = ANY (ARRAY['DRAFT', 'PUBLISHED'])))"),
            "status IN ['DRAFT', 'PUBLISHED']"
        );
    }

    #[test]
    fn pg_check_compound_with_in() {
        assert_eq!(
            normalize_pg_check_expr("CHECK ((price > 0 AND role IN ('ADMIN', 'USER')))"),
            "price > 0 AND role IN ['ADMIN', 'USER']"
        );
    }

    #[test]
    fn normalize_generated_expr_strips_pg_casts() {
        assert_eq!(
            normalize_generated_expr("(first_name || (' '::text) || last_name)"),
            "first_name || (' ') || last_name"
        );
    }

    #[test]
    fn normalize_generated_expr_strips_outer_parens() {
        assert_eq!(
            normalize_generated_expr("((price * quantity))"),
            "price * quantity"
        );
    }

    #[test]
    fn normalize_check_expr_sql_and_bracket_forms_match() {
        assert_eq!(
            normalize_check_expr("status IN ('Draft', 'PUBLISHED')"),
            normalize_check_expr("status IN ['Draft', 'PUBLISHED']")
        );
    }

    #[test]
    fn normalize_check_expr_preserves_string_literal_casing() {
        assert_ne!(
            normalize_check_expr("status IN ('Draft', 'PUBLISHED')"),
            normalize_check_expr("status IN ('draft', 'PUBLISHED')")
        );
    }

    #[test]
    fn normalize_check_expr_strips_mysql_backticks() {
        assert_eq!(
            normalize_check_expr("(`status` in ('Draft', 'PUBLISHED'))"),
            "status IN ['Draft', 'PUBLISHED']"
        );
    }
}
