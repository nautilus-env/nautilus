//! The attributes written on a field or on a declaration, including the
//! relation they define and the referential actions it takes.

use std::fmt;

use crate::span::Span;

use super::{ComputedKind, Expr, Ident, StorageStrategy};

/// A field-level attribute (@id, @unique, etc.).
#[derive(Debug, Clone, PartialEq)]
pub enum FieldAttribute {
    /// @id attribute.
    Id,
    /// @unique attribute.
    Unique,
    /// @default(value) attribute.
    /// The `Span` covers the full `@default(...)` token range.
    Default(Expr, Span),
    /// @map("name") attribute.
    Map(String),
    /// @store(json) attribute for array storage strategy.
    Store {
        /// Storage strategy (currently only "json" supported).
        strategy: StorageStrategy,
        /// Span of the entire attribute.
        span: Span,
    },
    /// @relation(...) attribute.
    Relation {
        /// name: "relationName" (optional, required for multiple relations)
        name: Option<String>,
        /// fields: [field1, field2]
        fields: Option<Vec<Ident>>,
        /// references: [field1, field2]
        references: Option<Vec<Ident>>,
        /// onDelete: Cascade | SetNull | ...
        on_delete: Option<ReferentialAction>,
        /// onUpdate: Cascade | SetNull | ...
        on_update: Option<ReferentialAction>,
        /// Span of the entire attribute.
        span: Span,
    },
    /// @updatedAt — auto-set to current timestamp on every write.
    UpdatedAt {
        /// Span covering `@updatedAt`.
        span: Span,
    },
    /// @computed(expr, Stored | Virtual) — database-generated column.
    Computed {
        /// Parsed SQL expression (e.g. `price * quantity`).
        expr: crate::sql_expr::SqlExpr,
        /// Whether the value is stored on disk or computed on every read.
        kind: ComputedKind,
        /// Span of the entire `@computed(...)` attribute.
        span: Span,
    },
    /// @check(bool_expr) — column-level CHECK constraint.
    Check {
        /// Parsed boolean expression (e.g. `age >= 0 AND age <= 150`).
        expr: crate::bool_expr::BoolExpr,
        /// Span of the entire `@check(...)` attribute.
        span: Span,
    },
    /// @ignore — the column exists in the database but Nautilus does not
    /// manage it: it is left out of the generated client and of every
    /// migration.
    Ignore {
        /// Span covering `@ignore`.
        span: Span,
    },
}

/// Referential actions for foreign key constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferentialAction {
    /// CASCADE action.
    Cascade,
    /// RESTRICT action.
    Restrict,
    /// NO ACTION.
    NoAction,
    /// SET NULL.
    SetNull,
    /// SET DEFAULT.
    SetDefault,
}

impl fmt::Display for ReferentialAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReferentialAction::Cascade => write!(f, "Cascade"),
            ReferentialAction::Restrict => write!(f, "Restrict"),
            ReferentialAction::NoAction => write!(f, "NoAction"),
            ReferentialAction::SetNull => write!(f, "SetNull"),
            ReferentialAction::SetDefault => write!(f, "SetDefault"),
        }
    }
}

/// A model-level attribute (@@map, @@id, etc.).
#[derive(Debug, Clone, PartialEq)]
pub enum ModelAttribute {
    /// @@map("table_name") attribute.
    Map(String),
    /// @@id([field1, field2]) composite primary key.
    Id(Vec<Ident>),
    /// @@unique([field1, field2]) composite unique constraint.
    Unique(Vec<Ident>),
    /// @@index([field1, field2], type: Hash, opclass: vector_l2_ops, m: 16, ef_construction: 64, name: "idx_name", map: "db_idx", where: active = true) index.
    Index {
        /// Fields that form the index key.
        fields: Vec<Ident>,
        /// Optional index type (`type:` argument). `None` -> let the DBMS choose.
        index_type: Option<Ident>,
        /// Optional pgvector operator class (`opclass:` argument).
        opclass: Option<Ident>,
        /// Optional pgvector HNSW parameter (`m:`).
        m: Option<u32>,
        /// Optional pgvector HNSW parameter (`ef_construction:`).
        ef_construction: Option<u32>,
        /// Optional pgvector IVFFlat parameter (`lists:`).
        lists: Option<u32>,
        /// Optional logical name (`name:` argument).
        name: Option<String>,
        /// Optional physical DB name (`map:` argument).
        map: Option<String>,
        /// Optional partial-index predicate (`where:` argument). When set, the
        /// index only covers the rows for which the predicate holds.
        predicate: Option<crate::bool_expr::BoolExpr>,
        /// Span of the entire `@@index(...)` attribute.
        span: Span,
    },
    /// @@check(bool_expr) — table-level CHECK constraint.
    Check {
        /// Parsed boolean expression (e.g. `start_date < end_date`).
        expr: crate::bool_expr::BoolExpr,
        /// Span of the entire `@@check(...)` attribute.
        span: Span,
    },
    /// @@ignore — the table exists in the database but Nautilus does not
    /// manage it: it is left out of the generated client and of every
    /// migration.
    Ignore {
        /// Span covering `@@ignore`.
        span: Span,
    },
    /// @@schema("analytics") — the PostgreSQL schema that owns the table.
    Schema {
        /// Schema name, which must appear in the datasource's `schemas` list.
        name: String,
        /// Span covering `@@schema(...)`.
        span: Span,
    },
}
