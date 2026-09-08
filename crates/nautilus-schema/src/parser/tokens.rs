//! Reading the token stream: the primitives every other parser method is
//! written in, and the recovery that finds the next declaration after an error.

use crate::ast::Ident;
use crate::error::{Result, SchemaError};
use crate::span::Span;
use crate::token::{Token, TokenKind};

use super::Parser;

impl<'a> Parser<'a> {
    /// Parses an identifier.
    pub(super) fn parse_ident(&mut self) -> Result<Ident> {
        match self.peek_kind() {
            Some(TokenKind::Ident(ref name)) => {
                let name = name.clone();
                let span = self.advance().span;
                Ok(Ident::new(name, span))
            }
            // Allow `type` keyword as an identifier in contexts like
            // named arguments (e.g., `type: Hash` in @@index)
            Some(TokenKind::Type) => {
                let span = self.advance().span;
                Ok(Ident::new("type".to_string(), span))
            }
            // `import` is only a keyword at the top level, so a field or
            // argument may still be called `import`.
            Some(TokenKind::Import) => {
                let span = self.advance().span;
                Ok(Ident::new("import".to_string(), span))
            }
            Some(kind) => Err(SchemaError::Parse(
                format!("Expected identifier, found {:?}", kind),
                self.current_span(),
            )),
            None => Err(SchemaError::Parse(
                "Expected identifier, found EOF".to_string(),
                self.current_span(),
            )),
        }
    }

    /// Parses a string literal.
    pub(super) fn parse_string(&mut self) -> Result<String> {
        match self.peek_kind() {
            Some(TokenKind::String(ref s)) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            Some(kind) => Err(SchemaError::Parse(
                format!("Expected string, found {:?}", kind),
                self.current_span(),
            )),
            None => Err(SchemaError::Parse(
                "Expected string, found EOF".to_string(),
                self.current_span(),
            )),
        }
    }

    /// Parses a number literal.
    pub(super) fn parse_number(&mut self) -> Result<String> {
        match self.peek_kind() {
            Some(TokenKind::Number(ref n)) => {
                let n = n.clone();
                self.advance();
                Ok(n)
            }
            Some(kind) => Err(SchemaError::Parse(
                format!("Expected number, found {:?}", kind),
                self.current_span(),
            )),
            None => Err(SchemaError::Parse(
                "Expected number, found EOF".to_string(),
                self.current_span(),
            )),
        }
    }

    /// Parses an unsigned integer literal used by index arguments.
    pub(super) fn parse_u32_literal(&mut self, argument_name: &str) -> Result<u32> {
        let raw = self.parse_number()?;
        let span = self.previous_span();
        raw.parse::<u32>().map_err(|_| {
            SchemaError::Parse(
                format!(
                    "Expected '{}' to be a non-negative integer literal, found '{}'",
                    argument_name, raw
                ),
                span,
            )
        })
    }

    /// Checks if current token matches the given kind.
    pub(super) fn check(&self, kind: TokenKind) -> bool {
        self.peek_kind()
            .map(|k| std::mem::discriminant(&k) == std::mem::discriminant(&kind))
            .unwrap_or(false)
    }

    /// Expects the current token to be of the given kind, advances, returns token.
    pub(super) fn expect(&mut self, kind: TokenKind) -> Result<&'a Token> {
        if self.check(kind.clone()) {
            Ok(self.advance())
        } else {
            Err(SchemaError::Parse(
                format!("Expected {:?}, found {:?}", kind, self.peek_kind()),
                self.current_span(),
            ))
        }
    }

    /// Peeks at the current token kind.
    pub(super) fn peek_kind(&self) -> Option<TokenKind> {
        self.tokens.get(self.pos).map(|t| t.kind.clone())
    }

    /// Peeks at the token kind `offset` positions ahead of the cursor.
    ///
    /// Used for single-token lookahead (e.g. deciding whether an identifier
    /// followed by `=` should be parsed as a named function-call argument).
    pub(super) fn peek_kind_at(&self, offset: usize) -> Option<TokenKind> {
        self.tokens.get(self.pos + offset).map(|t| t.kind.clone())
    }

    /// Advances to the next token, returns current token.
    pub(super) fn advance(&mut self) -> &'a Token {
        let token = &self.tokens[self.pos];
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        token
    }

    /// Returns the span of the current token.
    pub(super) fn current_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| t.span)
            .unwrap_or_else(|| self.previous_span())
    }

    /// Returns the span of the previous token.
    pub(super) fn previous_span(&self) -> Span {
        if self.pos > 0 {
            self.tokens[self.pos - 1].span
        } else {
            Span::new(0, 0)
        }
    }

    /// Checks if we're at the end of the token stream.
    pub(super) fn is_at_end(&self) -> bool {
        self.pos >= self.tokens.len() || matches!(self.peek_kind(), Some(TokenKind::Eof))
    }

    /// Skips newline tokens.
    pub(super) fn skip_newlines(&mut self) {
        while matches!(self.peek_kind(), Some(TokenKind::Newline)) {
            self.advance();
        }
    }

    /// Recovers to the next declaration (for error recovery).
    pub(super) fn recover_to_next_declaration(&mut self) {
        while !self.is_at_end() {
            match self.peek_kind() {
                Some(TokenKind::Import)
                | Some(TokenKind::Datasource)
                | Some(TokenKind::Generator)
                | Some(TokenKind::Model)
                | Some(TokenKind::View)
                | Some(TokenKind::Enum)
                | Some(TokenKind::Type) => break,
                _ => {
                    self.advance();
                }
            }
        }
    }
}
