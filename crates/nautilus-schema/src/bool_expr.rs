//! Boolean expression parser for `@check` and `@@check` constraint attributes.
//!
//! The parser operates on tokens already produced by the schema [`Lexer`] and
//! builds a small AST ([`BoolExpr`]) via recursive descent with operator
//! precedence climbing.  The AST implements [`Display`] so it can be
//! round-tripped back to SQL-compatible text.

use std::fmt;

use crate::error::{Result, SchemaError};
use crate::span::Span;
use crate::token::{Token, TokenKind};

/// A comparison operator in a boolean expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `=`
    Eq,
    /// `!=` / `<>`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
}

impl fmt::Display for CmpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "<>",
            CmpOp::Lt => "<",
            CmpOp::Gt => ">",
            CmpOp::Le => "<=",
            CmpOp::Ge => ">=",
        })
    }
}

/// An operand (leaf value) in a boolean expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// A field reference (e.g. `age`, `status`).
    Field(String),
    /// A numeric literal (e.g. `18`, `3.14`).
    Number(String),
    /// A string literal (e.g. `"hello"`).
    StringLit(String),
    /// A boolean literal (`true` / `false`).
    Bool(bool),
    /// A bare identifier inside an `IN [...]` list — treated as an enum variant.
    EnumVariant(String),
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Field(name) => write!(f, "{}", name),
            Operand::Number(n) => write!(f, "{}", n),
            Operand::StringLit(s) => write!(f, "'{}'", s),
            Operand::Bool(b) => write!(f, "{}", if *b { "TRUE" } else { "FALSE" }),
            Operand::EnumVariant(v) => write!(f, "'{}'", v),
        }
    }
}

/// A parsed boolean expression node.
#[derive(Debug, Clone, PartialEq)]
pub enum BoolExpr {
    /// A comparison (e.g. `age > 18`).
    Comparison {
        /// Left-hand operand.
        left: Operand,
        /// Comparison operator.
        op: CmpOp,
        /// Right-hand operand.
        right: Operand,
    },
    /// Logical AND (e.g. `a AND b`).
    And(Box<BoolExpr>, Box<BoolExpr>),
    /// Logical OR (e.g. `a OR b`).
    Or(Box<BoolExpr>, Box<BoolExpr>),
    /// Logical NOT (e.g. `NOT a`).
    Not(Box<BoolExpr>),
    /// IN list (e.g. `status IN [ACTIVE, PENDING]`).
    In {
        /// Field being tested.
        field: String,
        /// List of values to test against.
        values: Vec<Operand>,
    },
    /// Parenthesised sub-expression.
    Paren(Box<BoolExpr>),
}

impl fmt::Display for BoolExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoolExpr::Comparison { left, op, right } => write!(f, "{} {} {}", left, op, right),
            BoolExpr::And(left, right) => write!(f, "{} AND {}", left, right),
            BoolExpr::Or(left, right) => write!(f, "{} OR {}", left, right),
            BoolExpr::Not(inner) => write!(f, "NOT {}", inner),
            BoolExpr::In { field, values } => {
                write!(f, "{} IN [", field)?;
                for (i, val) in values.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    // In schema text, enum variants are bare identifiers; to_sql() quotes them.
                    match val {
                        Operand::EnumVariant(v) => write!(f, "{}", v)?,
                        other => write!(f, "{}", other)?,
                    }
                }
                write!(f, "]")
            }
            BoolExpr::Paren(inner) => write!(f, "({})", inner),
        }
    }
}

impl BoolExpr {
    /// Collect all field references in this expression.
    pub fn field_references(&self) -> Vec<&str> {
        let mut refs = Vec::new();
        self.collect_field_refs(&mut refs);
        refs
    }

    fn collect_field_refs<'a>(&'a self, refs: &mut Vec<&'a str>) {
        match self {
            BoolExpr::Comparison { left, right, .. } => {
                if let Operand::Field(name) = left {
                    refs.push(name);
                }
                if let Operand::Field(name) = right {
                    refs.push(name);
                }
            }
            BoolExpr::And(l, r) | BoolExpr::Or(l, r) => {
                l.collect_field_refs(refs);
                r.collect_field_refs(refs);
            }
            BoolExpr::Not(inner) | BoolExpr::Paren(inner) => {
                inner.collect_field_refs(refs);
            }
            BoolExpr::In { field, .. } => {
                refs.push(field);
            }
        }
    }

    /// Collect `(field_name, [variant_names])` pairs from `IN` nodes where values
    /// are enum variants. Used by the validator for enum checking.
    pub fn enum_in_lists(&self) -> Vec<(&str, Vec<&str>)> {
        let mut result = Vec::new();
        self.collect_enum_in_lists(&mut result);
        result
    }

    fn collect_enum_in_lists<'a>(&'a self, result: &mut Vec<(&'a str, Vec<&'a str>)>) {
        match self {
            BoolExpr::In { field, values } => {
                let variants: Vec<&str> = values
                    .iter()
                    .filter_map(|v| match v {
                        Operand::EnumVariant(name) => Some(name.as_str()),
                        _ => None,
                    })
                    .collect();
                if !variants.is_empty() {
                    result.push((field.as_str(), variants));
                }
            }
            BoolExpr::And(l, r) | BoolExpr::Or(l, r) => {
                l.collect_enum_in_lists(result);
                r.collect_enum_in_lists(result);
            }
            BoolExpr::Not(inner) | BoolExpr::Paren(inner) => {
                inner.collect_enum_in_lists(result);
            }
            BoolExpr::Comparison { .. } => {}
        }
    }

    /// Render this expression as valid SQL (for DDL `CHECK (...)` clauses).
    ///
    /// This differs from `Display` in that `IN` lists use SQL `IN (...)` syntax
    /// and enum variants are rendered as quoted string literals.
    pub fn to_sql(&self) -> String {
        self.to_sql_mapped(&|name| name.to_string())
    }

    /// Render this expression as valid SQL, mapping logical field names to their
    /// physical database column names using the provided function.
    pub fn to_sql_mapped<F>(&self, map_field: &F) -> String
    where
        F: Fn(&str) -> String,
    {
        match self {
            BoolExpr::Comparison { left, op, right } => {
                let left_s = match left {
                    Operand::Field(name) => map_field(name),
                    other => other.to_string(),
                };
                let right_s = match right {
                    Operand::Field(name) => map_field(name),
                    other => other.to_string(),
                };
                format!("{} {} {}", left_s, op, right_s)
            }
            BoolExpr::And(left, right) => format!(
                "{} AND {}",
                left.to_sql_mapped(map_field),
                right.to_sql_mapped(map_field)
            ),
            BoolExpr::Or(left, right) => format!(
                "{} OR {}",
                left.to_sql_mapped(map_field),
                right.to_sql_mapped(map_field)
            ),
            BoolExpr::Not(inner) => format!("NOT {}", inner.to_sql_mapped(map_field)),
            BoolExpr::In { field, values } => {
                let vals: Vec<String> = values.iter().map(|v| v.to_string()).collect();
                format!("{} IN ({})", map_field(field), vals.join(", "))
            }
            BoolExpr::Paren(inner) => format!("({})", inner.to_sql_mapped(map_field)),
        }
    }
}

/// Recursive-descent parser that converts a slice of schema tokens into a
/// [`BoolExpr`] tree.
struct BoolExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
    /// Span used for error reporting when there are no more tokens.
    fallback_span: Span,
}

impl<'a> BoolExprParser<'a> {
    fn new(tokens: &'a [Token], fallback_span: Span) -> Self {
        Self {
            tokens,
            pos: 0,
            fallback_span,
        }
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or(self.fallback_span)
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos];
        self.pos += 1;
        tok
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn is_keyword(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(TokenKind::Ident(s)) if s.eq_ignore_ascii_case(kw))
    }

    fn parse_expr(&mut self) -> Result<BoolExpr> {
        self.parse_or()
    }

    /// OR has the lowest precedence.
    fn parse_or(&mut self) -> Result<BoolExpr> {
        let mut left = self.parse_and()?;
        while self.is_keyword("OR") {
            self.advance();
            let right = self.parse_and()?;
            left = BoolExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<BoolExpr> {
        let mut left = self.parse_not()?;
        while self.is_keyword("AND") {
            self.advance();
            let right = self.parse_not()?;
            left = BoolExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<BoolExpr> {
        if self.is_keyword("NOT") {
            self.advance();
            let inner = self.parse_not()?;
            return Ok(BoolExpr::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<BoolExpr> {
        if self.at_end() {
            return Err(SchemaError::Parse(
                "Unexpected end of check expression".to_string(),
                self.span(),
            ));
        }

        if matches!(self.peek(), Some(TokenKind::LParen)) {
            self.advance();
            let inner = self.parse_expr()?;
            match self.peek() {
                Some(TokenKind::RParen) => {
                    self.advance();
                    return Ok(BoolExpr::Paren(Box::new(inner)));
                }
                _ => {
                    return Err(SchemaError::Parse(
                        "Expected ')' after parenthesised expression".to_string(),
                        self.span(),
                    ));
                }
            }
        }

        if matches!(self.peek(), Some(TokenKind::True)) {
            self.advance();
            return Ok(BoolExpr::Comparison {
                left: Operand::Bool(true),
                op: CmpOp::Eq,
                right: Operand::Bool(true),
            });
        }
        if matches!(self.peek(), Some(TokenKind::False)) {
            self.advance();
            return Ok(BoolExpr::Comparison {
                left: Operand::Bool(false),
                op: CmpOp::Eq,
                right: Operand::Bool(true),
            });
        }

        let left = self.parse_operand(false)?;

        if self.is_keyword("IN") {
            let field_name = match &left {
                Operand::Field(name) => name.clone(),
                _ => {
                    return Err(SchemaError::Parse(
                        "Left side of IN must be a field reference".to_string(),
                        self.span(),
                    ));
                }
            };
            self.advance();
            let values = self.parse_in_list()?;
            return Ok(BoolExpr::In {
                field: field_name,
                values,
            });
        }

        let op = self.parse_cmp_op()?;
        let right = self.parse_operand(false)?;

        Ok(BoolExpr::Comparison { left, op, right })
    }

    /// Parse a single operand (field reference, literal, etc.).
    /// When `in_list` is true, bare identifiers are treated as enum variants.
    fn parse_operand(&mut self, in_list: bool) -> Result<Operand> {
        if self.at_end() {
            return Err(SchemaError::Parse(
                "Expected operand in check expression".to_string(),
                self.span(),
            ));
        }

        match self.peek().cloned() {
            Some(TokenKind::Number(n)) => {
                self.advance();
                Ok(Operand::Number(n))
            }
            Some(TokenKind::String(s)) => {
                self.advance();
                Ok(Operand::StringLit(s))
            }
            Some(TokenKind::True) => {
                self.advance();
                Ok(Operand::Bool(true))
            }
            Some(TokenKind::False) => {
                self.advance();
                Ok(Operand::Bool(false))
            }
            Some(TokenKind::Ident(name)) => {
                self.advance();
                if in_list {
                    Ok(Operand::EnumVariant(name))
                } else {
                    Ok(Operand::Field(name))
                }
            }
            // Schema keywords (e.g. `String`, `Int`) are valid field/enum names inside expressions.
            Some(k) if k.is_keyword() => {
                let tok = self.advance();
                let name = tok.kind.to_string();
                if in_list {
                    Ok(Operand::EnumVariant(name))
                } else {
                    Ok(Operand::Field(name))
                }
            }
            Some(other) => Err(SchemaError::Parse(
                format!("Unexpected token '{}' in check expression", other),
                self.span(),
            )),
            None => Err(SchemaError::Parse(
                "Unexpected end of check expression".to_string(),
                self.span(),
            )),
        }
    }

    fn parse_cmp_op(&mut self) -> Result<CmpOp> {
        if self.at_end() {
            return Err(SchemaError::Parse(
                "Expected comparison operator".to_string(),
                self.span(),
            ));
        }

        match self.peek() {
            Some(TokenKind::Equal) => {
                self.advance();
                Ok(CmpOp::Eq)
            }
            Some(TokenKind::BangEqual) => {
                self.advance();
                Ok(CmpOp::Ne)
            }
            Some(TokenKind::LAngle) => {
                self.advance();
                // `<>` is accepted as an alternative to `!=`.
                if matches!(self.peek(), Some(TokenKind::RAngle)) {
                    self.advance();
                    Ok(CmpOp::Ne)
                } else {
                    Ok(CmpOp::Lt)
                }
            }
            Some(TokenKind::RAngle) => {
                self.advance();
                Ok(CmpOp::Gt)
            }
            Some(TokenKind::LessEqual) => {
                self.advance();
                Ok(CmpOp::Le)
            }
            Some(TokenKind::GreaterEqual) => {
                self.advance();
                Ok(CmpOp::Ge)
            }
            Some(other) => Err(SchemaError::Parse(
                format!(
                    "Expected comparison operator (=, !=, <, >, <=, >=), got '{}'",
                    other
                ),
                self.span(),
            )),
            None => Err(SchemaError::Parse(
                "Expected comparison operator".to_string(),
                self.span(),
            )),
        }
    }

    fn parse_in_list(&mut self) -> Result<Vec<Operand>> {
        match self.peek() {
            Some(TokenKind::LBracket) => {
                self.advance();
            }
            _ => {
                return Err(SchemaError::Parse(
                    "Expected '[' after IN".to_string(),
                    self.span(),
                ));
            }
        }

        let mut values = Vec::new();

        if !matches!(self.peek(), Some(TokenKind::RBracket)) {
            values.push(self.parse_operand(true)?);
            while matches!(self.peek(), Some(TokenKind::Comma)) {
                self.advance();
                values.push(self.parse_operand(true)?);
            }
        }

        match self.peek() {
            Some(TokenKind::RBracket) => {
                self.advance();
                Ok(values)
            }
            _ => Err(SchemaError::Parse(
                "Expected ']' to close IN list".to_string(),
                self.span(),
            )),
        }
    }
}

/// Parse a slice of schema tokens into a validated [`BoolExpr`] tree.
///
/// The token slice should contain **only** the expression tokens (i.e. without
/// the surrounding `@check(` ... `)` scaffolding).
///
/// `fallback_span` is used for error reporting when the slice is empty.
pub fn parse_bool_expr(tokens: &[Token], fallback_span: Span) -> Result<BoolExpr> {
    if tokens.is_empty() {
        return Err(SchemaError::Parse(
            "@check expression is empty".to_string(),
            fallback_span,
        ));
    }

    let mut parser = BoolExprParser::new(tokens, fallback_span);
    let expr = parser.parse_expr()?;

    if !parser.at_end() {
        return Err(SchemaError::Parse(
            format!(
                "Unexpected token '{}' after check expression",
                parser.tokens[parser.pos].kind
            ),
            parser.span(),
        ));
    }

    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn tokenize(src: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(src);
        let mut tokens = Vec::new();
        loop {
            let tok = lexer.next_token().expect("lex error");
            match tok.kind {
                TokenKind::Eof => break,
                TokenKind::Newline => continue,
                _ => tokens.push(tok),
            }
        }
        tokens
    }

    fn parse(src: &str) -> BoolExpr {
        let tokens = tokenize(src);
        parse_bool_expr(&tokens, Span::new(0, 0)).expect("parse error")
    }

    fn parse_err(src: &str) -> String {
        let tokens = tokenize(src);
        match parse_bool_expr(&tokens, Span::new(0, 0)) {
            Err(e) => format!("{}", e),
            Ok(expr) => panic!("Expected error, got: {:?}", expr),
        }
    }

    #[test]
    fn simple_comparison() {
        let expr = parse("age > 18");
        assert_eq!(expr.to_string(), "age > 18");
    }

    #[test]
    fn less_equal() {
        let expr = parse("age <= 150");
        assert_eq!(expr.to_string(), "age <= 150");
    }

    #[test]
    fn greater_equal() {
        let expr = parse("score >= 0");
        assert_eq!(expr.to_string(), "score >= 0");
    }

    #[test]
    fn not_equal() {
        let expr = parse("status != 0");
        assert_eq!(expr.to_string(), "status <> 0");
    }

    #[test]
    fn equality() {
        let expr = parse("active = true");
        assert_eq!(expr.to_string(), "active = TRUE");
    }

    #[test]
    fn and_expression() {
        let expr = parse("age > 18 AND age <= 150");
        assert_eq!(expr.to_string(), "age > 18 AND age <= 150");
    }

    #[test]
    fn or_expression() {
        let expr = parse("age < 18 OR age > 65");
        assert_eq!(expr.to_string(), "age < 18 OR age > 65");
    }

    #[test]
    fn not_expression() {
        let expr = parse("NOT age < 0");
        assert_eq!(expr.to_string(), "NOT age < 0");
    }

    #[test]
    fn in_with_enum_variants() {
        let expr = parse("status IN [ACTIVE, PENDING]");
        assert_eq!(expr.to_string(), "status IN [ACTIVE, PENDING]");
    }

    #[test]
    fn in_with_numbers() {
        let expr = parse("priority IN [1, 2, 3]");
        assert_eq!(expr.to_string(), "priority IN [1, 2, 3]");
    }

    #[test]
    fn in_with_strings() {
        let expr = parse("role IN [\"admin\", \"moderator\"]");
        assert_eq!(expr.to_string(), "role IN ['admin', 'moderator']");
    }

    #[test]
    fn complex_and_or() {
        let expr = parse("age > 18 AND status IN [ACTIVE, PENDING]");
        assert_eq!(expr.to_string(), "age > 18 AND status IN [ACTIVE, PENDING]");
    }

    #[test]
    fn parenthesised() {
        let expr = parse("(age > 18 OR admin = true) AND active = true");
        assert_eq!(
            expr.to_string(),
            "(age > 18 OR admin = TRUE) AND active = TRUE"
        );
    }

    #[test]
    fn sql_output() {
        let expr = parse("status IN [ACTIVE, PENDING]");
        assert_eq!(expr.to_sql(), "status IN ('ACTIVE', 'PENDING')");
    }

    #[test]
    fn sql_output_complex() {
        let expr = parse("age > 18 AND status IN [ACTIVE, PENDING]");
        assert_eq!(
            expr.to_sql(),
            "age > 18 AND status IN ('ACTIVE', 'PENDING')"
        );
    }

    #[test]
    fn field_references() {
        let expr = parse("age > 18 AND status IN [ACTIVE]");
        let refs = expr.field_references();
        assert_eq!(refs, vec!["age", "status"]);
    }

    #[test]
    fn enum_in_lists() {
        let expr = parse("status IN [ACTIVE, PENDING] AND role IN [ADMIN]");
        let lists = expr.enum_in_lists();
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0], ("status", vec!["ACTIVE", "PENDING"]));
        assert_eq!(lists[1], ("role", vec!["ADMIN"]));
    }

    #[test]
    fn empty_is_error() {
        let tokens: Vec<Token> = vec![];
        assert!(parse_bool_expr(&tokens, Span::new(0, 0)).is_err());
    }

    #[test]
    fn missing_operator_is_error() {
        let err = parse_err("age 18");
        assert!(err.contains("Expected comparison operator"));
    }

    #[test]
    fn unclosed_in_list_is_error() {
        let err = parse_err("status IN [ACTIVE, PENDING");
        assert!(err.contains("Expected ']'"));
    }

    #[test]
    fn missing_in_bracket_is_error() {
        let err = parse_err("status IN ACTIVE");
        assert!(err.contains("Expected '['"));
    }
}
