//! In-process entry points for an embedded Rust client.
//!
//! These skip the JSON-RPC envelope: a typed call reaches the handler with the
//! arguments it already holds, and an embedded request answers with decoded
//! rows instead of serialized JSON where the method allows it. Both still fold
//! their timing into the counters `engine.metrics` reports.

use nautilus_connector::Row;
use nautilus_core::{FindManyArgs, FindUniqueArgs};
use nautilus_protocol::{
    AggregateParams, CountParams, CreateManyParams, CreateParams, DeleteManyParams,
    EngineMetricsResult, ExplainResult, GroupByParams, ProtocolError, RpcRequest, UpdateManyParams,
    UpdateParams, UpsertParams, QUERY_AGGREGATE, QUERY_COUNT, QUERY_CREATE, QUERY_CREATE_MANY,
    QUERY_DELETE_MANY, QUERY_EXPLAIN, QUERY_FIND_MANY, QUERY_FIND_UNIQUE, QUERY_GROUP_BY,
    QUERY_UPDATE, QUERY_UPDATE_MANY, QUERY_UPSERT,
};

use super::{crud, dispatch_inner};
use crate::state::EngineState;

#[derive(Debug)]
pub enum EmbeddedResponse {
    Rows(Vec<Row>),
    Count(i64),
    Json(Box<serde_json::value::RawValue>),
}

/// Time one in-process call and fold it into the per-method counters.
///
/// The typed and embedded entry points bypass [`super::dispatch`], so without this the
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
    args: &FindManyArgs,
    transaction_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
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
    args: &FindUniqueArgs,
    transaction_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
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
) -> Result<Vec<Row>, ProtocolError> {
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
) -> Result<Vec<Row>, ProtocolError> {
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
) -> Result<Vec<Row>, ProtocolError> {
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
) -> Result<Vec<Row>, ProtocolError> {
    recorded(
        state,
        QUERY_UPSERT,
        crud::handle_upsert_typed(state, params),
    )
    .await
}

/// Snapshot the engine's runtime counters in-process without an RPC envelope.
pub async fn engine_metrics_typed(state: &EngineState, reset: bool) -> EngineMetricsResult {
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
) -> Result<Vec<Row>, ProtocolError> {
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
    args: &FindManyArgs,
    analyze: bool,
    transaction_id: Option<&str>,
) -> Result<ExplainResult, ProtocolError> {
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
) -> Result<Vec<Row>, ProtocolError> {
    recorded(
        state,
        QUERY_GROUP_BY,
        crud::handle_group_by_typed(state, params),
    )
    .await
}
