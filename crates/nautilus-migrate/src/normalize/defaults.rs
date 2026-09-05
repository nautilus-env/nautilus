//! `DEFAULT` expressions: the form introspection stores, and the form the diff
//! compares.

use super::sql_text::{collapse_whitespace, strip_casts};
use crate::utils::strip_outer_parens;

/// Strip Postgres-generated type casts from a default expression.
pub(crate) fn normalize_pg_default(default: &str) -> String {
    let s = strip_casts(default.trim());
    collapse_whitespace(&s)
}

/// Normalise a SQLite default expression for comparison.
pub(crate) fn normalize_sqlite_default(raw: &str) -> String {
    let s = raw.trim();
    strip_outer_parens(s)
}

/// Normalise a default-value expression for comparison so that cosmetic
/// differences don't cause false-positive [`Change::DefaultChanged`].
///
/// Lowercases, trims whitespace, and strips a single balanced layer of outer
/// parentheses so that enum literal casing (`'DRAFT'` vs `'draft'`) and
/// SQLite paren differences don't produce false positives.
pub(crate) fn normalize_default(s: &str) -> String {
    let lowered = s.trim().to_lowercase();
    strip_outer_parens(&lowered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_default_strips_cast() {
        assert_eq!(normalize_pg_default("'hello'::text"), "'hello'");
    }

    #[test]
    fn pg_default_no_cast() {
        assert_eq!(normalize_pg_default("42"), "42");
    }

    #[test]
    fn pg_default_preserves_function() {
        assert_eq!(normalize_pg_default("now()"), "now()");
    }

    #[test]
    fn pg_default_nextval_keeps_closing_paren() {
        assert_eq!(
            normalize_pg_default("nextval('tags_id_seq'::regclass)"),
            "nextval('tags_id_seq')"
        );
        assert_eq!(
            normalize_pg_default("nextval('_nautilus_migrations_id_seq'::regclass)"),
            "nextval('_nautilus_migrations_id_seq')"
        );
    }

    #[test]
    fn pg_default_character_varying_cast() {
        assert_eq!(
            normalize_pg_default("'DRAFT'::character varying"),
            "'DRAFT'"
        );
    }

    #[test]
    fn pg_default_quoted_identifier_cast() {
        assert_eq!(normalize_pg_default("'DRAFT'::\"poststatus\""), "'DRAFT'");
        assert_eq!(normalize_pg_default("'user'::\"role\""), "'user'");
        assert_eq!(normalize_pg_default("'USER'::\"Role\""), "'USER'");
    }

    #[test]
    fn sqlite_default_strips_parens() {
        assert_eq!(normalize_sqlite_default("(42)"), "42");
    }

    #[test]
    fn sqlite_default_preserves_nested_parens() {
        assert_eq!(normalize_sqlite_default("((1+2))"), "(1+2)");
    }

    #[test]
    fn sqlite_default_no_parens() {
        assert_eq!(normalize_sqlite_default("'hello'"), "'hello'");
    }

    #[test]
    fn sqlite_default_preserves_string_casing() {
        assert_eq!(normalize_sqlite_default("'Hello World'"), "'Hello World'");
    }
}
