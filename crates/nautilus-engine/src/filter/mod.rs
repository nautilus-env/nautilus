//! Query arguments: the components the planner consumes and the two ways a
//! request produces them.
//!
//! [`types`] holds the shapes, [`json_args`] reads the JSON `args` object of a
//! request and [`typed_args`] adapts the Rust client's typed arguments; the
//! remaining modules own one argument each — `where`, `orderBy`, `include` and
//! `select` — plus the checks that span several of them.

mod context;
mod include;
mod json_args;
mod ordering;
#[cfg(test)]
mod tests;
mod typed_args;
mod types;
mod validate;
mod where_filter;

pub use types::{
    IncludeNode, JoinTableInfo, QueryArgs, RelationInfo, RelationJoin, RelationMap,
    VectorNearestQuery,
};

pub(crate) use context::SchemaContext;
pub(crate) use json_args::ensure_known_arg_keys;
pub(crate) use ordering::{parse_group_by_order_by, parse_having, GroupByOrderItem};
pub(crate) use types::FieldTypeMap;
pub(crate) use where_filter::{parse_where_filter, qualify_filter_columns};
