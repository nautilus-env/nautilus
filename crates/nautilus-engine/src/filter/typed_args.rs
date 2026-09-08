//! Adaptation of the Rust client's typed `FindManyArgs` into [`QueryArgs`],
//! without passing through the JSON protocol shape.
//!
//! The typed form already carries parsed expressions, so this path only
//! normalizes the names it received and applies the checks the JSON parser
//! applies to the arguments it produces.

use std::collections::{HashMap, HashSet};

use nautilus_core::{FindManyArgs, IncludeRelation, OrderBy, Value};
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{ResolvedFieldType, ScalarType};

use super::types::{FieldTypeMap, IncludeNode, QueryArgs, VectorNearestQuery};
use super::validate::{ensure_nearest_supported, ensure_select_include_exclusive};

fn strip_column_qualifier(name: &str) -> String {
    name.split_once("__")
        .map(|(_, column)| column.to_string())
        .unwrap_or_else(|| name.to_string())
}

fn normalize_order_by(order_by: &[OrderBy]) -> Vec<OrderBy> {
    order_by
        .iter()
        .map(|order| OrderBy::new(strip_column_qualifier(&order.column), order.direction))
        .collect()
}

fn normalize_select(select: &HashMap<String, bool>) -> HashSet<String> {
    select
        .iter()
        .filter(|(_, enabled)| **enabled)
        .map(|(field, _)| strip_column_qualifier(field))
        .collect()
}

fn normalize_cursor(cursor: &HashMap<String, Value>) -> HashMap<String, Value> {
    cursor
        .iter()
        .map(|(field, value)| (strip_column_qualifier(field), value.clone()))
        .collect()
}

fn normalize_distinct(distinct: &[String]) -> Vec<String> {
    distinct
        .iter()
        .map(|field| strip_column_qualifier(field))
        .collect()
}

fn include_relation_to_node(include: &IncludeRelation) -> IncludeNode {
    IncludeNode {
        filter: include.where_.clone(),
        nested: include_map_to_nodes(&include.include),
        take: include.take,
        skip: include.skip,
        order_by: normalize_order_by(&include.order_by),
    }
}

fn include_map_to_nodes(
    include: &HashMap<String, IncludeRelation>,
) -> HashMap<String, IncludeNode> {
    include
        .iter()
        .map(|(field, relation)| (field.clone(), include_relation_to_node(relation)))
        .collect()
}
impl QueryArgs {
    /// Convert typed Rust `FindManyArgs` into internal query components without
    /// passing through the JSON protocol shape.
    pub(crate) fn from_find_many_args(
        args: &FindManyArgs,
        field_types: &FieldTypeMap,
    ) -> Result<Self, ProtocolError> {
        let select = normalize_select(&args.select);
        let include = include_map_to_nodes(&args.include);

        ensure_select_include_exclusive(&select, &include)?;

        let (take, backward) = if let Some(take) = args.take {
            if take < 0 {
                (Some(take.unsigned_abs() as i32), true)
            } else {
                (Some(take), false)
            }
        } else {
            (None, false)
        };

        let cursor = args.cursor.as_ref().map(normalize_cursor);
        let distinct = normalize_distinct(&args.distinct);
        let nearest = if let Some(nearest) = args.nearest.as_ref() {
            let field = strip_column_qualifier(&nearest.field);
            let field_type = field_types.get(field.as_str()).ok_or_else(|| {
                ProtocolError::InvalidParams(format!(
                    "'nearest.field' references unknown field '{}'",
                    field
                ))
            })?;

            let ResolvedFieldType::Scalar(ScalarType::Vector { .. }) = field_type else {
                return Err(ProtocolError::InvalidParams(format!(
                    "'nearest.field' must reference a Vector field, got '{}'",
                    field
                )));
            };

            Some(VectorNearestQuery {
                field,
                query: nearest.query.clone(),
                metric: nearest.metric,
            })
        } else {
            None
        };

        ensure_nearest_supported(
            nearest.as_ref(),
            take,
            backward,
            cursor.is_some(),
            &distinct,
        )?;

        Ok(QueryArgs {
            filter: args.where_.clone(),
            order_by: normalize_order_by(&args.order_by),
            take,
            skip: args.skip,
            include,
            select,
            cursor,
            backward,
            distinct,
            nearest,
            partition: None,
            join: None,
        })
    }
}
