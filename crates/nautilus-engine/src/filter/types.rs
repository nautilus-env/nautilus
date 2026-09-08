//! The query components the planner consumes.
//!
//! [`QueryArgs`] is filled either from the JSON `args` object or from the typed
//! `FindManyArgs` of the Rust client; the relation and include descriptions
//! here are what both forms produce.

use std::collections::{HashMap, HashSet};

use nautilus_core::{Expr, JoinClause, OrderBy, PartitionWindow, TableName, Value, VectorMetric};
use nautilus_schema::ir::ResolvedFieldType;

use crate::conversion::ValueHint;

/// Map from a model's logical field name to its resolved field type.
/// Used during filter parsing to emit `Value::Enum` for enum-typed fields.
pub(crate) type FieldTypeMap = HashMap<String, ResolvedFieldType>;

/// Metadata needed to compile EXISTS / NOT EXISTS subqueries for a single relation field.
#[derive(Debug, Clone)]
pub struct RelationInfo {
    /// Database table name of the parent model (the one being queried).
    pub parent_table: String,
    /// Logical model name of the target / child model (key in SchemaIr.models).
    pub target_logical_name: String,
    /// Database table name of the target / child model.
    pub target_table: TableName,
    /// DB-level column name of the FK in the **child** table (e.g. `"user_id"`).
    pub fk_db: String,
    /// DB-level column name of the PK in the **parent** table (e.g. `"id"`).
    pub pk_db: String,
    /// Whether this relation is one-to-many (`true`) or one-to-one / FK-side (`false`).
    pub is_array: bool,
    /// The join table, when the relation is an implicit many-to-many.
    ///
    /// Neither side holds a foreign key then, so `fk_db` names the target's own
    /// key column and every query reaches the children through the table
    /// described here instead of a column on one of the two models.
    pub via: Option<JoinTableInfo>,
}

/// The join table of an implicit many-to-many, as the query planner needs it.
#[derive(Debug, Clone)]
pub struct JoinTableInfo {
    /// Physical name of the join table.
    pub table: TableName,
    /// Join-table column holding the parent's key.
    pub parent_column: String,
    /// Join-table column holding the child's key.
    pub child_column: String,
}

/// A map from relation *field* name (logical, as used in the `where` / `include` payload)
/// to its join metadata. Pass an empty map when no schema context is available.
pub type RelationMap = HashMap<String, RelationInfo>;

/// A node in the include tree for one relation.
#[derive(Debug, Clone)]
pub struct IncludeNode {
    /// Optional WHERE filter applied to the child relation query.
    pub filter: Option<Expr>,
    /// Nested includes: child's relation field name -> its own IncludeNode.
    pub nested: HashMap<String, IncludeNode>,
    /// LIMIT to apply to the child relation subquery (array relations only).
    pub take: Option<i32>,
    /// OFFSET to apply to the child relation subquery (array relations only).
    pub skip: Option<u32>,
    /// ORDER BY clauses to apply to the child relation subquery.
    pub order_by: Vec<OrderBy>,
}

/// pgvector nearest-neighbor search specification parsed from query args.
#[derive(Debug, Clone)]
pub struct VectorNearestQuery {
    /// Logical field name of the vector field.
    pub field: String,
    /// Query embedding.
    pub query: Vec<f32>,
    /// Distance metric used for ordering.
    pub metric: VectorMetric,
}

/// Parse query arguments from JSON into query components.
#[derive(Debug)]
pub struct QueryArgs {
    pub filter: Option<Expr>,
    pub order_by: Vec<OrderBy>,
    /// Absolute number of rows to fetch (direction is in `backward`).
    pub take: Option<i32>,
    /// Number of rows to skip (OFFSET).
    pub skip: Option<u32>,
    /// Relation fields to eager-load. Key = logical field name.
    pub include: HashMap<String, IncludeNode>,
    /// Projection: set of logical field names to SELECT. Empty = select all columns.
    pub select: HashSet<String>,
    /// Cursor for stable pagination: PK field name -> value, parsed from the `"cursor"` key.
    pub cursor: Option<HashMap<String, Value>>,
    /// True when the caller passed a negative `take`, requesting backward pagination.
    pub backward: bool,
    /// Columns to deduplicate on (maps to SELECT DISTINCT / DISTINCT ON).
    pub distinct: Vec<String>,
    /// Optional pgvector nearest-neighbor ordering.
    pub nearest: Option<VectorNearestQuery>,
    /// Optional per-partition row window, which makes `take`/`skip` apply once
    /// per group instead of once per result set. Set by the batched include
    /// path; never parsed from client args.
    pub partition: Option<PartitionWindow>,
    /// Optional extra table joined into the query. Set by the include path for
    /// an implicit many-to-many; never parsed from client args.
    pub join: Option<RelationJoin>,
}

/// A table joined into a `findMany` on top of the model's own, together with
/// the columns it contributes to every row.
///
/// Only the join table of an implicit many-to-many uses this: the relation has
/// no foreign key on either model, so the parent key each child belongs to has
/// to be read out of the join table and travel with the child row.
#[derive(Debug, Clone)]
pub struct RelationJoin {
    /// The joined table and its `ON` condition.
    pub clause: JoinClause,
    /// Decoding hint for each column of `clause.items`, in the same order.
    pub hints: Vec<Option<ValueHint>>,
}
