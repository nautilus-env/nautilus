//! A field inside a `model` or a `type`: its name, its type, the modifier that
//! follows it and the attributes attached to it, plus the model-level
//! attributes written in the same block.

use crate::ast::*;
use crate::error::{Result, SchemaError};
use crate::token::TokenKind;

use super::Parser;

impl<'a> Parser<'a> {
    /// Parses a field declaration.
    pub(super) fn parse_field_decl(&mut self) -> Result<FieldDecl> {
        let name = self.parse_ident()?;
        let field_type = self.parse_field_type()?;
        let modifier = self.parse_field_modifier()?;
        let base_span = name.span.merge(self.previous_span());

        let mut attributes = Vec::new();
        while self.check(TokenKind::At) && !self.check(TokenKind::AtAt) {
            attributes.push(self.parse_field_attribute()?);
        }

        let span = if let Some(last_attr) = attributes.last() {
            match last_attr {
                FieldAttribute::Id => base_span,
                FieldAttribute::Unique => base_span,
                FieldAttribute::UpdatedAt { span } => base_span.merge(*span),
                FieldAttribute::Default(_, span) => base_span.merge(*span),
                FieldAttribute::Map(_) => base_span,
                FieldAttribute::Store { .. } => base_span,
                FieldAttribute::Relation { span, .. } => base_span.merge(*span),
                FieldAttribute::Computed { span, .. } => base_span.merge(*span),
                FieldAttribute::Check { span, .. } => base_span.merge(*span),
                FieldAttribute::Ignore { span } => base_span.merge(*span),
            }
        } else {
            base_span
        };

        Ok(FieldDecl {
            name,
            field_type,
            modifier,
            attributes,
            span,
        })
    }

    /// Parses a field type.
    pub(super) fn parse_field_type(&mut self) -> Result<FieldType> {
        let ident = self.parse_ident()?;

        let field_type = match ident.value.as_str() {
            "String" => FieldType::String,
            "Boolean" => FieldType::Boolean,
            "Int" => FieldType::Int,
            "BigInt" => FieldType::BigInt,
            "Float" => FieldType::Float,
            "DateTime" => FieldType::DateTime,
            "Bytes" => FieldType::Bytes,
            "Json" => FieldType::Json,
            "Uuid" => FieldType::Uuid,
            "Citext" => FieldType::Citext,
            "Hstore" => FieldType::Hstore,
            "Ltree" => FieldType::Ltree,
            "Geometry" => FieldType::Geometry,
            "Geography" => FieldType::Geography,
            "Vector" => {
                self.expect(TokenKind::LParen)?;
                let dimension = self.parse_number()?.parse::<u32>().map_err(|_| {
                    SchemaError::Parse(
                        "Invalid vector dimension value".to_string(),
                        self.current_span(),
                    )
                })?;
                self.expect(TokenKind::RParen)?;
                FieldType::Vector { dimension }
            }
            "Jsonb" => FieldType::Jsonb,
            "Xml" => FieldType::Xml,
            "Char" => {
                self.expect(TokenKind::LParen)?;
                let length = self.parse_number()?.parse::<u32>().map_err(|_| {
                    SchemaError::Parse("Invalid length value".to_string(), self.current_span())
                })?;
                self.expect(TokenKind::RParen)?;
                FieldType::Char { length }
            }
            "VarChar" => {
                self.expect(TokenKind::LParen)?;
                let length = self.parse_number()?.parse::<u32>().map_err(|_| {
                    SchemaError::Parse("Invalid length value".to_string(), self.current_span())
                })?;
                self.expect(TokenKind::RParen)?;
                FieldType::VarChar { length }
            }
            "Decimal" => {
                if self.check(TokenKind::LParen) {
                    self.advance();
                    let precision = self.parse_number()?.parse::<u32>().map_err(|_| {
                        SchemaError::Parse(
                            "Invalid precision value".to_string(),
                            self.current_span(),
                        )
                    })?;
                    self.expect(TokenKind::Comma)?;
                    let scale = self.parse_number()?.parse::<u32>().map_err(|_| {
                        SchemaError::Parse("Invalid scale value".to_string(), self.current_span())
                    })?;
                    self.expect(TokenKind::RParen)?;
                    FieldType::Decimal { precision, scale }
                } else {
                    return Err(SchemaError::Parse(
                        "Decimal type requires precision and scale: Decimal(p, s)".to_string(),
                        ident.span,
                    ));
                }
            }
            _ => FieldType::UserType(ident.value),
        };

        Ok(field_type)
    }

    /// Parses field modifiers (?, !, or []).
    pub(super) fn parse_field_modifier(&mut self) -> Result<FieldModifier> {
        if self.check(TokenKind::Question) {
            self.advance();
            Ok(FieldModifier::Optional)
        } else if self.check(TokenKind::Bang) {
            self.advance();
            Ok(FieldModifier::NotNull)
        } else if self.check(TokenKind::LBracket) {
            self.advance();
            self.expect(TokenKind::RBracket)?;
            Ok(FieldModifier::Array)
        } else {
            Ok(FieldModifier::None)
        }
    }

    /// Parses a field attribute (@id, @unique, etc.).
    pub(super) fn parse_field_attribute(&mut self) -> Result<FieldAttribute> {
        let at_token = self.expect(TokenKind::At)?;
        let at_span = at_token.span;
        let name = self.parse_ident()?;

        match name.value.as_str() {
            "id" => Ok(FieldAttribute::Id),
            "unique" => Ok(FieldAttribute::Unique),
            "ignore" => Ok(FieldAttribute::Ignore {
                span: at_span.merge(name.span),
            }),
            "updatedAt" => Ok(FieldAttribute::UpdatedAt {
                span: at_span.merge(name.span),
            }),
            "default" => {
                self.expect(TokenKind::LParen)?;
                let expr = self.parse_expr()?;
                let rparen = self.expect(TokenKind::RParen)?;
                let full_span = at_span.merge(rparen.span);
                Ok(FieldAttribute::Default(expr, full_span))
            }
            "map" => {
                self.expect(TokenKind::LParen)?;
                let map_name = self.parse_string()?;
                self.expect(TokenKind::RParen)?;
                Ok(FieldAttribute::Map(map_name))
            }
            "store" => {
                let start = name.span;
                self.expect(TokenKind::LParen)?;
                let strategy_ident = self.parse_ident()?;
                let strategy = match strategy_ident.value.as_str() {
                    "json" => StorageStrategy::Json,
                    "native" => StorageStrategy::Native,
                    _ => {
                        return Err(SchemaError::Parse(
                            format!(
                                "Unknown storage strategy: '{}'. Valid options: 'json', 'native'",
                                strategy_ident.value
                            ),
                            strategy_ident.span,
                        ))
                    }
                };
                let end = self.expect(TokenKind::RParen)?.span;
                Ok(FieldAttribute::Store {
                    strategy,
                    span: start.merge(end),
                })
            }
            "relation" => {
                let start = name.span;
                self.expect(TokenKind::LParen)?;

                let mut rel_name = None;
                let mut fields = None;
                let mut references = None;
                let mut on_delete = None;
                let mut on_update = None;

                if matches!(self.peek_kind(), Some(TokenKind::String(_))) {
                    rel_name = Some(self.parse_string()?);

                    if self.check(TokenKind::Comma) {
                        self.advance();
                    }
                }

                while !self.check(TokenKind::RParen) && !self.is_at_end() {
                    let arg_name = self.parse_ident()?;
                    self.expect(TokenKind::Colon)?;

                    match arg_name.value.as_str() {
                        "name" => rel_name = Some(self.parse_string()?),
                        "fields" => fields = Some(self.parse_ident_array()?),
                        "references" => references = Some(self.parse_ident_array()?),
                        "onDelete" => on_delete = Some(self.parse_referential_action()?),
                        "onUpdate" => on_update = Some(self.parse_referential_action()?),
                        _ => {
                            return Err(SchemaError::Parse(
                                format!("Unknown relation argument: {}", arg_name.value),
                                arg_name.span,
                            ))
                        }
                    }

                    if self.check(TokenKind::Comma) {
                        self.advance();
                    }
                }

                let end = self.expect(TokenKind::RParen)?.span;
                Ok(FieldAttribute::Relation {
                    name: rel_name,
                    fields,
                    references,
                    on_delete,
                    on_update,
                    span: start.merge(end),
                })
            }
            "computed" => {
                let start = at_span;
                self.expect(TokenKind::LParen)?;
                let expr = self.parse_sql_expr()?;
                self.expect(TokenKind::Comma)?;
                let kind_ident = self.parse_ident()?;
                let kind = match kind_ident.value.as_str() {
                    "Stored" => ComputedKind::Stored,
                    "Virtual" => ComputedKind::Virtual,
                    other => {
                        return Err(SchemaError::Parse(
                            format!(
                                "Unknown computed kind '{}'. Valid options: Stored, Virtual",
                                other
                            ),
                            kind_ident.span,
                        ))
                    }
                };
                let end = self.expect(TokenKind::RParen)?.span;
                Ok(FieldAttribute::Computed {
                    expr,
                    kind,
                    span: start.merge(end),
                })
            }
            "check" => {
                let start = at_span;
                self.expect(TokenKind::LParen)?;
                let expr = self.parse_bool_expr()?;
                let end = self.expect(TokenKind::RParen)?.span;
                Ok(FieldAttribute::Check {
                    expr,
                    span: start.merge(end),
                })
            }
            _ => Err(SchemaError::Parse(
                format!("Unknown field attribute: @{}", name.value),
                name.span,
            )),
        }
    }

    /// Parses a model attribute (@@map, @@id, etc.).
    pub(super) fn parse_model_attribute(&mut self) -> Result<ModelAttribute> {
        self.expect(TokenKind::AtAt)?;
        let name = self.parse_ident()?;

        match name.value.as_str() {
            "map" => {
                self.expect(TokenKind::LParen)?;
                let map_name = self.parse_string()?;
                self.expect(TokenKind::RParen)?;
                Ok(ModelAttribute::Map(map_name))
            }
            "id" => {
                self.expect(TokenKind::LParen)?;
                let fields = self.parse_ident_array()?;
                self.expect(TokenKind::RParen)?;
                Ok(ModelAttribute::Id(fields))
            }
            "unique" => {
                self.expect(TokenKind::LParen)?;
                let fields = self.parse_ident_array()?;
                self.expect(TokenKind::RParen)?;
                Ok(ModelAttribute::Unique(fields))
            }
            "index" => {
                let start = name.span;
                self.expect(TokenKind::LParen)?;
                let fields = self.parse_ident_array()?;
                let mut index_type: Option<Ident> = None;
                let mut opclass: Option<Ident> = None;
                let mut m: Option<u32> = None;
                let mut ef_construction: Option<u32> = None;
                let mut lists: Option<u32> = None;
                let mut index_name: Option<String> = None;
                let mut index_map: Option<String> = None;
                let mut predicate: Option<crate::bool_expr::BoolExpr> = None;
                while self.check(TokenKind::Comma) {
                    self.advance();
                    if self.check(TokenKind::RParen) {
                        break;
                    }
                    let key = self.parse_ident()?;
                    self.expect(TokenKind::Colon)?;
                    match key.value.as_str() {
                        "type" => {
                            index_type = Some(self.parse_ident()?);
                        }
                        "opclass" => {
                            opclass = Some(self.parse_ident()?);
                        }
                        "m" => {
                            m = Some(self.parse_u32_literal("m")?);
                        }
                        "ef_construction" => {
                            ef_construction = Some(self.parse_u32_literal("ef_construction")?);
                        }
                        "lists" => {
                            lists = Some(self.parse_u32_literal("lists")?);
                        }
                        "name" => {
                            index_name = Some(self.parse_string()?);
                        }
                        "map" => {
                            index_map = Some(self.parse_string()?);
                        }
                        "where" => {
                            predicate = Some(self.parse_bool_expr_until(true)?);
                        }
                        _ => {
                            return Err(SchemaError::Parse(
                                format!("Unknown @@index argument: '{}'", key.value),
                                key.span,
                            ));
                        }
                    }
                }
                let end = self.expect(TokenKind::RParen)?.span;
                Ok(ModelAttribute::Index {
                    fields,
                    index_type,
                    opclass,
                    m,
                    ef_construction,
                    lists,
                    name: index_name,
                    map: index_map,
                    predicate,
                    span: start.merge(end),
                })
            }
            "check" => {
                self.expect(TokenKind::LParen)?;
                let expr = self.parse_bool_expr()?;
                let end = self.expect(TokenKind::RParen)?.span;
                Ok(ModelAttribute::Check {
                    expr,
                    span: name.span.merge(end),
                })
            }
            "ignore" => Ok(ModelAttribute::Ignore { span: name.span }),
            "schema" => {
                self.expect(TokenKind::LParen)?;
                let schema_name = self.parse_string()?;
                let end = self.expect(TokenKind::RParen)?.span;
                Ok(ModelAttribute::Schema {
                    name: schema_name,
                    span: name.span.merge(end),
                })
            }
            _ => Err(SchemaError::Parse(
                format!("Unknown model attribute: @@{}", name.value),
                name.span,
            )),
        }
    }

    /// Parses an array of identifiers [a, b, c].
    pub(super) fn parse_ident_array(&mut self) -> Result<Vec<Ident>> {
        self.expect(TokenKind::LBracket)?;
        let mut idents = Vec::new();

        while !self.check(TokenKind::RBracket) && !self.is_at_end() {
            idents.push(self.parse_ident()?);
            if self.check(TokenKind::Comma) {
                self.advance();
            }
        }

        self.expect(TokenKind::RBracket)?;
        Ok(idents)
    }

    /// Parses a referential action (Cascade, SetNull, etc.).
    pub(super) fn parse_referential_action(&mut self) -> Result<ReferentialAction> {
        let ident = self.parse_ident()?;
        match ident.value.as_str() {
            "Cascade" => Ok(ReferentialAction::Cascade),
            "Restrict" => Ok(ReferentialAction::Restrict),
            "NoAction" => Ok(ReferentialAction::NoAction),
            "SetNull" => Ok(ReferentialAction::SetNull),
            "SetDefault" => Ok(ReferentialAction::SetDefault),
            _ => Err(SchemaError::Parse(
                format!("Unknown referential action: {}", ident.value),
                ident.span,
            )),
        }
    }
}
