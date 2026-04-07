//! Goto-definition support for `.nautilus` schema files.

use super::{analyze, span_contains, AnalysisResult};
use crate::ast::{Declaration, FieldType, Schema};
use crate::span::Span;

/// Returns the span of the declaration that the symbol at `offset` refers to.
///
/// Handles:
/// - Field types of the form `UserType(name)` — resolves to the `ModelDecl`
///   or `EnumDecl` with that name.
///
/// Returns `None` if the offset does not sit on a resolvable reference.
pub fn goto_definition(source: &str, offset: usize) -> Option<Span> {
    let result = analyze(source);
    goto_definition_with_analysis(&result, offset)
}

/// Returns the span of the declaration that the symbol at `offset` refers to
/// using a previously computed [`AnalysisResult`].
pub fn goto_definition_with_analysis(result: &AnalysisResult, offset: usize) -> Option<Span> {
    let ast = result.ast.as_ref()?;

    for decl in &ast.declarations {
        match decl {
            Declaration::Model(model) => {
                for field in &model.fields {
                    if span_contains(field.span, offset) {
                        if let FieldType::UserType(ref name) = field.field_type {
                            return find_declaration_span(ast, name);
                        }
                    }
                }
            }
            Declaration::Type(type_decl) => {
                for field in &type_decl.fields {
                    if span_contains(field.span, offset) {
                        if let FieldType::UserType(ref name) = field.field_type {
                            return find_declaration_span(ast, name);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    None
}

/// Find the span of a model or enum declaration with the given name.
fn find_declaration_span(ast: &Schema, name: &str) -> Option<Span> {
    for decl in &ast.declarations {
        match decl {
            Declaration::Model(m) if m.name.value == name => return Some(m.span),
            Declaration::Enum(e) if e.name.value == name => return Some(e.span),
            Declaration::Type(t) if t.name.value == name => return Some(t.span),
            _ => {}
        }
    }
    None
}
