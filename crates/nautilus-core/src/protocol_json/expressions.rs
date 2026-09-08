//! The leaves both the argument object and the filter object end at: a column
//! reference and a value operand.
//!
//! A [`Expr::Param`] carries a [`Value`](crate::Value), which the wire format
//! writes as plain JSON; a column carries the `Model__column` alias the query
//! builder uses to keep joined columns apart, which the wire format writes as
//! the bare field name.

use serde_json::Value as JsonValue;

use crate::{Error, Expr, Result};

/// Turn a `Model__column` alias into the field name used on the wire.
pub(super) fn strip_column_qualifier(name: &str) -> String {
    name.split_once("__")
        .map(|(_, column)| column.to_string())
        .unwrap_or_else(|| name.to_string())
}

pub(super) fn expr_value_to_json(expr: &Expr) -> Result<JsonValue> {
    match expr {
        Expr::Param(value) => Ok(value.to_json_plain()),
        other => Err(Error::InvalidQuery(format!(
            "query cannot be serialized to engine JSON: unsupported value expression {:?}",
            other
        ))),
    }
}

pub(super) fn list_expr_to_json_array(expr: &Expr) -> Result<JsonValue> {
    let Expr::List(items) = expr else {
        return Err(Error::InvalidQuery(format!(
            "query cannot be serialized to engine JSON: unsupported list operand {:?}",
            expr
        )));
    };

    let mut values = Vec::with_capacity(items.len());
    for item in items {
        values.push(expr_value_to_json(item)?);
    }

    Ok(JsonValue::Array(values))
}

/// Map a LIKE pattern back onto the substring operator that produced it.
///
/// The engine writes `contains` / `startsWith` / `endsWith` as a LIKE pattern
/// with the literal term escaped, so recovering the operator means removing the
/// wildcards the operator implies and, for [`BinaryOp::LikeEscape`], undoing
/// that escaping. A pattern without leading or trailing `%` stays a `like`.
///
/// [`BinaryOp::LikeEscape`]: crate::BinaryOp::LikeEscape
pub(super) fn like_operator_and_value(
    expr: &Expr,
    escaped: bool,
) -> Result<(Option<&'static str>, JsonValue)> {
    let value = match expr {
        Expr::Param(value) => value.to_json_plain(),
        other => {
            return Err(Error::InvalidQuery(format!(
                "query cannot be serialized to engine JSON: unsupported LIKE operand {:?}",
                other
            )));
        }
    };

    let Some(pattern) = value.as_str() else {
        return Ok((Some("like"), value));
    };

    let term = |raw: &str| {
        JsonValue::String(if escaped {
            unescape_like_term(raw)
        } else {
            raw.to_string()
        })
    };

    if pattern.starts_with('%') && pattern.ends_with('%') && pattern.len() >= 2 {
        return Ok((Some("contains"), term(&pattern[1..pattern.len() - 1])));
    }

    if let Some(stripped) = pattern.strip_prefix('%') {
        return Ok((Some("endsWith"), term(stripped)));
    }

    if let Some(stripped) = pattern.strip_suffix('%') {
        return Ok((Some("startsWith"), term(stripped)));
    }

    Ok((Some("like"), JsonValue::String(pattern.to_string())))
}

/// Undo the `\` escaping the engine applies to a literal substring search term.
fn unescape_like_term(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
                continue;
            }
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Value;

    fn like(pattern: &str, escaped: bool) -> (Option<&'static str>, JsonValue) {
        like_operator_and_value(&Expr::param(Value::String(pattern.to_string())), escaped)
            .expect("a string parameter is a valid LIKE operand")
    }

    #[test]
    fn wildcards_choose_the_substring_operator() {
        assert_eq!(like("%rust%", false), (Some("contains"), json!("rust")));
        assert_eq!(like("rust%", false), (Some("startsWith"), json!("rust")));
        assert_eq!(like("%rust", false), (Some("endsWith"), json!("rust")));
        assert_eq!(like("rust", false), (Some("like"), json!("rust")));
        assert_eq!(like("%%", false), (Some("contains"), json!("")));
        assert_eq!(like("%", false), (Some("endsWith"), json!("")));
    }

    #[test]
    fn an_escaped_pattern_gives_back_the_literal_term() {
        assert_eq!(
            like(r"%50\%\_off%", true),
            (Some("contains"), json!("50%_off"))
        );
        assert_eq!(
            like(r"c:\\dir%", true),
            (Some("startsWith"), json!(r"c:\dir"))
        );
        assert_eq!(like(r"%50\%%", false), (Some("contains"), json!(r"50\%")));
    }

    #[test]
    fn a_column_alias_becomes_the_bare_field_name() {
        assert_eq!(strip_column_qualifier("Entry__slug"), "slug");
        assert_eq!(strip_column_qualifier("slug"), "slug");
    }

    #[test]
    fn a_non_parameter_operand_is_rejected() {
        let column = Expr::column("Entry__slug");

        assert!(matches!(
            expr_value_to_json(&column).expect_err("a column is not a value"),
            Error::InvalidQuery(_)
        ));
        assert!(matches!(
            list_expr_to_json_array(&column).expect_err("a column is not a list"),
            Error::InvalidQuery(_)
        ));
        assert!(matches!(
            like_operator_and_value(&column, false).expect_err("a column is not a pattern"),
            Error::InvalidQuery(_)
        ));
    }
}
