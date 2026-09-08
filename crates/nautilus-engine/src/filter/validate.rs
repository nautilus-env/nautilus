//! Checks that involve more than one query argument, applied by the JSON and
//! the typed entry point alike.

use nautilus_protocol::ProtocolError;

use super::types::VectorNearestQuery;

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
