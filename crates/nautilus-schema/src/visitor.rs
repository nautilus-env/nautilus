//! Visitor pattern for traversing the AST.
//!
//! This module provides a trait-based visitor pattern for flexible AST traversal.
//! Implement the [`Visitor`] trait to define custom operations on AST nodes.
//!
//! # Example
//!
//! ```ignore
//! use nautilus_schema::{visitor::Visitor, ast::*};
//!
//! struct ModelCounter {
//!     count: usize,
//! }
//!
//! impl Visitor for ModelCounter {
//!     fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
//!         self.count += 1;
//!         walk_model(self, model)
//!     }
//! }
//! ```

use crate::ast::*;
use crate::error::Result;

/// Visitor trait for traversing AST nodes.
///
/// All methods have default implementations that continue traversing child nodes.
/// Override specific methods to implement custom behavior.
pub trait Visitor: Sized {
    /// Visit the entire schema.
    fn visit_schema(&mut self, schema: &Schema) -> Result<()> {
        walk_schema(self, schema)
    }

    /// Visit a top-level declaration.
    fn visit_declaration(&mut self, decl: &Declaration) -> Result<()> {
        walk_declaration(self, decl)
    }

    /// Visit a datasource declaration.
    fn visit_datasource(&mut self, datasource: &DatasourceDecl) -> Result<()> {
        walk_datasource(self, datasource)
    }

    /// Visit a generator declaration.
    fn visit_generator(&mut self, generator: &GeneratorDecl) -> Result<()> {
        walk_generator(self, generator)
    }

    /// Visit a model declaration.
    fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
        walk_model(self, model)
    }

    /// Visit an enum declaration.
    fn visit_enum(&mut self, enum_decl: &EnumDecl) -> Result<()> {
        walk_enum(self, enum_decl)
    }

    /// Visit a field declaration.
    fn visit_field(&mut self, field: &FieldDecl) -> Result<()> {
        walk_field(self, field)
    }

    /// Visit an enum variant.
    fn visit_enum_variant(&mut self, variant: &EnumVariant) -> Result<()> {
        walk_enum_variant(self, variant)
    }

    /// Visit a field attribute.
    fn visit_field_attribute(&mut self, attr: &FieldAttribute) -> Result<()> {
        walk_field_attribute(self, attr)
    }

    /// Visit a model attribute.
    fn visit_model_attribute(&mut self, attr: &ModelAttribute) -> Result<()> {
        walk_model_attribute(self, attr)
    }

    /// Visit an expression.
    fn visit_expr(&mut self, expr: &Expr) -> Result<()> {
        walk_expr(self, expr)
    }

    /// Visit a configuration field.
    fn visit_config_field(&mut self, field: &ConfigField) -> Result<()> {
        walk_config_field(self, field)
    }

    /// Visit a composite type declaration.
    fn visit_type_decl(&mut self, type_decl: &TypeDecl) -> Result<()> {
        walk_type_decl(self, type_decl)
    }
}

/// Walk through a schema, visiting all declarations.
pub fn walk_schema<V: Visitor>(visitor: &mut V, schema: &Schema) -> Result<()> {
    for decl in &schema.declarations {
        visitor.visit_declaration(decl)?;
    }
    Ok(())
}

/// Walk through a declaration.
pub fn walk_declaration<V: Visitor>(visitor: &mut V, decl: &Declaration) -> Result<()> {
    match decl {
        Declaration::Datasource(ds) => visitor.visit_datasource(ds),
        Declaration::Generator(gen) => visitor.visit_generator(gen),
        Declaration::Model(model) => visitor.visit_model(model),
        Declaration::Enum(enum_decl) => visitor.visit_enum(enum_decl),
        Declaration::Type(type_decl) => visitor.visit_type_decl(type_decl),
    }
}

/// Walk through a datasource declaration.
pub fn walk_datasource<V: Visitor>(visitor: &mut V, datasource: &DatasourceDecl) -> Result<()> {
    for field in &datasource.fields {
        visitor.visit_config_field(field)?;
    }
    Ok(())
}

/// Walk through a generator declaration.
pub fn walk_generator<V: Visitor>(visitor: &mut V, generator: &GeneratorDecl) -> Result<()> {
    for field in &generator.fields {
        visitor.visit_config_field(field)?;
    }
    Ok(())
}

/// Walk through a model declaration.
pub fn walk_model<V: Visitor>(visitor: &mut V, model: &ModelDecl) -> Result<()> {
    for field in &model.fields {
        visitor.visit_field(field)?;
    }
    for attr in &model.attributes {
        visitor.visit_model_attribute(attr)?;
    }
    Ok(())
}

/// Walk through an enum declaration.
pub fn walk_enum<V: Visitor>(visitor: &mut V, enum_decl: &EnumDecl) -> Result<()> {
    for variant in &enum_decl.variants {
        visitor.visit_enum_variant(variant)?;
    }
    Ok(())
}

/// Walk through a field declaration.
pub fn walk_field<V: Visitor>(visitor: &mut V, field: &FieldDecl) -> Result<()> {
    for attr in &field.attributes {
        visitor.visit_field_attribute(attr)?;
    }
    Ok(())
}

/// Walk through an enum variant.
pub fn walk_enum_variant<V: Visitor>(_visitor: &mut V, _variant: &EnumVariant) -> Result<()> {
    Ok(())
}

/// Walk through a field attribute.
pub fn walk_field_attribute<V: Visitor>(visitor: &mut V, attr: &FieldAttribute) -> Result<()> {
    match attr {
        FieldAttribute::Default(expr, _) => visitor.visit_expr(expr),
        FieldAttribute::Relation { .. } => {
            // Relation fields are identifiers, not expressions
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Walk through a model attribute.
pub fn walk_model_attribute<V: Visitor>(_visitor: &mut V, _attr: &ModelAttribute) -> Result<()> {
    Ok(())
}

/// Walk through an expression.
pub fn walk_expr<V: Visitor>(visitor: &mut V, expr: &Expr) -> Result<()> {
    match expr {
        Expr::FunctionCall { args, .. } => {
            for arg in args {
                visitor.visit_expr(arg)?;
            }
            Ok(())
        }
        Expr::Array { elements, .. } => {
            for elem in elements {
                visitor.visit_expr(elem)?;
            }
            Ok(())
        }
        Expr::NamedArg { value, .. } => visitor.visit_expr(value),
        Expr::Literal(_) | Expr::Ident(_) => Ok(()),
    }
}

/// Walk through a configuration field.
pub fn walk_config_field<V: Visitor>(visitor: &mut V, field: &ConfigField) -> Result<()> {
    visitor.visit_expr(&field.value)
}

/// Walk through a composite type declaration.
pub fn walk_type_decl<V: Visitor>(visitor: &mut V, type_decl: &TypeDecl) -> Result<()> {
    for field in &type_decl.fields {
        visitor.visit_field(field)?;
    }
    Ok(())
}

/// Example visitor that counts nodes in the AST.
#[derive(Debug, Default)]
pub struct CountingVisitor {
    /// Number of models.
    pub models: usize,
    /// Number of enums.
    pub enums: usize,
    /// Number of fields.
    pub fields: usize,
    /// Number of datasources.
    pub datasources: usize,
    /// Number of generators.
    pub generators: usize,
}

impl Visitor for CountingVisitor {
    fn visit_datasource(&mut self, datasource: &DatasourceDecl) -> Result<()> {
        self.datasources += 1;
        walk_datasource(self, datasource)
    }

    fn visit_generator(&mut self, generator: &GeneratorDecl) -> Result<()> {
        self.generators += 1;
        walk_generator(self, generator)
    }

    fn visit_model(&mut self, model: &ModelDecl) -> Result<()> {
        self.models += 1;
        walk_model(self, model)
    }

    fn visit_enum(&mut self, enum_decl: &EnumDecl) -> Result<()> {
        self.enums += 1;
        walk_enum(self, enum_decl)
    }

    fn visit_field(&mut self, field: &FieldDecl) -> Result<()> {
        self.fields += 1;
        walk_field(self, field)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    #[test]
    fn test_walk_expression() {
        let expr = Expr::FunctionCall {
            name: Ident::new("now".to_string(), Span::new(0, 3)),
            args: vec![],
            span: Span::new(0, 5),
        };

        let mut visitor = CountingVisitor::default();
        visitor.visit_expr(&expr).unwrap();
    }

    #[test]
    fn test_walk_nested_expr() {
        let expr = Expr::Array {
            elements: vec![
                Expr::Literal(Literal::String("test".to_string(), Span::new(0, 4))),
                Expr::FunctionCall {
                    name: Ident::new("env".to_string(), Span::new(0, 3)),
                    args: vec![Expr::Literal(Literal::String(
                        "VAR".to_string(),
                        Span::new(0, 3),
                    ))],
                    span: Span::new(0, 10),
                },
            ],
            span: Span::new(0, 20),
        };

        let mut visitor = CountingVisitor::default();
        visitor.visit_expr(&expr).unwrap();
    }
}
