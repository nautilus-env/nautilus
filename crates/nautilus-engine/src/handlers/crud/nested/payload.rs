//! The shapes an operation payload is allowed to take.
//!
//! Nested operations accept a single object, an array of them, or `true` for
//! "every child of this parent", and address rows with a filter written either
//! bare or wrapped in `{ "where": ... }`. Reading raw JSON stops here: the
//! operations themselves work with the filters and `data` objects these
//! functions hand back.
use nautilus_protocol::ProtocolError;
use serde_json::{Map as JsonMap, Value as JsonValue};

/// One payload or each element of an array of them.
pub(super) fn payload_items(payload: &JsonValue) -> Vec<&JsonValue> {
    match payload {
        JsonValue::Array(items) => items.iter().collect(),
        other => vec![other],
    }
}

pub(super) fn require_object<'a>(
    value: &'a JsonValue,
    context: &str,
) -> Result<&'a JsonMap<String, JsonValue>, ProtocolError> {
    value
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams(format!("{context} must be an object")))
}

pub(super) fn require_member<'a>(
    object: &'a JsonMap<String, JsonValue>,
    key: &str,
    context: &str,
) -> Result<&'a JsonValue, ProtocolError> {
    object
        .get(key)
        .ok_or_else(|| ProtocolError::InvalidParams(format!("{context} needs a '{key}' entry")))
}

/// Accept a filter written either bare or wrapped in `{ "where": ... }`.
pub(super) fn unwrap_where(value: &JsonValue) -> JsonValue {
    value
        .as_object()
        .filter(|obj| obj.len() == 1)
        .and_then(|obj| obj.get("where"))
        .cloned()
        .unwrap_or_else(|| value.clone())
}

/// Narrow `scope` with a filter the caller supplied, which can only ever
/// restrict it further.
pub(super) fn scoped_filter(scope: JsonValue, extra: Option<&JsonValue>) -> JsonValue {
    match extra {
        Some(extra) if extra.as_object().is_some_and(|obj| !obj.is_empty()) => {
            serde_json::json!({ "AND": [scope, unwrap_where(extra)] })
        }
        _ => scope,
    }
}

/// Filters for the operations that address children by an optional `where`:
/// `true` (or no payload at all) means every child of this parent.
pub(super) fn child_filters(payload: &JsonValue, link: &JsonValue) -> Vec<JsonValue> {
    match payload {
        JsonValue::Bool(true) | JsonValue::Null => vec![link.clone()],
        JsonValue::Array(items) => items
            .iter()
            .map(|item| scoped_filter(link.clone(), Some(item)))
            .collect(),
        other => vec![scoped_filter(link.clone(), Some(other))],
    }
}

/// The `data` of a created child, with the columns pointing it at the parent.
pub(super) fn merge_link(
    item: &JsonValue,
    link: &JsonValue,
    field_name: &str,
) -> Result<JsonValue, ProtocolError> {
    let mut data = item
        .as_object()
        .ok_or_else(|| {
            ProtocolError::InvalidParams(format!(
                "Nested create on '{}' must be an object or an array of objects",
                field_name
            ))
        })?
        .clone();

    if let Some(link) = link.as_object() {
        for (key, value) in link {
            data.insert(key.clone(), value.clone());
        }
    }
    Ok(JsonValue::Object(data))
}

#[cfg(test)]
mod tests {
    use super::{child_filters, unwrap_where};
    use serde_json::Value as JsonValue;

    #[test]
    fn a_bare_filter_and_a_wrapped_one_unwrap_alike() {
        let bare = serde_json::json!({ "id": 1 });
        let wrapped = serde_json::json!({ "where": { "id": 1 } });
        assert_eq!(unwrap_where(&bare), unwrap_where(&wrapped));
    }

    #[test]
    fn a_child_filter_narrows_to_the_parent_link() {
        let link = serde_json::json!({ "authorId": 7 });
        assert_eq!(
            child_filters(&JsonValue::Bool(true), &link),
            vec![link.clone()]
        );
        assert_eq!(
            child_filters(&serde_json::json!({ "id": 3 }), &link),
            vec![serde_json::json!({ "AND": [{ "authorId": 7 }, { "id": 3 }] })]
        );
    }
}
