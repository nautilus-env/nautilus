//! RPC request dispatch and shared handler helpers.
//!
//! This module contains:
//! - `handle_request` — top-level entry point
//! - `dispatch` — inner routing table (also used by `transaction_batch`)
//! - `handle_handshake` — engine.handshake handler
//! - Shared helpers used by `crud` and `transactions` submodules

use nautilus_core::ColumnMarker;
use nautilus_protocol::wire::{err, ok};
use nautilus_protocol::{
    check_protocol_version, AggregateParams, CountParams, CreateManyParams, CreateParams,
    DeleteManyParams, EngineMetricsParams, GroupByParams, HandshakeParams, HandshakeResult,
    ProtocolError, RpcError, RpcRequest, RpcResponse, SchemaValidateParams, SchemaValidateResult,
    UpdateManyParams, UpdateParams, UpsertParams, ENGINE_HANDSHAKE, ENGINE_METRICS,
    PROTOCOL_VERSION, QUERY_AGGREGATE, QUERY_COUNT, QUERY_CREATE, QUERY_CREATE_MANY, QUERY_DELETE,
    QUERY_DELETE_MANY, QUERY_EXPLAIN, QUERY_FIND_FIRST, QUERY_FIND_FIRST_OR_THROW, QUERY_FIND_MANY,
    QUERY_FIND_UNIQUE, QUERY_FIND_UNIQUE_OR_THROW, QUERY_GROUP_BY, QUERY_RAW, QUERY_RAW_STMT,
    QUERY_UPDATE, QUERY_UPDATE_MANY, QUERY_UPSERT, SCHEMA_VALIDATE, TRANSACTION_BATCH,
    TRANSACTION_COMMIT, TRANSACTION_ROLLBACK, TRANSACTION_START,
};
use nautilus_schema::ir::{FieldIr, ModelIr};
use nautilus_schema::{analyze, Severity};
use tokio::sync::mpsc;

use crate::state::EngineState;

mod crud;
mod transactions;

/// Pure include-hydration helpers re-exported for the `hydrate_includes`
/// criterion bench. Not part of the public engine API.
#[doc(hidden)]
pub use crud::include::{build_include_values, group_key, GroupKey, IncludeProjection};

#[derive(Debug)]
pub enum EmbeddedResponse {
    Rows(Vec<nautilus_connector::Row>),
    Count(i64),
    Json(Box<serde_json::value::RawValue>),
}

/// Deserialize `request.params` directly into the handler's concrete params
/// type. This is the single per-request parse: the transport keeps `params`
/// as raw JSON (`Box<RawValue>`), so no intermediate `serde_json::Value` DOM
/// is built or re-walked here.
pub(super) fn parse_params<P: serde::de::DeserializeOwned>(
    request: &RpcRequest,
    context: &str,
) -> Result<P, ProtocolError> {
    serde_json::from_str(request.params.get())
        .map_err(|e| ProtocolError::InvalidParams(format!("Invalid {context} params: {}", e)))
}

/// Build a `ColumnMarker` for a scalar field.
pub(super) fn field_marker(model: &ModelIr, field: &FieldIr) -> ColumnMarker {
    ColumnMarker::new(&model.db_name, &field.db_name)
}

/// Build a map from logical field name -> resolved field type for a model.
/// Used by tests that exercise the filter parser in isolation.
#[cfg(test)]
pub(super) fn build_field_type_map(model: &ModelIr) -> crate::filter::FieldTypeMap {
    crate::metadata::build_field_type_map(model)
}

/// Look up a model by logical name, returning a typed error on miss.
pub(super) fn get_model_or_error<'a>(
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
pub(super) fn get_writable_model_or_error<'a>(
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
/// `engine.metrics` would read as empty for an embedded Rust client.
async fn recorded<T, F>(state: &EngineState, method: &str, call: F) -> Result<T, ProtocolError>
where
    F: std::future::Future<Output = Result<T, ProtocolError>>,
{
    let started = std::time::Instant::now();
    let result = call.await;
    state.record_request(method, started.elapsed(), result.is_err());
    result
}

/// Dispatch RPC request to the appropriate handler.
///
/// `tx` is the response channel — forwarded to `handle_find_many` so that it can
/// emit partial (chunked) responses before returning the final chunk.
pub async fn handle_request(
    state: &EngineState,
    request: RpcRequest,
    tx: mpsc::Sender<RpcResponse>,
) -> RpcResponse {
    let id = request.id.clone();

    if request.method == QUERY_FIND_MANY {
        let started = std::time::Instant::now();
        let result = crud::handle_find_many(state, request, Some(tx)).await;
        state.record_request(QUERY_FIND_MANY, started.elapsed(), result.is_err());
        return response_from_result(id, result);
    }

    response_from_result(id, dispatch(state, request).await)
}

/// Handle an in-process request without allocating a response channel.
///
/// This is intended for embedded callers that only consume the final response
/// and do not use chunked `findMany` partials.
pub async fn handle_request_inline(state: &EngineState, request: RpcRequest) -> RpcResponse {
    let id = request.id.clone();
    response_from_result(id, dispatch(state, request).await)
}

/// Handle an in-process request and return typed rows/counts when possible.
///
/// This is intended for embedded Rust callers that can consume modeled results
/// directly without serializing them through the public JSON-RPC wire shape.
pub async fn handle_request_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<EmbeddedResponse, ProtocolError> {
    let started = std::time::Instant::now();
    let method = request.method.clone();
    let result = handle_request_embedded_inner(state, request).await;
    state.record_request(&method, started.elapsed(), result.is_err());
    result
}

async fn handle_request_embedded_inner(
    state: &EngineState,
    request: RpcRequest,
) -> Result<EmbeddedResponse, ProtocolError> {
    match request.method.as_str() {
        QUERY_FIND_MANY => crud::handle_find_many_embedded(state, request)
            .await
            .map(EmbeddedResponse::Rows),
        QUERY_CREATE => crud::handle_create_embedded(state, request)
            .await
            .map(EmbeddedResponse::Rows),
        QUERY_CREATE_MANY => crud::handle_create_many_embedded(state, request)
            .await
            .map(EmbeddedResponse::Rows),
        QUERY_UPSERT => crud::handle_upsert_embedded(state, request)
            .await
            .map(EmbeddedResponse::Rows),
        QUERY_UPDATE => crud::handle_update_embedded(state, request)
            .await
            .map(EmbeddedResponse::Rows),
        QUERY_COUNT => crud::handle_count_embedded(state, request)
            .await
            .map(EmbeddedResponse::Count),
        QUERY_GROUP_BY => crud::handle_group_by_embedded(state, request)
            .await
            .map(EmbeddedResponse::Rows),
        _ => dispatch_inner(state, request)
            .await
            .map(EmbeddedResponse::Json),
    }
}

/// Handle a typed Rust `findMany` request in-process without going through the
/// JSON-RPC envelope or engine JSON argument format.
pub async fn handle_find_many_typed(
    state: &EngineState,
    model_name: &str,
    args: &nautilus_core::FindManyArgs,
    transaction_id: Option<&str>,
) -> Result<Vec<nautilus_connector::Row>, ProtocolError> {
    recorded(
        state,
        QUERY_FIND_MANY,
        crud::handle_find_many_typed(state, model_name, args, transaction_id),
    )
    .await
}

/// Handle a typed Rust `findUnique` request in-process without an RPC envelope.
pub async fn handle_find_unique_typed(
    state: &EngineState,
    model_name: &str,
    args: &nautilus_core::FindUniqueArgs,
    transaction_id: Option<&str>,
) -> Result<Vec<nautilus_connector::Row>, ProtocolError> {
    recorded(
        state,
        QUERY_FIND_UNIQUE,
        crud::handle_find_unique_typed(state, model_name, args, transaction_id),
    )
    .await
}

/// Handle a typed Rust `create` request in-process without an RPC envelope.
pub async fn handle_create_typed(
    state: &EngineState,
    params: CreateParams,
) -> Result<Vec<nautilus_connector::Row>, ProtocolError> {
    recorded(
        state,
        QUERY_CREATE,
        crud::handle_create_typed(state, params),
    )
    .await
}

/// Handle a typed Rust `createMany` request in-process without an RPC envelope.
pub async fn handle_create_many_typed(
    state: &EngineState,
    params: CreateManyParams,
) -> Result<Vec<nautilus_connector::Row>, ProtocolError> {
    recorded(
        state,
        QUERY_CREATE_MANY,
        crud::handle_create_many_typed(state, params),
    )
    .await
}

/// Handle a typed Rust `update` request in-process without an RPC envelope.
pub async fn handle_update_typed(
    state: &EngineState,
    params: UpdateParams,
) -> Result<Vec<nautilus_connector::Row>, ProtocolError> {
    recorded(
        state,
        QUERY_UPDATE,
        crud::handle_update_typed(state, params),
    )
    .await
}

/// Handle a typed Rust `upsert` request in-process without an RPC envelope.
pub async fn handle_upsert_typed(
    state: &EngineState,
    params: UpsertParams,
) -> Result<Vec<nautilus_connector::Row>, ProtocolError> {
    recorded(
        state,
        QUERY_UPSERT,
        crud::handle_upsert_typed(state, params),
    )
    .await
}

/// Snapshot the engine's runtime counters in-process without an RPC envelope.
pub async fn engine_metrics_typed(
    state: &EngineState,
    reset: bool,
) -> nautilus_protocol::EngineMetricsResult {
    state.metrics_snapshot(reset).await
}

/// Handle a typed Rust `updateMany` request in-process without an RPC envelope.
pub async fn handle_update_many_typed(
    state: &EngineState,
    params: UpdateManyParams,
) -> Result<usize, ProtocolError> {
    recorded(
        state,
        QUERY_UPDATE_MANY,
        crud::handle_update_many_typed(state, params),
    )
    .await
}

/// Handle a typed Rust `deleteMany` request in-process without an RPC envelope.
pub async fn handle_delete_many_typed(
    state: &EngineState,
    params: DeleteManyParams,
) -> Result<usize, ProtocolError> {
    recorded(
        state,
        QUERY_DELETE_MANY,
        crud::handle_delete_many_typed(state, params),
    )
    .await
}

/// Handle a typed Rust `aggregate` request in-process without an RPC envelope.
pub async fn handle_aggregate_typed(
    state: &EngineState,
    params: AggregateParams,
) -> Result<Vec<nautilus_connector::Row>, ProtocolError> {
    recorded(
        state,
        QUERY_AGGREGATE,
        crud::handle_aggregate_typed(state, params),
    )
    .await
}

/// Handle a typed Rust `explain` request in-process without an RPC envelope.
pub async fn handle_explain_typed(
    state: &EngineState,
    model_name: &str,
    args: &nautilus_core::FindManyArgs,
    analyze: bool,
    transaction_id: Option<&str>,
) -> Result<nautilus_protocol::ExplainResult, ProtocolError> {
    recorded(
        state,
        QUERY_EXPLAIN,
        crud::handle_explain_typed(state, model_name, args, analyze, transaction_id),
    )
    .await
}

/// Handle a typed Rust `count` request in-process without an RPC envelope.
pub async fn handle_count_typed(
    state: &EngineState,
    params: CountParams,
) -> Result<i64, ProtocolError> {
    recorded(state, QUERY_COUNT, crud::handle_count_typed(state, params)).await
}

/// Handle a typed Rust `groupBy` request in-process without an RPC envelope.
pub async fn handle_group_by_typed(
    state: &EngineState,
    params: GroupByParams,
) -> Result<Vec<nautilus_connector::Row>, ProtocolError> {
    recorded(
        state,
        QUERY_GROUP_BY,
        crud::handle_group_by_typed(state, params),
    )
    .await
}

fn response_from_result(
    id: Option<nautilus_protocol::RpcId>,
    result: Result<Box<serde_json::value::RawValue>, ProtocolError>,
) -> RpcResponse {
    match result {
        Ok(value) => ok(id, value),
        Err(protocol_error) => {
            let rpc_error: RpcError = protocol_error.into();
            err(id, rpc_error.code, rpc_error.message, rpc_error.data)
        }
    }
}

/// Inner dispatch: route method name to handler, returning a raw Result.
///
/// Extracted so that [`transactions::handle_transaction_batch`] can re-use the
/// same routing table without constructing full RPC responses.
pub(super) async fn dispatch(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let started = std::time::Instant::now();
    let method = request.method.clone();
    let result = dispatch_inner(state, request).await;
    state.record_request(&method, started.elapsed(), result.is_err());
    result
}

async fn dispatch_inner(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    match request.method.as_str() {
        ENGINE_HANDSHAKE => handle_handshake(state, request).await,
        SCHEMA_VALIDATE => handle_schema_validate(state, request).await,
        ENGINE_METRICS => handle_engine_metrics(state, request).await,
        QUERY_FIND_MANY => crud::handle_find_many(state, request, None).await,
        QUERY_FIND_FIRST => crud::handle_find_first(state, request).await,
        QUERY_FIND_UNIQUE => crud::handle_find_unique(state, request).await,
        QUERY_FIND_UNIQUE_OR_THROW => crud::handle_find_unique_or_throw(state, request).await,
        QUERY_FIND_FIRST_OR_THROW => crud::handle_find_first_or_throw(state, request).await,
        QUERY_CREATE => crud::handle_create(state, request).await,
        QUERY_CREATE_MANY => crud::handle_create_many(state, request).await,
        QUERY_UPDATE => crud::handle_update(state, request).await,
        QUERY_UPDATE_MANY => crud::handle_update_many(state, request).await,
        QUERY_UPSERT => crud::handle_upsert(state, request).await,
        QUERY_DELETE => crud::handle_delete(state, request).await,
        QUERY_DELETE_MANY => crud::handle_delete_many(state, request).await,
        QUERY_COUNT => crud::handle_count(state, request).await,
        QUERY_GROUP_BY => crud::handle_group_by(state, request).await,
        QUERY_AGGREGATE => crud::handle_aggregate(state, request).await,
        QUERY_EXPLAIN => crud::handle_explain(state, request).await,
        QUERY_RAW => crud::handle_raw_query(state, request).await,
        QUERY_RAW_STMT => crud::handle_raw_stmt_query(state, request).await,
        TRANSACTION_START => transactions::handle_transaction_start(state, request).await,
        TRANSACTION_COMMIT => transactions::handle_transaction_commit(state, request).await,
        TRANSACTION_ROLLBACK => transactions::handle_transaction_rollback(state, request).await,
        TRANSACTION_BATCH => transactions::handle_transaction_batch(state, request).await,
        _ => Err(ProtocolError::InvalidMethod(request.method)),
    }
}

/// Handle engine.handshake
async fn handle_handshake(
    _state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: HandshakeParams = parse_params(&request, "handshake")?;

    check_protocol_version(params.protocol_version)?;

    let result = HandshakeResult {
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
    };

    serialize_result(&result, "handshake result")
}

/// Handle `engine.metrics`.
async fn handle_engine_metrics(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: EngineMetricsParams = parse_params(&request, "engine.metrics")?;

    check_protocol_version(params.protocol_version)?;

    serialize_result(
        &state.metrics_snapshot(params.reset).await,
        "engine.metrics result",
    )
}

async fn handle_schema_validate(
    _state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: SchemaValidateParams = parse_params(&request, "schema.validate")?;

    check_protocol_version(params.protocol_version)?;

    let analysis = analyze(&params.schema);
    let errors: Vec<String> = analysis
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| diagnostic.message)
        .collect();

    let result = SchemaValidateResult {
        valid: errors.is_empty(),
        errors: (!errors.is_empty()).then_some(errors),
    };

    serialize_result(&result, "schema.validate result")
}

fn serialize_result<T: serde::Serialize>(
    result: &T,
    context: &str,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let s = sonic_rs::to_string(result)
        .map_err(|e| ProtocolError::Internal(format!("Failed to serialize {context}: {}", e)))?;
    serde_json::value::RawValue::from_string(s)
        .map_err(|e| ProtocolError::Internal(format!("Failed to wrap {context}: {}", e)))
}

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
