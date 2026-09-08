//! What every handler does with a request before it can act on it: read the
//! params it carries and resolve the model it names.

use nautilus_core::ColumnMarker;
use nautilus_protocol::{ProtocolError, RpcRequest};
use nautilus_schema::ir::{FieldIr, ModelIr};

use crate::state::EngineState;

/// Deserialize `request.params` directly into the handler's concrete params
/// type. This is the single per-request parse: the transport keeps `params`
/// as raw JSON (`Box<RawValue>`), so no intermediate `serde_json::Value` DOM
/// is built or re-walked here.
pub(crate) fn parse_params<P: serde::de::DeserializeOwned>(
    request: &RpcRequest,
    context: &str,
) -> Result<P, ProtocolError> {
    serde_json::from_str(request.params.get())
        .map_err(|e| ProtocolError::InvalidParams(format!("Invalid {context} params: {}", e)))
}

/// Build a `ColumnMarker` for a scalar field.
pub(crate) fn field_marker(model: &ModelIr, field: &FieldIr) -> ColumnMarker {
    ColumnMarker::new(&model.db_name, &field.db_name)
}

/// Build a map from logical field name -> resolved field type for a model.
/// Used by tests that exercise the filter parser in isolation.
#[cfg(test)]
pub(crate) fn build_field_type_map(model: &ModelIr) -> crate::filter::FieldTypeMap {
    crate::metadata::build_field_type_map(model)
}

/// Look up a model by logical name, returning a typed error on miss.
pub(crate) fn get_model_or_error<'a>(
    state: &'a EngineState,
    model_name: &str,
) -> Result<&'a ModelIr, ProtocolError> {
    state
        .models()
        .get(model_name)
        .ok_or_else(|| ProtocolError::InvalidModel(format!("Model not found: {}", model_name)))
}

/// Look up a model that a write may target, rejecting `view` blocks.
///
/// A view has no storage of its own, so every write method is a client error
/// rather than something the database could be asked to attempt.
pub(crate) fn get_writable_model_or_error<'a>(
    state: &'a EngineState,
    model_name: &str,
) -> Result<&'a ModelIr, ProtocolError> {
    let model = get_model_or_error(state, model_name)?;
    if model.is_view {
        return Err(ProtocolError::UnsupportedOperation(format!(
            "'{}' is a view and is read-only",
            model_name
        )));
    }
    Ok(model)
}

/// Time one in-process call and fold it into the per-method counters.
///
/// The typed and embedded entry points bypass [`dispatch`], so without this the
/// counters would only ever see requests that arrived over the wire and
#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_schema::ir::{PrimaryKeyIr, ResolvedFieldType, ScalarType};
    use nautilus_schema::Span;

    #[test]
    fn field_marker_builds_correct_marker() {
        let model = ModelIr {
            logical_name: "User".to_string(),
            db_name: "users".to_string(),
            schema: None,
            fields: vec![],
            primary_key: PrimaryKeyIr::Single("id".to_string()),
            unique_constraints: vec![],
            indexes: vec![],
            check_constraints: vec![],
            span: Span::new(0, 0),
            is_ignored: false,
            is_view: false,
            is_join_table: false,
        };
        let field = FieldIr {
            logical_name: "id".to_string(),
            db_name: "id".to_string(),
            field_type: ResolvedFieldType::Scalar(ScalarType::Int),
            is_required: true,
            is_array: false,
            storage_strategy: None,
            default_value: None,
            is_unique: false,
            is_updated_at: false,
            computed: None,
            check: None,
            span: Span::new(0, 0),
            is_ignored: false,
        };
        let marker = field_marker(&model, &field);
        assert_eq!(marker.table, "users");
        assert_eq!(marker.name, "id");
    }
}
