//! Lightweight SQL expression parser for `@computed` and future attribute
//! expressions.
//!
//! The parser operates on tokens already produced by the schema [`Lexer`] and
//! builds a small AST ([`SqlExpr`]) via recursive descent with operator
//! precedence climbing.  The AST implements [`Display`] so it can be
//! round-tripped back to SQL text.

use std::fmt;

use crate::error::{Result, SchemaError};
use crate::span::Span;
use crate::token::{Token, TokenKind};
/// A binary operator in a SQL expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Mod,
    /// `||` (string concatenation)
    Concat,
    /// `<`
    Lt,
    /// `>`
    Gt,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Concat => "||",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
        })
    }
}

/// A unary operator in a SQL expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `-` (negation)
    Neg,
    /// `+` (no-op, explicit positive)
    Pos,
}

impl fmt::Display for UnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UnaryOp::Neg => "-",
            UnaryOp::Pos => "+",
        })
    }
}

/// A parsed SQL expression node.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlExpr {
    /// Column reference or SQL keyword (e.g. `price`, `COALESCE`).
    Ident(String),
    /// Numeric literal (e.g. `42`, `3.14`).
    Number(String),
    /// String literal (e.g. `"hello"`).
    StringLit(String),
    /// Boolean literal (`true` / `false`).
    Bool(bool),
    /// Binary operation (e.g. `price * quantity`).
    BinaryOp {
        /// Left-hand side.
        left: Box<SqlExpr>,
        /// Operator.
        op: BinOp,
        /// Right-hand side.
        right: Box<SqlExpr>,
    },
    /// Unary operation (e.g. `-amount`).
    UnaryOp {
        /// Operator.
        op: UnaryOp,
        /// Operand.
        operand: Box<SqlExpr>,
    },
    /// Function call (e.g. `COALESCE(a, b)`).
    FnCall {
        /// Function name.
        name: String,
        /// Argument list.
        args: Vec<SqlExpr>,
    },
    /// Parenthesised sub-expression.
    Paren(Box<SqlExpr>),
}
impl fmt::Display for SqlExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SqlExpr::Ident(name) => write!(f, "{}", name),
            SqlExpr::Number(n) => write!(f, "{}", n),
            SqlExpr::StringLit(s) => write!(f, "\"{}\"", s),
            SqlExpr::Bool(b) => write!(f, "{}", b),
            SqlExpr::BinaryOp { left, op, right } => {
                write!(f, "{} {} {}", left, op, right)
            }
            SqlExpr::UnaryOp { op, operand } => write!(f, "{}{}", op, operand),
            SqlExpr::FnCall { name, args } => {
                write!(f, "{}(", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            SqlExpr::Paren(inner) => write!(f, "({})", inner),
        }
    }
}

impl SqlExpr {
    /// Render this expression, mapping logical field identifiers to their
    /// physical database column names using the provided function.
    pub fn to_sql_mapped<F>(&self, map_field: &F) -> String
    where
        F: Fn(&str) -> String,
    {
        match self {
            SqlExpr::Ident(name) => map_field(name),
            SqlExpr::Number(n) => n.clone(),
            SqlExpr::StringLit(s) => format!("\"{}\"", s),
            SqlExpr::Bool(b) => b.to_string(),
            SqlExpr::BinaryOp { left, op, right } => format!(
                "{} {} {}",
                left.to_sql_mapped(map_field),
                op,
                right.to_sql_mapped(map_field)
            ),
            SqlExpr::UnaryOp { op, operand } => {
                format!("{}{}", op, operand.to_sql_mapped(map_field))
            }
            SqlExpr::FnCall { name, args } => {
                let args_s: Vec<String> = args.iter().map(|a| a.to_sql_mapped(map_field)).collect();
                format!("{}({})", name, args_s.join(", "))
            }
            SqlExpr::Paren(inner) => format!("({})", inner.to_sql_mapped(map_field)),
        }
    }
}
/// Recursive-descent parser that converts a slice of schema tokens into a
/// [`SqlExpr`] tree.
struct SqlExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
    /// Span used for error reporting when there are no more tokens.
    fallback_span: Span,
}

impl<'a> SqlExprParser<'a> {
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

    /// Operator precedence (higher = tighter binding).
    fn precedence(op: &BinOp) -> u8 {
        match op {
            BinOp::Concat => 1,
            BinOp::Lt | BinOp::Gt => 2,
            BinOp::Add | BinOp::Sub => 3,
            BinOp::Mul | BinOp::Div | BinOp::Mod => 4,
        }
    }

    fn token_to_binop(kind: &TokenKind) -> Option<BinOp> {
        match kind {
            TokenKind::Plus => Some(BinOp::Add),
            TokenKind::Minus => Some(BinOp::Sub),
            TokenKind::Star => Some(BinOp::Mul),
            TokenKind::Slash => Some(BinOp::Div),
            TokenKind::Percent => Some(BinOp::Mod),
            TokenKind::DoublePipe => Some(BinOp::Concat),
            TokenKind::LAngle => Some(BinOp::Lt),
            TokenKind::RAngle => Some(BinOp::Gt),
            _ => None,
        }
    }

    fn parse_expr(&mut self) -> Result<SqlExpr> {
        self.parse_binary(0)
    }

    fn parse_binary(&mut self, min_prec: u8) -> Result<SqlExpr> {
        let mut left = self.parse_unary()?;

        while let Some(kind) = self.peek().cloned() {
            let Some(op) = Self::token_to_binop(&kind) else {
                break;
            };
            let prec = Self::precedence(&op);
            if prec < min_prec {
                break;
            }
            self.advance();
            let right = self.parse_binary(prec + 1)?;
            left = SqlExpr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<SqlExpr> {
        match self.peek() {
            Some(TokenKind::Minus) => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(SqlExpr::UnaryOp {
                    op: UnaryOp::Neg,
                    operand: Box::new(operand),
                })
            }
            Some(TokenKind::Plus) => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(SqlExpr::UnaryOp {
                    op: UnaryOp::Pos,
                    operand: Box::new(operand),
                })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<SqlExpr> {
        if self.at_end() {
            return Err(SchemaError::Parse(
                "Unexpected end of SQL expression".to_string(),
                self.span(),
            ));
        }

        match self.peek().cloned() {
            Some(TokenKind::Number(n)) => {
                self.advance();
                Ok(SqlExpr::Number(n))
            }
            Some(TokenKind::String(s)) => {
                self.advance();
                Ok(SqlExpr::StringLit(s))
            }
            Some(TokenKind::True) => {
                self.advance();
                Ok(SqlExpr::Bool(true))
            }
            Some(TokenKind::False) => {
                self.advance();
                Ok(SqlExpr::Bool(false))
            }
            Some(TokenKind::Ident(_)) => self.parse_ident_or_call(),
            // Keywords used as identifiers inside SQL expressions
            Some(k) if k.is_keyword() => self.parse_ident_or_call(),
            Some(TokenKind::LParen) => {
                self.advance();
                let inner = self.parse_expr()?;
                match self.peek() {
                    Some(TokenKind::RParen) => {
                        self.advance();
                        Ok(SqlExpr::Paren(Box::new(inner)))
                    }
                    _ => Err(SchemaError::Parse(
                        "Expected ')' after parenthesised expression".to_string(),
                        self.span(),
                    )),
                }
            }
            Some(other) => Err(SchemaError::Parse(
                format!("Unexpected token '{}' in SQL expression", other),
                self.span(),
            )),
            None => Err(SchemaError::Parse(
                "Unexpected end of SQL expression".to_string(),
                self.span(),
            )),
        }
    }

    fn parse_ident_or_call(&mut self) -> Result<SqlExpr> {
        let tok = self.advance();
        let name = match &tok.kind {
            TokenKind::Ident(s) => s.clone(),
            // Allow schema keywords as SQL identifiers (e.g. `model`, `enum`)
            other => other.to_string(),
        };

        if self.peek() == Some(&TokenKind::LParen) {
            self.advance();
            let mut args = Vec::new();
            if self.peek() != Some(&TokenKind::RParen) {
                args.push(self.parse_expr()?);
                while self.peek() == Some(&TokenKind::Comma) {
                    self.advance();
                    args.push(self.parse_expr()?);
                }
            }
            match self.peek() {
                Some(TokenKind::RParen) => {
                    self.advance();
                    Ok(SqlExpr::FnCall { name, args })
                }
                _ => Err(SchemaError::Parse(
                    format!("Expected ')' after arguments of function '{}'", name),
                    self.span(),
                )),
            }
        } else {
            Ok(SqlExpr::Ident(name))
        }
    }
}
/// Parse a slice of schema tokens into a validated [`SqlExpr`] tree.
///
/// The token slice should contain **only** the expression tokens (i.e. without
/// the surrounding `@computed(` ... `, Stored)` scaffolding).
///
/// `fallback_span` is used for error reporting when the slice is empty.
pub fn parse_sql_expr(tokens: &[Token], fallback_span: Span) -> Result<SqlExpr> {
    if tokens.is_empty() {
        return Err(SchemaError::Parse(
            "@computed expression is empty".to_string(),
            fallback_span,
        ));
    }

    let mut parser = SqlExprParser::new(tokens, fallback_span);
    let expr = parser.parse_expr()?;

    if !parser.at_end() {
        return Err(SchemaError::Parse(
            format!(
                "Unexpected token '{}' after SQL expression",
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

    /// Tokenise a raw string (skipping newlines) for expression parsing.
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

    fn parse(src: &str) -> SqlExpr {
        let tokens = tokenize(src);
        parse_sql_expr(&tokens, Span::new(0, 0)).expect("parse error")
    }

    fn parse_err(src: &str) -> String {
        let tokens = tokenize(src);
        match parse_sql_expr(&tokens, Span::new(0, 0)) {
            Err(e) => format!("{}", e),
            Ok(expr) => panic!("Expected error, got: {:?}", expr),
        }
    }

    #[test]
    fn simple_ident() {
        assert_eq!(parse("price").to_string(), "price");
    }

    #[test]
    fn binary_mul() {
        let expr = parse("price * quantity");
        assert_eq!(expr.to_string(), "price * quantity");
    }

    #[test]
    fn precedence_add_mul() {
        let expr = parse("a + b * c");
        assert!(matches!(expr, SqlExpr::BinaryOp { op: BinOp::Add, .. }));
    }

    #[test]
    fn concat_operator() {
        let expr = parse("first_name || \" \" || last_name");
        assert_eq!(expr.to_string(), "first_name || \" \" || last_name");
    }

    #[test]
    fn function_call() {
        let expr = parse("COALESCE(a, b)");
        assert!(matches!(expr, SqlExpr::FnCall { .. }));
        assert_eq!(expr.to_string(), "COALESCE(a, b)");
    }

    #[test]
    fn nested_function() {
        let expr = parse("UPPER(TRIM(name))");
        assert_eq!(expr.to_string(), "UPPER(TRIM(name))");
    }

    #[test]
    fn paren_expr() {
        let expr = parse("(a + b) * c");
        assert_eq!(expr.to_string(), "(a + b) * c");
    }

    #[test]
    fn unary_neg() {
        let expr = parse("-amount");
        assert_eq!(expr.to_string(), "-amount");
    }

    #[test]
    fn number_literal() {
        let expr = parse("score * 10");
        assert_eq!(expr.to_string(), "score * 10");
    }

    #[test]
    fn boolean_literal() {
        let expr = parse("true");
        assert_eq!(expr.to_string(), "true");
    }

    #[test]
    fn complex_expr() {
        let expr = parse("(price * quantity) - COALESCE(discount, 0)");
        assert_eq!(
            expr.to_string(),
            "(price * quantity) - COALESCE(discount, 0)"
        );
    }
    #[test]
    fn empty_is_error() {
        let tokens: Vec<Token> = vec![];
        assert!(parse_sql_expr(&tokens, Span::new(0, 0)).is_err());
    }

    #[test]
    fn only_operators_is_error() {
        let err = parse_err("* * *");
        assert!(err.contains("Unexpected token"));
    }

    #[test]
    fn trailing_operator_is_error() {
        let err = parse_err("a +");
        assert!(err.contains("Unexpected end"));
    }

    #[test]
    fn unclosed_paren_is_error() {
        let err = parse_err("(a + b");
        assert!(err.contains("Expected ')'"));
    }

    #[test]
    fn double_operator_is_error() {
        let err = parse_err("a + * b");
        assert!(err.contains("Unexpected token"));
    }
}
