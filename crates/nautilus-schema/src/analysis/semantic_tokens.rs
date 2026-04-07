//! Semantic token provider for `.nautilus` schema files.
//!
//! Produces a list of [`SemanticToken`]s by walking the AST and resolving
//! every `UserType` field reference to either a model or an enum, using
//! the parsed token stream to recover the exact source span.

use crate::ast::{Declaration, FieldType, Schema};
use crate::span::Span;
use crate::token::{Token, TokenKind};
use std::collections::HashSet;

/// The semantic category of a type reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticKind {
    /// Reference to a `model` declaration.
    ModelRef,
    /// Reference to an `enum` declaration.
    EnumRef,
    /// Reference to a `type` (composite type) declaration.
    CompositeTypeRef,
}

/// A single semantic token: a source span and its semantic kind.
#[derive(Debug, Clone)]
pub struct SemanticToken {
    /// The source span of the token.
    pub span: Span,
    /// The semantic kind (model or enum reference).
    pub kind: SemanticKind,
}

/// Walk `ast` and return semantic tokens for every user-type field reference.
///
/// The `tokens` slice (from the lexer) is used to recover the precise span of
/// each type name — the AST's `FieldType::UserType` only stores the name string.
pub fn semantic_tokens(ast: &Schema, tokens: &[Token]) -> Vec<SemanticToken> {
    let model_names: HashSet<&str> = ast.models().map(|m| m.name.value.as_str()).collect();
    let enum_names: HashSet<&str> = ast.enums().map(|e| e.name.value.as_str()).collect();
    let type_names: HashSet<&str> = ast.types().map(|t| t.name.value.as_str()).collect();

    let mut result = Vec::new();

    for decl in &ast.declarations {
        match decl {
            Declaration::Model(model) => {
                collect_field_tokens(
                    &model.fields,
                    tokens,
                    &model_names,
                    &enum_names,
                    &type_names,
                    &mut result,
                );
            }
            Declaration::Type(type_decl) => {
                collect_field_tokens(
                    &type_decl.fields,
                    tokens,
                    &model_names,
                    &enum_names,
                    &type_names,
                    &mut result,
                );
            }
            _ => {}
        }
    }

    result.sort_by_key(|t| t.span.start);
    result
}

fn collect_field_tokens(
    fields: &[crate::ast::FieldDecl],
    tokens: &[Token],
    model_names: &HashSet<&str>,
    enum_names: &HashSet<&str>,
    type_names: &HashSet<&str>,
    result: &mut Vec<SemanticToken>,
) {
    for field in fields {
        if let FieldType::UserType(type_name) = &field.field_type {
            let kind = if type_names.contains(type_name.as_str()) {
                SemanticKind::CompositeTypeRef
            } else if model_names.contains(type_name.as_str()) {
                SemanticKind::ModelRef
            } else if enum_names.contains(type_name.as_str()) {
                SemanticKind::EnumRef
            } else {
                continue;
            };

            if let Some(tok) = tokens.iter().find(|t| {
                matches!(&t.kind, TokenKind::Ident(name) if name == type_name)
                    && t.span.start >= field.span.start
                    && t.span.end <= field.span.end
                    && t.span.start != field.name.span.start
            }) {
                result.push(SemanticToken {
                    span: tok.span,
                    kind,
                });
            }
        }
    }
}
