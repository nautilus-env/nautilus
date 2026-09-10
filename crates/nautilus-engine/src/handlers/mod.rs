//! RPC request dispatch.
//!
//! [`handle_request`] and [`handle_request_inline`] answer a request that
//! arrived over the wire; `dispatch` is the routing table they and
//! `transactions::handle_transaction_batch` share. The handlers themselves
//! live in `crud` for the model methods, `service` for the methods about
//! the engine and `transactions` for the transaction lifecycle, while
//! `embedded` holds the in-process entry points of the Rust client.

use nautilus_protocol::wire::{err, ok};
use nautilus_protocol::{
    ProtocolError, RpcError, RpcRequest, RpcResponse, ENGINE_HANDSHAKE, ENGINE_METRICS,
    QUERY_AGGREGATE, QUERY_COUNT, QUERY_CREATE, QUERY_CREATE_MANY, QUERY_DELETE, QUERY_DELETE_MANY,
    QUERY_EXPLAIN, QUERY_FIND_FIRST, QUERY_FIND_FIRST_OR_THROW, QUERY_FIND_MANY, QUERY_FIND_UNIQUE,
    QUERY_FIND_UNIQUE_OR_THROW, QUERY_GROUP_BY, QUERY_RAW, QUERY_RAW_STMT, QUERY_UPDATE,
    QUERY_UPDATE_MANY, QUERY_UPSERT, SCHEMA_VALIDATE, TRANSACTION_BATCH, TRANSACTION_COMMIT,
    TRANSACTION_ROLLBACK, TRANSACTION_START,
};
use tokio::sync::mpsc;

use crate::state::EngineState;

mod crud;
mod embedded;
mod request;
mod service;
mod transactions;

/// Pure include-hydration helpers re-exported for the `hydrate_includes`
/// criterion bench. Not part of the public engine API.
#[doc(hidden)]
pub use crud::include::{build_include_values, group_key, GroupKey, IncludeProjection};

pub use embedded::{
    engine_metrics_typed, handle_aggregate_typed, handle_count_typed, handle_create_many_typed,
    handle_create_typed, handle_delete_many_typed, handle_explain_typed, handle_find_many_typed,
    handle_find_unique_typed, handle_group_by_typed, handle_request_embedded,
    handle_update_many_typed, handle_update_typed, handle_upsert_typed, EmbeddedResponse,
};

pub(super) use request::{
    field_marker, get_model_or_error, get_writable_model_or_error, parse_params,
};

#[cfg(test)]
pub(super) use request::build_field_type_map;

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

pub(super) async fn dispatch_inner(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    match request.method.as_str() {
        ENGINE_HANDSHAKE => service::handle_handshake(state, request).await,
        SCHEMA_VALIDATE => service::handle_schema_validate(state, request).await,
        ENGINE_METRICS => service::handle_engine_metrics(state, request).await,
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
