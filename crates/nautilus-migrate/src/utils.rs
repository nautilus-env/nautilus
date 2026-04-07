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
/// is correctly balanced by that closing paren (e.g. `((a)(b))` — 2 layers —
/// would not be stripped to `(a)(b)` but `(a)` would be stripped to `a`).
pub(crate) fn strip_outer_parens(s: &str) -> String {
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        let depth: i32 = inner.chars().fold(0i32, |d, c| match c {
            '(' => d + 1,
            ')' => d - 1,
            _ => d,
        });
        if depth == 0 {
            return inner.to_string();
        }
    }
    s.to_string()
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
    fn no_strip_without_parens() {
        assert_eq!(strip_outer_parens("hello"), "hello");
    }

    #[test]
    fn no_strip_empty() {
        assert_eq!(strip_outer_parens(""), "");
    }
}
