//! Normalisation rules shared by the diff passes.
//!
//! A database rewrites what it is given — it quotes identifiers, adds casts and
//! parentheses, and spells a type its own way — so comparing the inspected text
//! against the schema text literally reports changes for columns that never
//! changed. Each rule here reduces both sides to the form in which a difference
//! is a real difference.

use nautilus_schema::{Lexer, Span, TokenKind};

use crate::ddl::DatabaseProvider;

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
pub(super) fn column_types_match(provider: DatabaseProvider, live: &str, target: &str) -> bool {
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

/// Normalise a default-value expression for comparison so that cosmetic
/// differences don't cause false-positive [`Change::DefaultChanged`].
///
/// Lowercases, trims whitespace, and strips a single balanced layer of outer
/// parentheses so that enum literal casing (`'DRAFT'` vs `'draft'`) and
/// SQLite paren differences don't produce false positives.
pub(crate) fn normalize_default(s: &str) -> String {
    let lowered = s.trim().to_lowercase();
    crate::utils::strip_outer_parens(&lowered)
}

pub(super) fn normalize_check_expr(s: &str) -> String {
    let s = strip_identifier_quotes(s.trim());
    let s = strip_check_casts(&s);
    let s = strip_numeric_check_parens(&s);
    let mut s = s.trim().to_string();
    loop {
        let stripped = crate::utils::strip_outer_parens(&s);
        if stripped == s {
            break;
        }
        s = stripped;
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = convert_check_any_array_to_in(&s);
    let s = convert_check_in_parens_to_brackets(&s);
    canonicalize_check_bool_expr(&s).unwrap_or(s)
}

fn strip_identifier_quotes(s: &str) -> String {
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

fn strip_check_casts(s: &str) -> String {
    let mut result = s.to_string();
    while let Some(idx) = result.rfind("::") {
        let after = &result[idx + 2..];
        let type_end = if let Some(rest) = after.strip_prefix('"') {
            rest.find('"').map(|i| i + 2).unwrap_or(after.len())
        } else {
            after
                .find(|c: char| !c.is_alphanumeric() && c != '_' && c != ' ')
                .unwrap_or(after.len())
        };
        result = format!("{}{}", &result[..idx], &result[idx + 2 + type_end..]);
    }
    result
}

fn strip_numeric_check_parens(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '(' {
            let mut inner = String::new();
            let mut depth = 1i32;
            for c in chars.by_ref() {
                match c {
                    '(' => {
                        depth += 1;
                        inner.push(c);
                    }
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        inner.push(c);
                    }
                    _ => inner.push(c),
                }
            }
            let trimmed = inner.trim();
            let is_numeric = !trimmed.is_empty()
                && trimmed
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == '-');
            if is_numeric {
                result.push_str(trimmed);
            } else {
                result.push('(');
                result.push_str(&strip_numeric_check_parens(&inner));
                result.push(')');
            }
        } else {
            result.push(ch);
        }
    }

    result
}

fn convert_check_any_array_to_in(s: &str) -> String {
    let lower = s.to_lowercase();
    let marker = "= any (array[";

    let Some(eq_pos) = lower.find(marker) else {
        return s.to_string();
    };

    let field = s[..eq_pos].trim();
    let bracket_start = eq_pos + marker.len();
    let rest = &s[bracket_start..];

    let mut depth = 1i32;
    let mut bracket_end = None;
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    bracket_end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    let Some(bclose) = bracket_end else {
        return s.to_string();
    };

    let items = &rest[..bclose];
    let after_array = rest[bclose + 1..].trim_start();
    let after_paren = after_array.strip_prefix(')').unwrap_or(after_array);

    if after_paren.is_empty() {
        format!("{} IN [{}]", field, items)
    } else {
        format!("{} IN [{}] {}", field, items, after_paren.trim_start())
    }
}

fn convert_check_in_parens_to_brackets(s: &str) -> String {
    let lower = s.to_lowercase();
    let marker = " in (";

    if !lower.contains(marker) {
        return s.to_string();
    }

    let mut result = String::with_capacity(s.len());
    let mut pos = 0usize;

    while let Some(rel) = lower[pos..].find(marker) {
        let abs = pos + rel;
        result.push_str(&s[pos..abs]);
        result.push_str(" IN [");

        let after_open = abs + marker.len();
        let rest = &s[after_open..];

        let mut depth = 1i32;
        let mut close = rest.len();
        for (i, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        result.push_str(&rest[..close]);
        result.push(']');
        pos = after_open + close + 1;
    }

    result.push_str(&s[pos..]);
    result
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
pub(super) fn normalize_generated_expr(s: &str) -> String {
    let mut s = s.to_lowercase();
    // Strip Postgres-style type casts: `::text`, `::integer`, `::character varying`, `"enum"`
    while let Some(idx) = s.find("::") {
        let after = &s[idx + 2..];
        let type_end = if let Some(rest) = after.strip_prefix('"') {
            rest.find('"').map(|i| i + 2).unwrap_or(after.len())
        } else {
            after
                .find(|c: char| !c.is_alphanumeric() && c != '_' && c != ' ')
                .unwrap_or(after.len())
        };
        s = format!("{}{}", &s[..idx], &s[idx + 2 + type_end..]);
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = s.trim().to_string();
    loop {
        let stripped = crate::utils::strip_outer_parens(&s);
        if stripped == s {
            break;
        }
        s = stripped;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

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
