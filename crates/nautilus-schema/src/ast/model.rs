//! The declarations that describe data: a `model` or `view`, the composite
//! `type` blocks and the `enum`s, down to the fields they hold.

use crate::span::Span;

use super::{FieldAttribute, FieldType, Ident, ModelAttribute};

/// A model or `view` block declaration.
///
/// A view is parsed into the same node as a model and differs only by
/// [`is_view`](Self::is_view): it names a read-only relation that Nautilus
/// queries but never creates, alters, drops or writes to.
///
/// # Example
///
/// ```prisma
/// model User {
///   id    Int    @id @default(autoincrement())
///   email String @unique
///   @@map("users")
/// }
///
/// view ActiveUser {
///   id    Int    @id
///   email String
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDecl {
    /// The model name (e.g., "User").
    pub name: Ident,
    /// Field declarations.
    pub fields: Vec<FieldDecl>,
    /// Model-level attributes (@@map, @@id, etc.).
    pub attributes: Vec<ModelAttribute>,
    /// `true` when the block was declared with the `view` keyword.
    pub is_view: bool,
    /// Span covering the entire model block.
    pub span: Span,
}

impl ModelDecl {
    /// Finds a field by name.
    pub fn find_field(&self, name: &str) -> Option<&FieldDecl> {
        self.fields.iter().find(|f| f.name.value == name)
    }

    /// Gets the physical table name from @@map attribute, or the model name.
    pub fn table_name(&self) -> &str {
        self.attributes
            .iter()
            .find_map(|attr| match attr {
                ModelAttribute::Map(name) => Some(name.as_str()),
                _ => None,
            })
            .unwrap_or(&self.name.value)
    }

    /// The keyword this block was declared with, for diagnostics that quote it
    /// back to the user.
    pub fn keyword(&self) -> &'static str {
        if self.is_view {
            "view"
        } else {
            "model"
        }
    }

    /// The PostgreSQL schema declared with `@@schema("...")`, if any.
    pub fn schema_name(&self) -> Option<&str> {
        self.attributes.iter().find_map(|attr| match attr {
            ModelAttribute::Schema { name, .. } => Some(name.as_str()),
            _ => None,
        })
    }

    /// Checks if this model carries `@@ignore`.
    pub fn is_ignored(&self) -> bool {
        self.attributes
            .iter()
            .any(|attr| matches!(attr, ModelAttribute::Ignore { .. }))
    }

    /// Checks if this model has a composite primary key (@@id).
    pub fn has_composite_key(&self) -> bool {
        self.attributes
            .iter()
            .any(|attr| matches!(attr, ModelAttribute::Id(_)))
    }

    /// Returns all fields that are part of relations.
    /// This includes fields with user-defined types (model/enum references).
    pub fn relation_fields(&self) -> impl Iterator<Item = &FieldDecl> {
        self.fields.iter().filter(|f| {
            f.has_relation_attribute() || matches!(f.field_type, FieldType::UserType(_))
        })
    }
}

/// A field declaration within a model.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    /// The field name.
    pub name: Ident,
    /// The field type.
    pub field_type: FieldType,
    /// Optional or array modifier.
    pub modifier: FieldModifier,
    /// Field-level attributes (@id, @unique, etc.).
    pub attributes: Vec<FieldAttribute>,
    /// Span covering the entire field declaration.
    pub span: Span,
}

impl FieldDecl {
    /// Checks if this field is optional (has `?` modifier).
    pub fn is_optional(&self) -> bool {
        matches!(self.modifier, FieldModifier::Optional)
    }

    /// Checks if this field has an explicit not-null modifier (`!`).
    pub fn is_not_null(&self) -> bool {
        matches!(self.modifier, FieldModifier::NotNull)
    }

    /// Checks if this field is an array (has `[]` modifier).
    pub fn is_array(&self) -> bool {
        matches!(self.modifier, FieldModifier::Array)
    }

    /// Finds a field attribute by kind.
    pub fn find_attribute(&self, kind: &str) -> Option<&FieldAttribute> {
        self.attributes.iter().find(|attr| {
            matches!(
                (kind, attr),
                ("id", FieldAttribute::Id)
                    | ("unique", FieldAttribute::Unique)
                    | ("default", FieldAttribute::Default(_, _))
                    | ("map", FieldAttribute::Map(_))
                    | ("relation", FieldAttribute::Relation { .. })
                    | ("check", FieldAttribute::Check { .. })
            )
        })
    }

    /// Checks if this field carries `@ignore`.
    pub fn is_ignored(&self) -> bool {
        self.attributes
            .iter()
            .any(|attr| matches!(attr, FieldAttribute::Ignore { .. }))
    }

    /// Checks if this field has a @relation attribute.
    pub fn has_relation_attribute(&self) -> bool {
        self.attributes
            .iter()
            .any(|attr| matches!(attr, FieldAttribute::Relation { .. }))
    }

    /// Gets the physical column name from @map attribute, or the field name.
    pub fn column_name(&self) -> &str {
        self.attributes
            .iter()
            .find_map(|attr| match attr {
                FieldAttribute::Map(name) => Some(name.as_str()),
                _ => None,
            })
            .unwrap_or(&self.name.value)
    }
}

/// Field type modifiers (optional, not-null, or array).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldModifier {
    /// No modifier (required field).
    None,
    /// Optional field (`?`).
    Optional,
    /// Explicit not-null field (`!`).
    NotNull,
    /// Array field (`[]`).
    Array,
}

/// An enum block declaration.
///
/// # Example
///
/// ```prisma
/// enum Role {
///   USER
///   ADMIN
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    /// The enum name (e.g., "Role").
    pub name: Ident,
    /// Enum variants.
    pub variants: Vec<EnumVariant>,
    /// Span covering the entire enum block.
    pub span: Span,
}

/// A composite type block declaration.
///
/// Composite types define named struct-like types that can be embedded in models.
/// On PostgreSQL they map to native composite types; on MySQL/SQLite they are
/// serialised to JSON (`@store(Json)` is required on the model field).
///
/// # Example
///
/// ```prisma
/// type Address {
///   street String
///   city   String
///   zip    String
///   @@map("address_t")
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    /// The type name (e.g., "Address").
    pub name: Ident,
    /// Field declarations (scalars, enums, and arrays — no relations).
    pub fields: Vec<FieldDecl>,
    /// Type-level attributes. Only `@@map` is supported on composite types.
    pub attributes: Vec<ModelAttribute>,
    /// Span covering the entire type block.
    pub span: Span,
}

impl TypeDecl {
    /// Finds a field by name.
    pub fn find_field(&self, name: &str) -> Option<&FieldDecl> {
        self.fields.iter().find(|f| f.name.value == name)
    }

    /// Gets the explicit physical type name from a `@@map` attribute, if any.
    pub fn mapped_name(&self) -> Option<&str> {
        self.attributes.iter().find_map(|attr| match attr {
            ModelAttribute::Map(name) => Some(name.as_str()),
            _ => None,
        })
    }

    /// Returns the physical SQL type name: the `@@map` value when present,
    /// otherwise the lowercased logical name (PostgreSQL folds unquoted
    /// identifiers to lower case, and the inspector reports `typname` in lower
    /// case, so this keeps the schema/live-DB round-trip stable).
    pub fn db_type_name(&self) -> String {
        self.mapped_name()
            .map(str::to_string)
            .unwrap_or_else(|| self.name.value.to_lowercase())
    }
}

/// An enum variant.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    /// The variant name.
    pub name: Ident,
    /// Span covering the variant.
    pub span: Span,
}
