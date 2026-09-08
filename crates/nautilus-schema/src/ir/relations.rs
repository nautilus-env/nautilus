//! A relation between two models, and the join table an implicit
//! many-to-many is stored in.

use crate::ast::ReferentialAction;

/// Validated relation metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationIr {
    /// Optional relation name (required for multiple relations between same models).
    pub name: Option<String>,
    /// The logical name of the target model.
    pub target_model: String,
    /// Foreign key field names in the current model (logical names).
    pub fields: Vec<String>,
    /// Referenced field names in the target model (logical names).
    pub references: Vec<String>,
    /// Referential action on delete.
    pub on_delete: Option<ReferentialAction>,
    /// Referential action on update.
    pub on_update: Option<ReferentialAction>,
    /// The join table, when this is one side of an implicit many-to-many.
    ///
    /// `fields` and `references` are empty on such a relation because neither
    /// model carries a foreign key: the links live in the table named here.
    pub join: Option<ManyToManyJoinIr>,
}

/// The join table carrying an implicit many-to-many relation.
///
/// A relation declared as an array on both sides has nowhere to put a foreign
/// key, so Nautilus owns a table of links for it. That table is synthesised
/// into the schema — it is created and dropped by migrations like any other,
/// but it is not a model the user declared, so it never reaches a generated
/// client and is only ever read or written through the two array fields.
///
/// The two columns are named `A` and `B` after the convention every ORM with
/// this feature uses, `A` belonging to whichever side sorts first by
/// `(model, field)`. Sorting rather than declaration order is what makes the
/// table name and column roles the same whichever file the reader is looking
/// at, and it is the only naming that survives a self-relation, where both
/// ends name the same model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManyToManyJoinIr {
    /// Physical name of the join table (`_PostToTag`).
    pub table: String,
    /// Join-table column holding the key of the model that declares this field.
    pub self_column: String,
    /// Join-table column holding the key of the target model.
    pub target_column: String,
    /// Logical field on the declaring model that [`self_column`](Self::self_column) points at.
    pub self_reference: String,
    /// Logical field on the target model that [`target_column`](Self::target_column) points at.
    pub target_reference: String,
}
