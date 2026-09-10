//! The expressions written inside an attribute's arguments: the values of
//! `@default`, the SQL of `@computed` and the boolean predicate of `@check`.

use crate::ast::*;
use crate::error::{Result, SchemaError};
use crate::token::TokenKind;

use super::Parser;

impl<'a> Parser<'a> {
    /// Collects SQL expression tokens until a top-level comma or closing paren,
    /// then parses them into a validated [`SqlExpr`](crate::sql_expr::SqlExpr) tree.
    pub(super) fn parse_sql_expr(&mut self) -> Result<crate::sql_expr::SqlExpr> {
        if self.pos >= self.tokens.len() {
            return Err(SchemaError::Parse(
                "Unexpected end of file in @computed expression".to_string(),
                self.current_span(),
            ));
        }
        let fallback_span = self.current_span();
        let expr_start = self.pos;
        let mut depth: i32 = 0;

        loop {
            match self.peek_kind() {
                Some(TokenKind::LParen) => {
                    depth += 1;
                    self.advance();
                }
                Some(TokenKind::RParen) if depth == 0 => break,
                Some(TokenKind::RParen) => {
                    depth -= 1;
                    self.advance();
                }
                Some(TokenKind::Comma) if depth == 0 => break,
                None | Some(TokenKind::Eof) => {
                    return Err(SchemaError::Parse(
                        "Unexpected end of file in @computed expression".to_string(),
                        self.current_span(),
                    ));
                }
                _ => {
                    self.advance();
                }
            }
        }

        let expr_tokens: Vec<_> = self.tokens[expr_start..self.pos]
            .iter()
            .filter(|t| !matches!(t.kind, TokenKind::Newline))
            .cloned()
            .collect();

        crate::sql_expr::parse_sql_expr(&expr_tokens, fallback_span)
    }

    /// Collects boolean expression tokens until a top-level closing paren,
    /// then parses them into a validated [`BoolExpr`](crate::bool_expr::BoolExpr) tree.
    pub(super) fn parse_bool_expr(&mut self) -> Result<crate::bool_expr::BoolExpr> {
        self.parse_bool_expr_until(false)
    }

    /// Same as [`Self::parse_bool_expr`], but also stops at a top-level comma.
    ///
    /// `@@index(...)` takes the predicate as one named argument among several,
    /// so the expression ends at whichever comes first: the argument separator
    /// or the closing paren. Bracket depth is tracked so that the commas inside
    /// an `IN [A, B]` list do not terminate the expression.
    pub(super) fn parse_bool_expr_until(
        &mut self,
        stop_at_comma: bool,
    ) -> Result<crate::bool_expr::BoolExpr> {
        if self.pos >= self.tokens.len() {
            return Err(SchemaError::Parse(
                "Unexpected end of file in @check expression".to_string(),
                self.current_span(),
            ));
        }
        let fallback_span = self.current_span();
        let expr_start = self.pos;
        let mut depth: i32 = 0;
        let mut bracket_depth: i32 = 0;

        loop {
            match self.peek_kind() {
                Some(TokenKind::LParen) => {
                    depth += 1;
                    self.advance();
                }
                Some(TokenKind::RParen) if depth == 0 => break,
                Some(TokenKind::RParen) => {
                    depth -= 1;
                    self.advance();
                }
                Some(TokenKind::LBracket) => {
                    bracket_depth += 1;
                    self.advance();
                }
                Some(TokenKind::RBracket) => {
                    bracket_depth -= 1;
                    self.advance();
                }
                Some(TokenKind::Comma) if stop_at_comma && depth == 0 && bracket_depth == 0 => {
                    break
                }
                None | Some(TokenKind::Eof) => {
                    return Err(SchemaError::Parse(
                        "Unexpected end of file in @check expression".to_string(),
                        self.current_span(),
                    ));
                }
                _ => {
                    self.advance();
                }
            }
        }

        let expr_tokens: Vec<_> = self.tokens[expr_start..self.pos]
            .iter()
            .filter(|t| !matches!(t.kind, TokenKind::Newline))
            .cloned()
            .collect();

        crate::bool_expr::parse_bool_expr(&expr_tokens, fallback_span)
    }

    /// Parses an expression.
    pub(super) fn parse_expr(&mut self) -> Result<Expr> {
        match self.peek_kind() {
            Some(TokenKind::String(_)) => {
                let s = self.parse_string()?;
                let span = self.previous_span();
                Ok(Expr::Literal(Literal::String(s, span)))
            }
            Some(TokenKind::Number(_)) => {
                let n = self.parse_number()?;
                let span = self.previous_span();
                Ok(Expr::Literal(Literal::Number(n, span)))
            }
            Some(TokenKind::True) => {
                let span = self.advance().span;
                Ok(Expr::Literal(Literal::Boolean(true, span)))
            }
            Some(TokenKind::False) => {
                let span = self.advance().span;
                Ok(Expr::Literal(Literal::Boolean(false, span)))
            }
            Some(TokenKind::LBracket) => self.parse_array_expr(),
            Some(TokenKind::Ident(_)) => {
                let ident = self.parse_ident()?;

                if self.check(TokenKind::LParen) {
                    let start = ident.span;
                    self.advance();

                    let mut args = Vec::new();
                    while !self.check(TokenKind::RParen) && !self.is_at_end() {
                        args.push(self.parse_call_argument()?);
                        if self.check(TokenKind::Comma) {
                            self.advance();
                        }
                    }

                    let end = self.expect(TokenKind::RParen)?.span;
                    Ok(Expr::FunctionCall {
                        name: ident,
                        args,
                        span: start.merge(end),
                    })
                } else {
                    Ok(Expr::Ident(ident))
                }
            }
            Some(kind) => Err(SchemaError::Parse(
                format!("Expected expression, found {:?}", kind),
                self.current_span(),
            )),
            None => Err(SchemaError::Parse(
                "Unexpected end of file in expression".to_string(),
                self.current_span(),
            )),
        }
    }

    /// Parses a single function-call argument.
    ///
    /// Supports two forms:
    /// - positional: any `parse_expr` value;
    /// - named: `ident = expr`, emitted as [`Expr::NamedArg`]. This is what
    ///   the structured `extension(name = ..., schema = ...)` datasource
    ///   entry relies on.
    pub(super) fn parse_call_argument(&mut self) -> Result<Expr> {
        if let Some(TokenKind::Ident(_)) = self.peek_kind() {
            if matches!(self.peek_kind_at(1), Some(TokenKind::Equal)) {
                let name = self.parse_ident()?;
                self.expect(TokenKind::Equal)?;
                let value = self.parse_expr()?;
                let span = name.span.merge(value.span());
                return Ok(Expr::NamedArg {
                    name,
                    value: Box::new(value),
                    span,
                });
            }
        }
        self.parse_expr()
    }

    /// Parses an array expression [a, b, c].
    pub(super) fn parse_array_expr(&mut self) -> Result<Expr> {
        let start = self.expect(TokenKind::LBracket)?.span;
        let mut elements = Vec::new();

        while !self.check(TokenKind::RBracket) && !self.is_at_end() {
            elements.push(self.parse_expr()?);
            if self.check(TokenKind::Comma) {
                self.advance();
            }
        }

        let end = self.expect(TokenKind::RBracket)?.span;
        Ok(Expr::Array {
            elements,
            span: start.merge(end),
        })
    }
}
