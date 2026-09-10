//! What a validated declaration holds: a model with its fields, its key and
//! its constraints, an enum, and a composite type.

use crate::ast::{ComputedKind, StorageStrategy};
use crate::span::Span;

use super::index::IndexIr;
use super::{DefaultValue, ResolvedFieldType, ScalarType};

/// Validated model with fully resolved fields and metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelIr {
    /// The logical name as defined in the schema (e.g., "User").
    pub logical_name: String,
    /// The physical database table name (from @@map or logical_name).
    pub db_name: String,
    /// The PostgreSQL schema that owns the table, from `@@schema("...")`.
    ///
    /// `None` in single-schema mode, where the table name is rendered
    /// unqualified and resolves through the connection's `search_path`.
    pub schema: Option<String>,
    /// All fields in the model.
    pub fields: Vec<FieldIr>,
    /// Primary key metadata.
    pub primary_key: PrimaryKeyIr,
    /// Unique constraints (from @unique and @@unique).
    pub unique_constraints: Vec<UniqueConstraintIr>,
    /// Indexes (from @@index).
    pub indexes: Vec<IndexIr>,
    /// Table-level CHECK constraint expressions (SQL strings).
    pub check_constraints: Vec<String>,
    /// Whether the model carries `@@ignore` — the table exists but Nautilus
    /// does not manage it. See [`SchemaIr::without_ignored`](super::SchemaIr::without_ignored).
    pub is_ignored: bool,
    /// Whether this block was declared as a `view`.
    ///
    /// A view is read-only: Nautilus queries it like a table but never emits
    /// DDL for it and rejects every write method against it.
    pub is_view: bool,
    /// Whether Nautilus synthesised this model as the join table of an
    /// implicit many-to-many relation. See [`ManyToManyJoinIr`](super::ManyToManyJoinIr).
    pub is_join_table: bool,
    /// Span of the model declaration.
    pub span: Span,
}

impl ModelIr {
    /// Finds a field by logical name.
    pub fn find_field(&self, name: &str) -> Option<&FieldIr> {
        self.fields.iter().find(|f| f.logical_name == name)
    }

    /// Returns an iterator over scalar fields (non-relations).
    pub fn scalar_fields(&self) -> impl Iterator<Item = &FieldIr> {
        self.fields
            .iter()
            .filter(|f| !matches!(f.field_type, ResolvedFieldType::Relation(_)))
    }

    /// Returns an iterator over relation fields.
    pub fn relation_fields(&self) -> impl Iterator<Item = &FieldIr> {
        self.fields
            .iter()
            .filter(|f| matches!(f.field_type, ResolvedFieldType::Relation(_)))
    }

    /// Returns `true` when at least one field on this model is a pgvector
    /// `Vector(...)` column.
    pub fn has_vector_fields(&self) -> bool {
        self.fields.iter().any(FieldIr::is_vector)
    }

    /// Returns the logical names of every `Vector(...)` field on this model.
    pub fn vector_field_names(&self) -> impl Iterator<Item = &str> {
        self.fields
            .iter()
            .filter(|f| f.is_vector())
            .map(|f| f.logical_name.as_str())
    }
}

/// Validated field with resolved type.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldIr {
    /// The logical field name as defined in the schema (e.g., "userId").
    pub logical_name: String,
    /// The physical database column name (from @map or logical_name).
    pub db_name: String,
    /// The resolved field type (scalar, enum, or relation).
    pub field_type: ResolvedFieldType,
    /// Whether the field is required (not optional and not array).
    pub is_required: bool,
    /// Whether the field is an array.
    pub is_array: bool,
    /// Storage strategy for array fields (None for non-arrays or native support).
    pub storage_strategy: Option<StorageStrategy>,
    /// Default value (if specified via @default).
    pub default_value: Option<DefaultValue>,
    /// Whether the field has @unique.
    pub is_unique: bool,
    /// Whether the field has @updatedAt — auto-set to now() on every write.
    pub is_updated_at: bool,
    /// Computed column expression and kind — `None` for regular fields.
    pub computed: Option<(String, ComputedKind)>,
    /// Column-level CHECK constraint expression (SQL string). `None` for unconstrained fields.
    pub check: Option<String>,
    /// Whether the field carries `@ignore` — the column exists but Nautilus
    /// does not manage it. See [`SchemaIr::without_ignored`](super::SchemaIr::without_ignored).
    pub is_ignored: bool,
    /// Span of the field declaration.
    pub span: Span,
}

impl FieldIr {
    /// Returns `true` when this field's resolved type is a pgvector
    /// `Vector(dim)` scalar.
    pub fn is_vector(&self) -> bool {
        matches!(
            self.field_type,
            ResolvedFieldType::Scalar(ScalarType::Vector { .. })
        )
    }
}

/// Primary key metadata.
#[derive(Debug, Clone, PartialEq)]
pub enum PrimaryKeyIr {
    /// Single-field primary key (from @id).
    Single(String),
    /// Composite primary key (from @@id).
    Composite(Vec<String>),
}

impl PrimaryKeyIr {
    /// Returns the field names that form the primary key.
    pub fn fields(&self) -> Vec<&str> {
        match self {
            PrimaryKeyIr::Single(field) => vec![field.as_str()],
            PrimaryKeyIr::Composite(fields) => fields.iter().map(|s| s.as_str()).collect(),
        }
    }

    /// Returns true if this is a single-field primary key.
    pub fn is_single(&self) -> bool {
        matches!(self, PrimaryKeyIr::Single(_))
    }

    /// Returns true if this is a composite primary key.
    pub fn is_composite(&self) -> bool {
        matches!(self, PrimaryKeyIr::Composite(_))
    }
}

/// Unique constraint metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct UniqueConstraintIr {
    /// Field names (logical) that form the unique constraint.
    pub fields: Vec<String>,
}

/// Validated enum type.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumIr {
    /// The logical enum name (e.g., "Role").
    pub logical_name: String,
    /// Enum variant names.
    pub variants: Vec<String>,
    /// Span of the enum declaration.
    pub span: Span,
}

impl EnumIr {
    /// Checks if a variant exists.
    pub fn has_variant(&self, name: &str) -> bool {
        self.variants.iter().any(|v| v == name)
    }
}

/// A single field within a composite type.
///
/// Only scalar and enum field types are allowed — no relations or nested composite types.
#[derive(Debug, Clone, PartialEq)]
pub struct CompositeFieldIr {
    /// The logical field name as defined in the type block.
    pub logical_name: String,
    /// The physical name (from @map or logical_name).
    pub db_name: String,
    /// The resolved field type (Scalar or Enum only).
    pub field_type: ResolvedFieldType,
    /// Whether the field is required (not optional).
    pub is_required: bool,
    /// Whether the field is an array.
    pub is_array: bool,
    /// Storage strategy for array fields.
    pub storage_strategy: Option<StorageStrategy>,
    /// Span of the field declaration.
    pub span: Span,
}

/// Validated composite type (embedded struct).
#[derive(Debug, Clone, PartialEq)]
pub struct CompositeTypeIr {
    /// The logical type name as defined in the schema (e.g., "Address").
    pub logical_name: String,
    /// The physical SQL type name (`@@map` value or lowercased logical name).
    pub db_name: String,
    /// All fields of the composite type.
    pub fields: Vec<CompositeFieldIr>,
    /// Span of the type declaration.
    pub span: Span,
}
