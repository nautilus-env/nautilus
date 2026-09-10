//! Abstract Syntax Tree (AST) for the nautilus schema language.
//!
//! This module defines the complete AST structure for representing parsed schemas.
//! All nodes include [`Span`] information for precise error diagnostics.
//!
//! Traverse the AST through [`Visitor::visit_schema`](crate::visitor::Visitor::visit_schema).
//!
//! # Example
//!
//! ```
//! let source = "model User { id Int @id email String @unique }";
//! let schema = nautilus_schema::parse_schema_source(source).unwrap();
//!
//! assert_eq!(schema.declarations.len(), 1);
//! ```

mod attributes;
mod config;
mod expr;
mod model;
mod types;

use crate::span::Span;

pub use attributes::{FieldAttribute, ModelAttribute, ReferentialAction};
pub use config::{ConfigField, DatasourceDecl, GeneratorDecl, ImportDecl};
pub use expr::{Expr, Ident, Literal};
pub use model::{EnumDecl, EnumVariant, FieldDecl, FieldModifier, ModelDecl, TypeDecl};
pub use types::{ComputedKind, FieldType, StorageStrategy};

/// Top-level schema document containing all declarations.
#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    /// All declarations in the schema (datasources, generators, models, enums).
    pub declarations: Vec<Declaration>,
    /// Span covering the entire schema.
    pub span: Span,
}

impl Schema {
    /// Creates a new schema with the given declarations.
    pub fn new(declarations: Vec<Declaration>, span: Span) -> Self {
        Self { declarations, span }
    }

    /// Finds every model-shaped declaration, `view` blocks included.
    ///
    /// Views share the model node and every rule that applies to a model's
    /// fields applies to them; callers that care about the difference filter on
    /// [`ModelDecl::is_view`].
    pub fn models(&self) -> impl Iterator<Item = &ModelDecl> {
        self.declarations.iter().filter_map(|d| match d {
            Declaration::Model(m) => Some(m),
            _ => None,
        })
    }

    /// Finds all `view` declarations in the schema.
    pub fn views(&self) -> impl Iterator<Item = &ModelDecl> {
        self.models().filter(|m| m.is_view)
    }

    /// Finds all enum declarations in the schema.
    pub fn enums(&self) -> impl Iterator<Item = &EnumDecl> {
        self.declarations.iter().filter_map(|d| match d {
            Declaration::Enum(e) => Some(e),
            _ => None,
        })
    }

    /// Finds all composite type declarations in the schema.
    pub fn types(&self) -> impl Iterator<Item = &TypeDecl> {
        self.declarations.iter().filter_map(|d| match d {
            Declaration::Type(t) => Some(t),
            _ => None,
        })
    }

    /// Finds all import statements in the schema.
    pub fn imports(&self) -> impl Iterator<Item = &ImportDecl> {
        self.declarations.iter().filter_map(|d| match d {
            Declaration::Import(i) => Some(i),
            _ => None,
        })
    }

    /// Finds the first datasource declaration.
    pub fn datasource(&self) -> Option<&DatasourceDecl> {
        self.declarations.iter().find_map(|d| match d {
            Declaration::Datasource(ds) => Some(ds),
            _ => None,
        })
    }

    /// Finds the first generator declaration.
    pub fn generator(&self) -> Option<&GeneratorDecl> {
        self.declarations.iter().find_map(|d| match d {
            Declaration::Generator(g) => Some(g),
            _ => None,
        })
    }
}

/// A top-level declaration in the schema.
#[derive(Debug, Clone, PartialEq)]
pub enum Declaration {
    /// An `import` statement.
    Import(ImportDecl),
    /// A datasource block.
    Datasource(DatasourceDecl),
    /// A generator block.
    Generator(GeneratorDecl),
    /// A model block.
    Model(ModelDecl),
    /// An enum block.
    Enum(EnumDecl),
    /// A composite type block.
    Type(TypeDecl),
}

impl Declaration {
    /// Returns the span of this declaration.
    pub fn span(&self) -> Span {
        match self {
            Declaration::Import(i) => i.span,
            Declaration::Datasource(d) => d.span,
            Declaration::Generator(g) => g.span,
            Declaration::Model(m) => m.span,
            Declaration::Enum(e) => e.span,
            Declaration::Type(t) => t.span,
        }
    }

    /// Returns the name of this declaration.
    pub fn name(&self) -> &str {
        match self {
            Declaration::Import(i) => &i.path,
            Declaration::Datasource(d) => &d.name.value,
            Declaration::Generator(g) => &g.name.value,
            Declaration::Model(m) => &m.name.value,
            Declaration::Enum(e) => &e.name.value,
            Declaration::Type(t) => &t.name.value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_modifier() {
        assert_eq!(FieldModifier::None, FieldModifier::None);
        assert_ne!(FieldModifier::Optional, FieldModifier::Array);
    }

    #[test]
    fn test_field_type_display() {
        assert_eq!(FieldType::String.to_string(), "String");
        assert_eq!(FieldType::Int.to_string(), "Int");
        assert_eq!(
            FieldType::Decimal {
                precision: 10,
                scale: 2
            }
            .to_string(),
            "Decimal(10, 2)"
        );
    }

    #[test]
    fn test_ident() {
        let ident = Ident::new("test".to_string(), Span::new(0, 4));
        assert_eq!(ident.value, "test");
        assert_eq!(ident.to_string(), "test");
    }

    #[test]
    fn test_referential_action_display() {
        assert_eq!(ReferentialAction::Cascade.to_string(), "Cascade");
        assert_eq!(ReferentialAction::SetNull.to_string(), "SetNull");
    }

    #[test]
    fn test_model_table_name() {
        let model = ModelDecl {
            name: Ident::new("User".to_string(), Span::new(0, 4)),
            fields: vec![],
            attributes: vec![ModelAttribute::Map("users".to_string())],
            is_view: false,
            span: Span::new(0, 10),
        };
        assert_eq!(model.table_name(), "users");
    }

    #[test]
    fn test_model_table_name_default() {
        let model = ModelDecl {
            name: Ident::new("User".to_string(), Span::new(0, 4)),
            fields: vec![],
            attributes: vec![],
            is_view: false,
            span: Span::new(0, 10),
        };
        assert_eq!(model.table_name(), "User");
    }

    #[test]
    fn test_field_column_name() {
        let field = FieldDecl {
            name: Ident::new("userId".to_string(), Span::new(0, 6)),
            field_type: FieldType::Int,
            modifier: FieldModifier::None,
            attributes: vec![FieldAttribute::Map("user_id".to_string())],
            span: Span::new(0, 20),
        };
        assert_eq!(field.column_name(), "user_id");
    }

    #[test]
    fn test_schema_helpers() {
        let schema = Schema {
            declarations: vec![
                Declaration::Model(ModelDecl {
                    name: Ident::new("User".to_string(), Span::new(0, 4)),
                    fields: vec![],
                    attributes: vec![],
                    is_view: false,
                    span: Span::new(0, 10),
                }),
                Declaration::Enum(EnumDecl {
                    name: Ident::new("Role".to_string(), Span::new(0, 4)),
                    variants: vec![],
                    span: Span::new(0, 10),
                }),
            ],
            span: Span::new(0, 100),
        };

        assert_eq!(schema.models().count(), 1);
        assert_eq!(schema.enums().count(), 1);
        assert!(schema.datasource().is_none());
    }
}
