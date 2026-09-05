//! The individual schema changes a diff produces, their risk classification,
//! and how they are described to the user.

use nautilus_schema::ir::{FieldIr, IndexKind, ModelIr};

use nautilus_core::TableName;

/// A single schema change between the live database and the target schema.
#[derive(Debug, Clone)]
pub enum Change {
    /// A model exists in the target schema but has no corresponding table in
    /// the live database.
    NewTable(ModelIr),

    /// A table exists in the live database but has no corresponding model in
    /// the target schema.
    DroppedTable {
        /// DB table name.
        name: TableName,
    },

    /// A scalar field exists in the target model but the corresponding column
    /// is missing from the live table.
    AddedColumn {
        /// DB table name.
        table: TableName,
        /// Target field IR (contains `db_name`, type info, etc.).
        field: FieldIr,
    },

    /// A column exists in the live table but has no corresponding scalar field
    /// in the target model.
    DroppedColumn {
        /// DB table name.
        table: TableName,
        /// DB column name.
        column: String,
    },

    /// The SQL type of an existing column does not match the target field type.
    TypeChanged {
        /// DB table name.
        table: TableName,
        /// DB column name.
        column: String,
        /// Current live SQL type (normalised, lower-cased).
        from: String,
        /// Target SQL type (from schema, normalised, lower-cased).
        to: String,
    },

    /// The nullability of an existing column differs from the target field.
    NullabilityChanged {
        /// DB table name.
        table: TableName,
        /// DB column name.
        column: String,
        /// `true` when the target requires `NOT NULL` (column is becoming
        /// required); `false` when the target allows `NULL`.
        now_required: bool,
    },

    /// The column's database-generated identity differs from the target.
    ///
    /// MySQL only: `AUTO_INCREMENT` is a column attribute there, so a table
    /// created before the generator emitted it keeps a plain integer key that
    /// rejects every insert without an explicit id.
    AutoIncrementChanged {
        /// DB table name.
        table: TableName,
        /// DB column name.
        column: String,
        /// `true` when the target wants the database to generate the values.
        enabled: bool,
    },

    /// The DEFAULT expression of an existing column differs from the target.
    DefaultChanged {
        /// DB table name.
        table: TableName,
        /// DB column name.
        column: String,
        /// Current live default (lower-cased), or `None`.
        from: Option<String>,
        /// Target default (lower-cased), or `None`.
        to: Option<String>,
    },

    /// The set of primary-key columns has changed.
    PrimaryKeyChanged {
        /// DB table name.
        table: TableName,
    },

    /// A new index (defined in the target schema) is not present in the live
    /// database.
    IndexAdded {
        /// DB table name.
        table: TableName,
        /// Sorted DB column names that form the index key.
        columns: Vec<String>,
        /// Whether the index enforces uniqueness.
        unique: bool,
        /// Access method + extension payload (BTree, pgvector HNSW, ...).
        kind: IndexKind,
        /// Optional DDL name override (from `@@index(map: "...")` or `name:`).
        index_name: Option<String>,
        /// Partial-index predicate (from `@@index(where: ...)`), already
        /// rendered against physical column names.
        predicate: Option<String>,
    },

    /// An index that exists in the live database is not present in the target
    /// schema.
    IndexDropped {
        /// DB table name.
        table: TableName,
        /// Sorted DB column names that form the index key.
        columns: Vec<String>,
        /// Whether the index enforces uniqueness.
        unique: bool,
        /// Physical name of the index as it exists in the database.
        index_name: String,
    },

    /// The generation expression of a computed column has changed.
    ComputedExprChanged {
        /// DB table name.
        table: TableName,
        /// DB column name.
        column: String,
        /// Target field IR (needed to regenerate the column definition).
        field: FieldIr,
    },

    /// A CHECK constraint was added, removed, or changed.
    CheckChanged {
        /// DB table name.
        table: TableName,
        /// DB column name (`None` for table-level checks).
        column: Option<String>,
        /// Old expression, or `None` if being added.
        from: Option<String>,
        /// New expression, or `None` if being dropped.
        to: Option<String>,
    },

    /// A PostgreSQL composite type exists in the target schema but not in the live
    /// database.
    CreateCompositeType {
        /// DB type name (lower-cased).
        name: String,
    },

    /// A PostgreSQL composite type exists in the live database but has been removed
    /// from the target schema.
    DropCompositeType {
        /// DB type name (lower-cased).
        name: String,
    },

    /// A PostgreSQL composite type exists in both live and target but its field
    /// list has changed.
    AlterCompositeType {
        /// DB type name (lower-cased).
        name: String,
        /// Fields present in target but not in live: `(db_name, sql_type)`.
        added_fields: Vec<(String, String)>,
        /// Field DB names present in live but not in target.
        dropped_fields: Vec<String>,
        /// Fields whose SQL type changed: `(db_name, from_type, to_type)`.
        type_changed_fields: Vec<(String, String, String)>,
    },

    /// A PostgreSQL enum type exists in the target schema but not in the live
    /// database.
    CreateEnum {
        /// DB type name (lower-cased).
        name: String,
        /// Ordered variant labels.
        variants: Vec<String>,
    },

    /// A PostgreSQL enum type exists in the live database but has been removed
    /// from the target schema.
    DropEnum {
        /// DB type name (lower-cased).
        name: String,
    },

    /// A PostgreSQL schema is declared in the target datasource but does not
    /// exist in the live database.
    ///
    /// Nautilus creates schemas but never drops them: a schema can hold objects
    /// Nautilus does not manage, and dropping it would take them with it.
    CreateSchema {
        /// Schema name, as written in the datasource `schemas` list.
        name: String,
    },

    /// A PostgreSQL extension is declared in the target datasource but is not
    /// currently installed in the live database.
    CreateExtension {
        /// Extension name (lower-cased, as it appears in `pg_extension.extname`).
        name: String,
        /// Optional schema qualifier. When `Some`, the emitted DDL will include
        /// `WITH SCHEMA "<schema>"`.
        schema: Option<String>,
    },

    /// A PostgreSQL extension is installed in the live database but is no
    /// longer declared in the target datasource.
    DropExtension {
        /// Extension name (lower-cased).
        name: String,
    },

    /// A PostgreSQL enum type exists in both live and target but its variant
    /// list has changed.
    AlterEnum {
        /// DB type name (lower-cased).
        name: String,
        /// Variants present in target but not in live (to be added).
        added_variants: Vec<String>,
        /// Variants present in live but not in target (to be removed).
        removed_variants: Vec<String>,
    },

    /// A foreign-key constraint is present in the target schema but absent from
    /// the live database (or its referential actions have changed and the old
    /// constraint was already emitted as `ForeignKeyDropped`).
    ForeignKeyAdded {
        /// DB table name.
        table: TableName,
        /// Constraint name to create (auto-derived from table + columns).
        constraint_name: String,
        /// Local FK column names, in declaration order.
        columns: Vec<String>,
        /// Referenced table name.
        referenced_table: TableName,
        /// Referenced column names, in declaration order.
        referenced_columns: Vec<String>,
        /// ON DELETE action, or `None` for the database default (NO ACTION).
        on_delete: Option<String>,
        /// ON UPDATE action, or `None` for the database default (NO ACTION).
        on_update: Option<String>,
    },

    /// A foreign-key constraint exists in the live database but has no
    /// corresponding relation field in the target schema (or its referential
    /// actions changed and a replacement `ForeignKeyAdded` follows).
    ForeignKeyDropped {
        /// DB table name.
        table: TableName,
        /// Live constraint name to drop.
        constraint_name: String,
    },
}

/// Risk classification for a schema [`Change`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeRisk {
    /// Safe to apply — no data loss is possible.
    Safe,
    /// Requires confirmation — potential data loss or migration complexity.
    Destructive,
}

/// Presentation metadata for a schema [`Change`], as produced by
/// [`Change::describe`].
///
/// Carries only the text; rendering (colour, alignment, terminal width) is left
/// to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeDescription {
    /// Single-character marker: `+` for additions, `-` for removals, `~` for
    /// in-place alterations.
    pub sigil: &'static str,
    /// The object the change applies to, e.g. `users.email`, `type:address`.
    pub subject: String,
    /// Human-readable summary of the operation and its risk.
    pub annotation: String,
}

/// Classify a [`Change`] by its risk level.
///
/// Retained for backwards compatibility; prefer [`Change::risk`].
pub fn change_risk(change: &Change) -> ChangeRisk {
    change.risk()
}

impl Change {
    /// Classify this change by its risk level.
    pub fn risk(&self) -> ChangeRisk {
        match self {
            Change::NewTable(_)
            | Change::DefaultChanged { .. }
            | Change::IndexAdded { .. }
            | Change::IndexDropped { .. }
            | Change::ComputedExprChanged { .. }
            | Change::CheckChanged { .. } => ChangeRisk::Safe,

            Change::AddedColumn { field, .. } => {
                if field.is_required && field.default_value.is_none() && field.computed.is_none() {
                    ChangeRisk::Destructive
                } else {
                    ChangeRisk::Safe
                }
            }

            Change::NullabilityChanged {
                now_required: false,
                ..
            } => ChangeRisk::Safe,

            Change::AutoIncrementChanged { .. } => ChangeRisk::Safe,

            Change::DroppedTable { .. }
            | Change::DroppedColumn { .. }
            | Change::TypeChanged { .. }
            | Change::PrimaryKeyChanged { .. }
            | Change::NullabilityChanged {
                now_required: true, ..
            }
            | Change::DropEnum { .. }
            | Change::DropCompositeType { .. }
            | Change::DropExtension { .. } => ChangeRisk::Destructive,

            Change::CreateEnum { .. }
            | Change::CreateCompositeType { .. }
            | Change::CreateSchema { .. }
            | Change::CreateExtension { .. } => ChangeRisk::Safe,

            Change::AlterEnum {
                removed_variants, ..
            } => {
                if removed_variants.is_empty() {
                    ChangeRisk::Safe
                } else {
                    ChangeRisk::Destructive
                }
            }

            Change::AlterCompositeType {
                dropped_fields,
                type_changed_fields,
                ..
            } => {
                if dropped_fields.is_empty() && type_changed_fields.is_empty() {
                    ChangeRisk::Safe
                } else {
                    ChangeRisk::Destructive
                }
            }

            Change::ForeignKeyAdded { .. } => ChangeRisk::Destructive,
            Change::ForeignKeyDropped { .. } => ChangeRisk::Safe,
        }
    }

    /// Describe this change for display to the user.
    ///
    /// The wording of the annotation mirrors [`Change::risk`]: every change
    /// classified as [`ChangeRisk::Destructive`] says so, and explains why.
    pub fn describe(&self) -> ChangeDescription {
        let (sigil, subject, annotation) = match self {
            Change::NewTable(model) => ("+", model.db_name.clone(), "CREATE TABLE (safe)".into()),
            Change::DroppedTable { name } => (
                "-",
                name.to_string(),
                "DROP TABLE (destructive — data will be lost)".into(),
            ),
            Change::AddedColumn { table, field } => (
                "+",
                format!("{}.{}", table, field.db_name),
                "ADD COLUMN (safe)".into(),
            ),
            Change::DroppedColumn { table, column } => (
                "-",
                format!("{}.{}", table, column),
                "DROP COLUMN (destructive — data will be lost)".into(),
            ),
            Change::TypeChanged {
                table,
                column,
                from,
                to,
            } => (
                "~",
                format!("{}.{}", table, column),
                format!("TYPE {} -> {} (destructive — may truncate data)", from, to),
            ),
            Change::NullabilityChanged {
                table,
                column,
                now_required: true,
            } => (
                "~",
                format!("{}.{}", table, column),
                "NOT NULL (destructive — column may contain NULLs)".into(),
            ),
            Change::NullabilityChanged {
                table,
                column,
                now_required: false,
            } => ("~", format!("{}.{}", table, column), "NULL (safe)".into()),
            Change::AutoIncrementChanged {
                table,
                column,
                enabled: true,
            } => (
                "~",
                format!("{}.{}", table, column),
                "AUTO_INCREMENT (safe)".into(),
            ),
            Change::AutoIncrementChanged {
                table,
                column,
                enabled: false,
            } => (
                "~",
                format!("{}.{}", table, column),
                "DROP AUTO_INCREMENT (safe)".into(),
            ),
            Change::DefaultChanged {
                table,
                column,
                from,
                to,
            } => (
                "~",
                format!("{}.{}", table, column),
                format!(
                    "DEFAULT {} -> {} (safe)",
                    from.as_deref().unwrap_or("none"),
                    to.as_deref().unwrap_or("none"),
                ),
            ),
            Change::PrimaryKeyChanged { table } => (
                "~",
                table.to_string(),
                "PRIMARY KEY changed (destructive — requires rebuild)".into(),
            ),
            Change::IndexAdded {
                table,
                columns,
                unique,
                kind,
                ..
            } => ("+", format!("{} ({})", table, columns.join(", ")), {
                let type_str = kind
                    .as_type_str()
                    .map(|t| format!(" {} ", t))
                    .unwrap_or_default();
                if *unique {
                    format!("ADD UNIQUE{}INDEX (safe)", type_str)
                } else {
                    format!("ADD{}INDEX (safe)", type_str)
                }
            }),
            Change::IndexDropped { table, columns, .. } => (
                "-",
                format!("{} ({})", table, columns.join(", ")),
                "DROP INDEX (safe)".into(),
            ),
            Change::ComputedExprChanged { table, column, .. } => (
                "~",
                format!("{}.{}", table, column),
                "COMPUTED expression changed (safe — DROP + ADD COLUMN)".into(),
            ),
            Change::CheckChanged {
                table,
                column,
                from,
                to,
            } => (
                "~",
                match column {
                    Some(col) => format!("{}.{}", table, col),
                    None => table.to_string(),
                },
                format!(
                    "CHECK {} -> {} (safe)",
                    from.as_deref().unwrap_or("none"),
                    to.as_deref().unwrap_or("none"),
                ),
            ),
            Change::CreateCompositeType { name } => (
                "+",
                format!("type:{}", name),
                "CREATE TYPE composite (safe)".into(),
            ),
            Change::DropCompositeType { name } => (
                "-",
                format!("type:{}", name),
                "DROP TYPE composite (destructive — data will be lost)".into(),
            ),
            Change::AlterCompositeType {
                name,
                added_fields,
                dropped_fields,
                type_changed_fields,
            } => {
                let annotation = if dropped_fields.is_empty() && type_changed_fields.is_empty() {
                    format!("ADD ATTRIBUTE {} field(s) (safe)", added_fields.len())
                } else {
                    format!(
                        "ALTER TYPE: +{} ~{} -{} field(s) (destructive)",
                        added_fields.len(),
                        type_changed_fields.len(),
                        dropped_fields.len(),
                    )
                };
                ("~", format!("type:{}", name), annotation)
            }
            Change::CreateEnum { name, .. } => (
                "+",
                format!("enum:{}", name),
                "CREATE TYPE enum (safe)".into(),
            ),
            Change::DropEnum { name } => (
                "-",
                format!("enum:{}", name),
                "DROP TYPE enum (destructive — data will be lost)".into(),
            ),
            Change::AlterEnum {
                name,
                added_variants,
                removed_variants,
            } => {
                let annotation = if removed_variants.is_empty() {
                    format!("ADD VALUE {} variant(s) (safe)", added_variants.len())
                } else {
                    format!(
                        "ALTER ENUM: +{} -{} variant(s) (destructive — drop + recreate)",
                        added_variants.len(),
                        removed_variants.len(),
                    )
                };
                ("~", format!("enum:{}", name), annotation)
            }
            Change::CreateSchema { name } => (
                "+",
                format!("schema:{}", name),
                "CREATE SCHEMA (safe)".into(),
            ),
            Change::CreateExtension { name, schema } => {
                let annotation = match schema {
                    Some(s) => format!("CREATE EXTENSION ... WITH SCHEMA \"{}\" (safe)", s),
                    None => "CREATE EXTENSION (safe)".to_string(),
                };
                ("+", format!("ext:{}", name), annotation)
            }
            Change::DropExtension { name } => (
                "-",
                format!("ext:{}", name),
                "DROP EXTENSION (destructive — fails if objects still depend on it)".into(),
            ),
            Change::ForeignKeyAdded {
                table,
                columns,
                referenced_table,
                ..
            } => (
                "+",
                format!("{} ({})", table, columns.join(", ")),
                format!(
                    "ADD FOREIGN KEY -> {} (destructive — may fail on existing data)",
                    referenced_table,
                ),
            ),
            Change::ForeignKeyDropped {
                table,
                constraint_name,
            } => (
                "-",
                table.to_string(),
                format!("DROP FOREIGN KEY {} (safe)", constraint_name),
            ),
        };

        ChangeDescription {
            sigil,
            subject,
            annotation,
        }
    }
}
