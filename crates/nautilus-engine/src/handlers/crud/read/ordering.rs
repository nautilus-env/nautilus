//! `orderBy` target resolution.
//!
//! An `orderBy` entry names either a column of the model or a single path into
//! a composite field. Both spellings resolve here, so the planner only has to
//! place the resulting term and pick its direction.

use std::collections::HashMap;

use nautilus_core::{Expr, JsonPathCast};
use nautilus_protocol::ProtocolError;
use nautilus_schema::ir::{CompositeFieldIr, ModelIr, ResolvedFieldType, ScalarType};

use crate::state::EngineState;

pub(super) enum ResolvedOrderTarget {
    Column(String),
    Expr(Expr),
}

fn strip_order_qualifier(name: &str) -> &str {
    name.split_once("__")
        .map(|(_, column)| column)
        .unwrap_or(name)
}

fn is_orderable_nested_field(field: &CompositeFieldIr) -> bool {
    if field.is_array {
        return false;
    }

    match &field.field_type {
        ResolvedFieldType::Enum { .. } => true,
        ResolvedFieldType::Scalar(
            ScalarType::Boolean
            | ScalarType::Json
            | ScalarType::Jsonb
            | ScalarType::Hstore
            | ScalarType::Geometry
            | ScalarType::Geography
            | ScalarType::Vector { .. }
            | ScalarType::Bytes,
        ) => false,
        ResolvedFieldType::Scalar(_) => true,
        ResolvedFieldType::CompositeType { .. } | ResolvedFieldType::Relation(_) => false,
    }
}

fn json_cast_for_nested_field(field: &CompositeFieldIr) -> JsonPathCast {
    match &field.field_type {
        ResolvedFieldType::Scalar(ScalarType::Int | ScalarType::BigInt) => JsonPathCast::Signed,
        ResolvedFieldType::Scalar(ScalarType::Float) => JsonPathCast::Double,
        ResolvedFieldType::Scalar(ScalarType::Decimal { .. }) => JsonPathCast::Decimal,
        _ => JsonPathCast::None,
    }
}

pub(super) fn resolve_order_target(
    state: &EngineState,
    model: &ModelIr,
    logical_to_db: &HashMap<String, String>,
    field_path: &str,
) -> Result<ResolvedOrderTarget, ProtocolError> {
    let field_path = strip_order_qualifier(field_path);
    let Some((parent_name, nested_name)) = field_path.split_once('.') else {
        if model
            .scalar_fields()
            .find(|field| field.logical_name == field_path || field.db_name == field_path)
            .is_some_and(|field| {
                matches!(field.field_type, ResolvedFieldType::CompositeType { .. })
            })
        {
            return Err(ProtocolError::InvalidFilter(format!(
                "Composite field '{}' cannot be ordered directly; use '{}.<field>'",
                field_path, field_path
            )));
        }

        let db_col = logical_to_db
            .get(field_path)
            .cloned()
            .unwrap_or_else(|| field_path.to_string());
        return Ok(ResolvedOrderTarget::Column(db_col));
    };

    let parent_field = model
        .scalar_fields()
        .find(|field| field.logical_name == parent_name || field.db_name == parent_name)
        .ok_or_else(|| {
            ProtocolError::InvalidFilter(format!("Unknown orderBy field '{}'", parent_name))
        })?;

    if parent_field.is_array {
        return Err(ProtocolError::InvalidFilter(format!(
            "Composite array field '{}' cannot be used with orderBy path '{}'",
            parent_name, field_path
        )));
    }

    let ResolvedFieldType::CompositeType { type_name, .. } = &parent_field.field_type else {
        return Err(ProtocolError::InvalidFilter(format!(
            "orderBy path '{}' starts with non-composite field '{}'",
            field_path, parent_name
        )));
    };

    let composite = state.schema.get_composite_type(type_name).ok_or_else(|| {
        ProtocolError::QueryPlanning(format!(
            "Composite type '{}' not found while resolving orderBy '{}'",
            type_name, field_path
        ))
    })?;

    let nested_field = composite
        .fields
        .iter()
        .find(|field| field.logical_name == nested_name || field.db_name == nested_name)
        .ok_or_else(|| {
            ProtocolError::InvalidFilter(format!(
                "Unknown orderBy composite field '{}.{}'",
                parent_name, nested_name
            ))
        })?;

    if !is_orderable_nested_field(nested_field) {
        return Err(ProtocolError::InvalidFilter(format!(
            "Field '{}' cannot be used with classic orderBy because it is not orderable",
            field_path
        )));
    }

    Ok(ResolvedOrderTarget::Expr(Expr::composite_field(
        model.db_name.clone(),
        parent_field.db_name.clone(),
        nested_field.db_name.clone(),
        nested_field.logical_name.clone(),
        json_cast_for_nested_field(nested_field),
    )))
}
