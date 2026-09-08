//! The top-level blocks: `import`, `datasource`, `generator`, `model`, `view`,
//! `type` and `enum`.

use crate::ast::*;
use crate::error::Result;
use crate::token::TokenKind;

use super::Parser;

impl<'a> Parser<'a> {
    /// Parses an `import` statement.
    pub(super) fn parse_import(&mut self) -> Result<ImportDecl> {
        let start = self.expect(TokenKind::Import)?.span;
        let path_span = self.current_span();
        let path = self.parse_string()?;
        Ok(ImportDecl {
            path,
            path_span,
            span: start.merge(path_span),
        })
    }

    /// Parses a datasource block.
    pub(super) fn parse_datasource(&mut self) -> Result<DatasourceDecl> {
        let start = self.expect(TokenKind::Datasource)?.span;
        let name = self.parse_ident()?;
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut fields = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            fields.push(self.parse_config_field()?);
            self.skip_newlines();
        }

        let end = self.expect(TokenKind::RBrace)?.span;
        Ok(DatasourceDecl {
            name,
            fields,
            span: start.merge(end),
        })
    }

    /// Parses a generator block.
    pub(super) fn parse_generator(&mut self) -> Result<GeneratorDecl> {
        let start = self.expect(TokenKind::Generator)?.span;
        let name = self.parse_ident()?;
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut fields = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            fields.push(self.parse_config_field()?);
            self.skip_newlines();
        }

        let end = self.expect(TokenKind::RBrace)?.span;
        Ok(GeneratorDecl {
            name,
            fields,
            span: start.merge(end),
        })
    }

    /// Parses a configuration field (key = value).
    pub(super) fn parse_config_field(&mut self) -> Result<ConfigField> {
        let name = self.parse_ident()?;
        self.expect(TokenKind::Equal)?;
        let value = self.parse_expr()?;
        let span = name.span.merge(value.span());
        Ok(ConfigField { name, value, span })
    }

    /// Parses a model block.
    pub(super) fn parse_model(&mut self) -> Result<ModelDecl> {
        self.parse_model_block(TokenKind::Model, false)
    }

    /// Parses a `view` block.
    ///
    /// A view has the same body as a model; the keyword is the only difference
    /// the parser records.
    pub(super) fn parse_view(&mut self) -> Result<ModelDecl> {
        self.parse_model_block(TokenKind::View, true)
    }

    pub(super) fn parse_model_block(
        &mut self,
        keyword: TokenKind,
        is_view: bool,
    ) -> Result<ModelDecl> {
        let start = self.expect(keyword)?.span;
        let name = self.parse_ident()?;
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut fields = Vec::new();
        let mut attributes = Vec::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            if self.check(TokenKind::AtAt) {
                attributes.push(self.parse_model_attribute()?);
            } else {
                fields.push(self.parse_field_decl()?);
            }
            self.skip_newlines();
        }

        let end = self.expect(TokenKind::RBrace)?.span;
        Ok(ModelDecl {
            name,
            fields,
            attributes,
            is_view,
            span: start.merge(end),
        })
    }

    /// Parses a composite type block.
    pub(super) fn parse_type_decl(&mut self) -> Result<TypeDecl> {
        let start = self.expect(TokenKind::Type)?.span;
        let name = self.parse_ident()?;
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut fields = Vec::new();
        let mut attributes = Vec::new();

        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            // Composite types only allow `@@map`; the validator rejects any
            // other type-level attribute parsed here.
            if self.check(TokenKind::AtAt) {
                attributes.push(self.parse_model_attribute()?);
            } else {
                fields.push(self.parse_field_decl()?);
            }
            self.skip_newlines();
        }

        let end = self.expect(TokenKind::RBrace)?.span;
        Ok(TypeDecl {
            name,
            fields,
            attributes,
            span: start.merge(end),
        })
    }

    /// Parses an enum block.
    pub(super) fn parse_enum(&mut self) -> Result<EnumDecl> {
        let start = self.expect(TokenKind::Enum)?.span;
        let name = self.parse_ident()?;
        self.expect(TokenKind::LBrace)?;
        self.skip_newlines();

        let mut variants = Vec::new();
        while !self.check(TokenKind::RBrace) && !self.is_at_end() {
            let variant_name = self.parse_ident()?;
            let variant_span = variant_name.span;
            variants.push(EnumVariant {
                name: variant_name,
                span: variant_span,
            });
            self.skip_newlines();
        }

        let end = self.expect(TokenKind::RBrace)?.span;
        Ok(EnumDecl {
            name,
            variants,
            span: start.merge(end),
        })
    }
}
