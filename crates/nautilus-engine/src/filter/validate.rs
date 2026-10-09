//! Checks that involve more than one query argument, applied by the JSON and
//! the typed entry point alike.

use nautilus_protocol::ProtocolError;

use super::types::FieldTypeMap;
use super::types::VectorNearestQuery;
use nautilus_schema::ir::{ResolvedFieldType, ScalarType};

/// PostgreSQL XML has no equality or ordering, so neither a scalar XML
/// column nor an XML array can be compared as a query key.
pub(crate) fn ensure_xml_comparable(
    field: &str,
    field_types: &FieldTypeMap,
    operation: &str,
) -> Result<(), ProtocolError> {
    let column = field.split_once("__").map_or(field, |(_, column)| column);
    if field_types
        .get(column)
        .is_some_and(|field| matches!(field.field_type, ResolvedFieldType::Scalar(ScalarType::Xml)))
    {
        return Err(ProtocolError::InvalidFilter(format!(
            "Xml field '{}' cannot be used with {} because XML has no comparison operators",
            field, operation
        )));
    }
    Ok(())
}

/// Refuse a request that both projects columns and eager-loads relations.
pub(super) fn ensure_select_include_exclusive<T>(
    select: &std::collections::HashSet<String>,
    include: &std::collections::HashMap<String, T>,
) -> Result<(), ProtocolError> {
    if select.is_empty() || include.is_empty() {
        return Ok(());
    }
    Err(ProtocolError::InvalidParams(
        "'select' and 'include' cannot be used together. Use 'select' for projection only, or 'include' for relation loading.".to_string(),
    ))
}

/// Refuse the argument combinations a nearest-neighbour search cannot answer.
///
/// The ordering is the distance to the query embedding, so it has no stable
/// key to page from and no row to deduplicate against; the limit is what makes
/// the search bounded in the first place.
pub(super) fn ensure_nearest_supported(
    nearest: Option<&VectorNearestQuery>,
    take: Option<i32>,
    backward: bool,
    cursor: bool,
    distinct: &[String],
) -> Result<(), ProtocolError> {
    if nearest.is_none() {
        return Ok(());
    }
    if !matches!(take, Some(value) if value > 0) {
        return Err(ProtocolError::InvalidParams(
            "'nearest' requires a positive 'take' limit".to_string(),
        ));
    }
    if backward {
        return Err(ProtocolError::InvalidParams(
            "'nearest' does not support backward pagination".to_string(),
        ));
    }
    if cursor {
        return Err(ProtocolError::InvalidParams(
            "'nearest' cannot be combined with 'cursor'".to_string(),
        ));
    }
    if !distinct.is_empty() {
        return Err(ProtocolError::InvalidParams(
            "'nearest' cannot be combined with 'distinct'".to_string(),
        ));
    }
    Ok(())
}
