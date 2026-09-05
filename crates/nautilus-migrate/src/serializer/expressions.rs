//! Expressions carried back from the database: `DEFAULT` clauses, `CHECK`
//! predicates and index predicates.
//!
//! Each arrives as provider SQL naming physical columns, and has to leave as
//! schema source naming logical fields — which means parsing it rather than
//! rewriting the text.

use std::collections::HashMap;

use nautilus_schema::{
    bool_expr::{parse_bool_expr, BoolExpr, Operand},
    sql_expr::{parse_sql_expr, SqlExpr},
    Lexer, Span, Token, TokenKind,
};

use super::escape_schema_string;
use super::types::{can_infer_autoincrement, matching_named_type, parse_inline_enum_variants};

/// Try to produce a `@default(...)` attribute from a raw DEFAULT expression
/// string as returned by the database. Returns `None` when the default is too
/// complex to round-trip safely.
pub(super) fn infer_default_attr(
    raw: &str,
    col_type: &str,
    enums: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let t = raw.trim().to_lowercase();

    if t.contains("nextval") || t.contains("autoincrement") {
        if can_infer_autoincrement(col_type) {
            return Some("@default(autoincrement())".to_string());
        }
        return None;
    }

    if t == "true" || t == "false" {
        return Some(format!("@default({})", t));
    }

    // MySQL stores booleans as `tinyint(1)` and reports their default as `1` or
    // `0`. Rendering that verbatim produces `Boolean @default(1)`, which the
    // schema validator rejects as a type mismatch.
    if col_type.trim().eq_ignore_ascii_case("boolean") {
        match t.as_str() {
            "1" => return Some("@default(true)".to_string()),
            "0" => return Some("@default(false)".to_string()),
            _ => {}
        }
    }

    if t.parse::<f64>().is_ok() {
        return Some(format!("@default({})", t));
    }

    if t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2 {
        let inner = &raw.trim()[1..raw.trim().len() - 1];
        let base_type = col_type.trim().to_lowercase();
        let base_type = base_type.strip_suffix("[]").unwrap_or(&base_type);
        // An enum default is a bare variant, not a string literal — including
        // for a MySQL column enum, which names no type to match on.
        let is_enum = matching_named_type(base_type, enums).is_some()
            || parse_inline_enum_variants(col_type)
                .is_some_and(|variants| variants.iter().any(|variant| variant == inner));
        if is_enum {
            return Some(format!("@default({})", inner));
        }
        return Some(format!("@default(\"{}\")", inner));
    }

    if t == "now()" || t == "current_timestamp" || t.starts_with("current_timestamp") {
        return Some("@default(now())".to_string());
    }

    if t.contains("uuidv7") {
        return Some("@default(uuidv7())".to_string());
    }

    if t.contains("uuid") || t.contains("newid") {
        return Some("@default(uuid())".to_string());
    }

    None
}

pub(super) fn remap_sql_expr_identifiers(
    expr: &str,
    field_map: &HashMap<String, String>,
) -> String {
    parse_schema_sql_expr(expr)
        .map(|parsed| render_sql_expr_with_field_map(&parsed, field_map))
        .unwrap_or_else(|| expr.to_string())
}

/// Render a live CHECK expression (or index predicate) as schema text.
///
/// Predicates the expression language covers are re-rendered with logical field
/// names. Anything else — `IS NULL`, `LIKE`, function calls, typed literals —
/// is emitted as a raw quoted predicate rather than as bare database text: bare
/// text does not reparse, so the pulled schema would be unusable, while the raw
/// form round-trips and still pushes back verbatim.
pub(super) fn remap_bool_expr_identifiers(
    expr: &str,
    field_map: &HashMap<String, String>,
) -> String {
    parse_schema_bool_expr(expr)
        .map(|parsed| render_bool_expr_with_field_map(&parsed, field_map))
        .unwrap_or_else(|| format!("\"{}\"", escape_schema_string(&restore_sql_in_lists(expr))))
}

/// Undo the inspector's `IN (...)` -> `IN [...]` rewrite.
///
/// A raw predicate is pushed back to the database verbatim, so it has to be
/// SQL. The inspector normalises live expressions into the schema dialect
/// before the serializer sees them, and the bracket form is the one piece of
/// that dialect no database accepts.
fn restore_sql_in_lists(expr: &str) -> String {
    let lower = expr.to_lowercase();
    let marker = " in [";
    if !lower.contains(marker) {
        return expr.to_string();
    }

    let mut result = String::with_capacity(expr.len());
    let mut pos = 0usize;

    while let Some(rel) = lower[pos..].find(marker) {
        let abs = pos + rel;
        result.push_str(&expr[pos..abs]);
        result.push_str(" IN (");

        let after_open = abs + marker.len();
        let rest = &expr[after_open..];

        let mut depth = 1i32;
        let mut close = rest.len();
        for (i, c) in rest.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
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
        result.push(')');
        pos = (after_open + close + 1).min(expr.len());
    }

    result.push_str(&expr[pos..]);
    result
}

fn parse_schema_sql_expr(expr: &str) -> Option<SqlExpr> {
    let tokens = lex_expression_tokens(expr).ok()?;
    parse_sql_expr(&tokens, Span::new(0, expr.len())).ok()
}

fn parse_schema_bool_expr(expr: &str) -> Option<BoolExpr> {
    let tokens = lex_expression_tokens(expr).ok()?;
    parse_bool_expr(&tokens, Span::new(0, expr.len())).ok()
}

fn lex_expression_tokens(expr: &str) -> nautilus_schema::Result<Vec<Token>> {
    let mut lexer = Lexer::new(expr);
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        if matches!(token.kind, TokenKind::Eof) {
            break;
        }
        tokens.push(token);
    }
    Ok(tokens)
}

fn render_sql_expr_with_field_map(expr: &SqlExpr, field_map: &HashMap<String, String>) -> String {
    match expr {
        SqlExpr::Ident(name) => field_map.get(name).cloned().unwrap_or_else(|| name.clone()),
        SqlExpr::Number(n) => n.clone(),
        SqlExpr::StringLit(s) => format!("\"{}\"", s),
        SqlExpr::Bool(b) => b.to_string(),
        SqlExpr::BinaryOp { left, op, right } => format!(
            "{} {} {}",
            render_sql_expr_with_field_map(left, field_map),
            op,
            render_sql_expr_with_field_map(right, field_map)
        ),
        SqlExpr::UnaryOp { op, operand } => {
            format!(
                "{}{}",
                op,
                render_sql_expr_with_field_map(operand, field_map)
            )
        }
        SqlExpr::FnCall { name, args } => format!(
            "{}({})",
            name,
            args.iter()
                .map(|arg| render_sql_expr_with_field_map(arg, field_map))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SqlExpr::Paren(inner) => format!("({})", render_sql_expr_with_field_map(inner, field_map)),
    }
}

fn render_bool_expr_with_field_map(expr: &BoolExpr, field_map: &HashMap<String, String>) -> String {
    match expr {
        BoolExpr::Comparison { left, op, right } => format!(
            "{} {} {}",
            render_bool_operand_with_field_map(left, field_map, false),
            op,
            render_bool_operand_with_field_map(right, field_map, false)
        ),
        BoolExpr::And(left, right) => format!(
            "{} AND {}",
            render_bool_expr_with_field_map(left, field_map),
            render_bool_expr_with_field_map(right, field_map)
        ),
        BoolExpr::Or(left, right) => format!(
            "{} OR {}",
            render_bool_expr_with_field_map(left, field_map),
            render_bool_expr_with_field_map(right, field_map)
        ),
        BoolExpr::Not(inner) => {
            format!("NOT {}", render_bool_expr_with_field_map(inner, field_map))
        }
        BoolExpr::In { field, values } => format!(
            "{} IN [{}]",
            field_map
                .get(field)
                .cloned()
                .unwrap_or_else(|| field.clone()),
            values
                .iter()
                .map(|value| render_bool_operand_with_field_map(value, field_map, true))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        BoolExpr::Paren(inner) => {
            format!("({})", render_bool_expr_with_field_map(inner, field_map))
        }
        BoolExpr::Raw(_) => expr.to_string(),
    }
}

fn render_bool_operand_with_field_map(
    operand: &Operand,
    field_map: &HashMap<String, String>,
    enum_variant_in_list: bool,
) -> String {
    match operand {
        Operand::Field(name) => field_map.get(name).cloned().unwrap_or_else(|| name.clone()),
        Operand::EnumVariant(variant) if enum_variant_in_list => variant.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_boolean() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("true", "boolean", &no_enums),
            Some("@default(true)".into())
        );
    }

    #[test]
    fn default_boolean_from_mysql_tinyint() {
        let no_enums = HashMap::new();
        // MySQL reports a boolean default as 1/0; rendering it verbatim would
        // produce `Boolean @default(1)`, which fails schema validation.
        assert_eq!(
            infer_default_attr("1", "boolean", &no_enums),
            Some("@default(true)".into())
        );
        assert_eq!(
            infer_default_attr("0", "boolean", &no_enums),
            Some("@default(false)".into())
        );
    }

    #[test]
    fn numeric_defaults_on_non_boolean_columns_are_untouched() {
        let no_enums = HashMap::new();
        assert_eq!(
            infer_default_attr("1", "integer", &no_enums),
            Some("@default(1)".into())
        );
        assert_eq!(
            infer_default_attr("0", "integer", &no_enums),
            Some("@default(0)".into())
        );
    }

    #[test]
    fn default_number() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("42", "integer", &no_enums),
            Some("@default(42)".into())
        );
    }

    #[test]
    fn default_string() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("'hello'", "text", &no_enums),
            Some("@default(\"hello\")".into())
        );
    }

    #[test]
    fn default_now() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("current_timestamp", "timestamp", &no_enums),
            Some("@default(now())".into())
        );
    }

    #[test]
    fn default_uuid() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("gen_random_uuid()", "uuid", &no_enums),
            Some("@default(uuid())".into())
        );
    }

    #[test]
    fn default_uuidv7() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("uuidv7()", "uuid", &no_enums),
            Some("@default(uuidv7())".into())
        );
    }

    #[test]
    fn default_nextval_skipped() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("nextval('seq')", "integer", &no_enums),
            Some("@default(autoincrement())".into())
        );
    }

    #[test]
    fn default_enum_literal() {
        let mut enums: HashMap<String, Vec<String>> = HashMap::new();
        enums.insert(
            "status".to_string(),
            vec!["DRAFT".to_string(), "PUBLISHED".to_string()],
        );
        assert_eq!(
            infer_default_attr("'DRAFT'", "status", &enums),
            Some("@default(DRAFT)".into())
        );
    }

    #[test]
    fn default_string_not_confused_with_enum() {
        let no_enums: HashMap<String, Vec<String>> = HashMap::new();
        assert_eq!(
            infer_default_attr("'hello'", "text", &no_enums),
            Some("@default(\"hello\")".into())
        );
    }
}
