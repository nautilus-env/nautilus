//! Recursive descent parser for the nautilus schema language.
//!
//! This module provides a parser that transforms a stream of tokens into an AST.
//!
//! # Example
//!
//! ```ignore
//! use nautilus_schema::{Lexer, Parser};
//!
//! let source = r#"
//!     model User {
//!       id    Int    @id
//!       email String @unique
//!     }
//! "#;
//!
//! let tokens = Lexer::new(source).collect::<Result<Vec<_>, _>>().unwrap();
//! let schema = Parser::new(&tokens, source).parse_schema().unwrap();
//! ```

mod declarations;
mod expressions;
mod fields;
mod tokens;

use crate::ast::*;
use crate::error::{Result, SchemaError};
use crate::token::{Token, TokenKind};

/// Parser for schema files.
pub struct Parser<'a> {
    /// Token stream.
    tokens: &'a [Token],
    /// Current position in token stream.
    pos: usize,
    /// Errors collected during error-recovery (non-fatal parse failures).
    recovered_errors: Vec<SchemaError>,
}

impl<'a> Parser<'a> {
    /// Creates a new parser from a token slice and the original source text.
    pub fn new(tokens: &'a [Token], source: &'a str) -> Self {
        let _ = source; // kept in signature for API compatibility
        Self {
            tokens,
            pos: 0,
            recovered_errors: Vec::new(),
        }
    }

    /// Returns all errors that were silently recovered from during parsing.
    ///
    /// These are non-fatal: the parser managed to continue past them by
    /// skipping to the next top-level declaration.  Call this after
    /// [`parse_schema`] to collect the full set of parse diagnostics.
    pub fn take_errors(&mut self) -> Vec<SchemaError> {
        std::mem::take(&mut self.recovered_errors)
    }

    /// Parses a complete schema.
    pub fn parse_schema(&mut self) -> Result<Schema> {
        let start = self.current_span();
        self.skip_newlines();

        let mut declarations = Vec::new();

        while !self.is_at_end() {
            match self.parse_declaration() {
                Ok(decl) => declarations.push(decl),
                Err(e) => {
                    // Error recovery: record the error and skip to the next declaration.
                    self.recovered_errors.push(e);
                    self.recover_to_next_declaration();
                }
            }
            self.skip_newlines();
        }

        let end = self.previous_span();
        Ok(Schema::new(declarations, start.merge(end)))
    }

    /// Parses a top-level declaration.
    pub(super) fn parse_declaration(&mut self) -> Result<Declaration> {
        self.skip_newlines();

        match self.peek_kind() {
            Some(TokenKind::Import) => Ok(Declaration::Import(self.parse_import()?)),
            Some(TokenKind::Datasource) => Ok(Declaration::Datasource(self.parse_datasource()?)),
            Some(TokenKind::Generator) => Ok(Declaration::Generator(self.parse_generator()?)),
            Some(TokenKind::Model) => Ok(Declaration::Model(self.parse_model()?)),
            Some(TokenKind::View) => Ok(Declaration::Model(self.parse_view()?)),
            Some(TokenKind::Enum) => Ok(Declaration::Enum(self.parse_enum()?)),
            Some(TokenKind::Type) => Ok(Declaration::Type(self.parse_type_decl()?)),
            Some(kind) => Err(SchemaError::Parse(
                format!("Expected declaration, found {:?}", kind),
                self.current_span(),
            )),
            None => Err(SchemaError::Parse(
                "Unexpected end of file".to_string(),
                self.current_span(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    pub(super) fn tokenize(input: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(input);
        let mut tokens = Vec::new();
        loop {
            match lexer.next_token() {
                Ok(token) => {
                    if matches!(token.kind, TokenKind::Eof) {
                        tokens.push(token);
                        break;
                    }
                    tokens.push(token);
                }
                Err(e) => panic!("Tokenization failed: {}", e),
            }
        }
        tokens
    }

    #[test]
    pub(super) fn test_parse_empty_schema() {
        let input = "";
        let tokens = tokenize(input);
        let schema = Parser::new(&tokens, input).parse_schema().unwrap();
        assert_eq!(schema.declarations.len(), 0);
    }

    #[test]
    pub(super) fn test_parse_simple_model() {
        let input = r#"
            model User {
                id Int @id
            }
        "#;
        let tokens = tokenize(input);
        let schema = Parser::new(&tokens, input).parse_schema().unwrap();
        assert_eq!(schema.declarations.len(), 1);

        match &schema.declarations[0] {
            Declaration::Model(model) => {
                assert_eq!(model.name.value, "User");
                assert_eq!(model.fields.len(), 1);
                assert_eq!(model.fields[0].name.value, "id");
            }
            _ => panic!("Expected model declaration"),
        }
    }

    #[test]
    pub(super) fn test_parse_field_types() {
        let input = r#"
            model Test {
                str String
                num Int
                big BigInt
                opt String?
                arr Int[]
                dec Decimal(10, 2)
            }
        "#;
        let tokens = tokenize(input);
        let schema = Parser::new(&tokens, input).parse_schema().unwrap();

        match &schema.declarations[0] {
            Declaration::Model(model) => {
                assert_eq!(model.fields.len(), 6);
                assert!(matches!(model.fields[0].field_type, FieldType::String));
                assert!(matches!(model.fields[1].field_type, FieldType::Int));
                assert!(matches!(model.fields[2].field_type, FieldType::BigInt));
                assert!(model.fields[3].is_optional());
                assert!(model.fields[4].is_array());
                assert!(matches!(
                    model.fields[5].field_type,
                    FieldType::Decimal {
                        precision: 10,
                        scale: 2
                    }
                ));
            }
            _ => panic!("Expected model"),
        }
    }

    #[test]
    pub(super) fn test_parse_field_attributes() {
        let input = r#"
            model User {
                id Int @id @default(autoincrement())
                email String @unique @map("user_email")
            }
        "#;
        let tokens = tokenize(input);
        let schema = Parser::new(&tokens, input).parse_schema().unwrap();

        match &schema.declarations[0] {
            Declaration::Model(model) => {
                assert_eq!(model.fields[0].attributes.len(), 2);
                assert!(matches!(model.fields[0].attributes[0], FieldAttribute::Id));
                assert!(matches!(
                    model.fields[0].attributes[1],
                    FieldAttribute::Default(..)
                ));

                assert_eq!(model.fields[1].attributes.len(), 2);
                assert!(matches!(
                    model.fields[1].attributes[0],
                    FieldAttribute::Unique
                ));
                assert!(matches!(
                    model.fields[1].attributes[1],
                    FieldAttribute::Map(_)
                ));
            }
            _ => panic!("Expected model"),
        }
    }

    #[test]
    pub(super) fn test_parse_model_attributes() {
        let input = r#"
            model User {
                id Int
                @@map("users")
                @@id([id])
            }
        "#;
        let tokens = tokenize(input);
        let schema = Parser::new(&tokens, input).parse_schema().unwrap();

        match &schema.declarations[0] {
            Declaration::Model(model) => {
                assert_eq!(model.attributes.len(), 2);
                assert!(matches!(model.attributes[0], ModelAttribute::Map(_)));
                assert!(matches!(model.attributes[1], ModelAttribute::Id(_)));
            }
            _ => panic!("Expected model"),
        }
    }

    #[test]
    pub(super) fn test_parse_enum() {
        let input = r#"
            enum Role {
                USER
                ADMIN
            }
        "#;
        let tokens = tokenize(input);
        let schema = Parser::new(&tokens, input).parse_schema().unwrap();

        match &schema.declarations[0] {
            Declaration::Enum(enum_decl) => {
                assert_eq!(enum_decl.name.value, "Role");
                assert_eq!(enum_decl.variants.len(), 2);
                assert_eq!(enum_decl.variants[0].name.value, "USER");
                assert_eq!(enum_decl.variants[1].name.value, "ADMIN");
            }
            _ => panic!("Expected enum"),
        }
    }

    #[test]
    pub(super) fn test_parse_datasource() {
        let input = r#"
            datasource db {
                provider = "postgresql"
                url = env("DATABASE_URL")
            }
        "#;
        let tokens = tokenize(input);
        let schema = Parser::new(&tokens, input).parse_schema().unwrap();

        match &schema.declarations[0] {
            Declaration::Datasource(ds) => {
                assert_eq!(ds.name.value, "db");
                assert_eq!(ds.fields.len(), 2);
                assert_eq!(ds.provider(), Some("postgresql"));
            }
            _ => panic!("Expected datasource"),
        }
    }

    #[test]
    pub(super) fn test_parse_generator() {
        let input = r#"
            generator client {
                provider = "nautilus-client-rs"
                output = "../generated"
            }
        "#;
        let tokens = tokenize(input);
        let schema = Parser::new(&tokens, input).parse_schema().unwrap();

        match &schema.declarations[0] {
            Declaration::Generator(gen) => {
                assert_eq!(gen.name.value, "client");
                assert_eq!(gen.fields.len(), 2);
            }
            _ => panic!("Expected generator"),
        }
    }

    #[test]
    pub(super) fn test_parse_relation() {
        let input = r#"
            model Post {
                userId Int
                user User @relation(fields: [userId], references: [id], onDelete: Cascade)
            }
        "#;
        let tokens = tokenize(input);
        let schema = Parser::new(&tokens, input).parse_schema().unwrap();

        match &schema.declarations[0] {
            Declaration::Model(model) => match &model.fields[1].attributes[0] {
                FieldAttribute::Relation {
                    fields,
                    references,
                    on_delete,
                    ..
                } => {
                    assert!(fields.is_some());
                    assert!(references.is_some());
                    assert_eq!(*on_delete, Some(ReferentialAction::Cascade));
                }
                _ => panic!("Expected relation attribute"),
            },
            _ => panic!("Expected model"),
        }
    }
}
