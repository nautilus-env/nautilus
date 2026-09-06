//! `findMany` and `findFirst`.
//!
//! The RPC, embedded and typed entry points differ only in how they obtain
//! their arguments: each parses its own params, then hands a [`QueryArgs`] to
//! the shared executor, which runs the plan and hydrates relation includes.

use std::sync::Arc;

use nautilus_connector::Row;
use nautilus_dialect::Sql;
use nautilus_protocol::{
    FindFirstParams, FindManyParams, ProtocolError, RpcId, RpcRequest, RpcResponse,
};
use nautilus_schema::ir::ModelIr;
use serde_json::value::RawValue;
use tokio::sync::mpsc;

use super::plan::{build_find_many_plan, find_many_cache_request};
use super::stream;
use crate::conversion::{check_protocol_version, normalize_rows_with_hints};
use crate::filter::{QueryArgs, SchemaContext};
use crate::handlers::crud::common::wrap_data_result;
use crate::handlers::crud::include::hydrate_rows_with_includes;
use crate::handlers::{get_model_or_error, parse_params};
use crate::plan_cache::CachedReadPlan;
use crate::state::EngineState;

/// Run a parsed `findMany`, from either the plan cache or a freshly built
/// plan, and apply the transformations the plan could not push into the SQL.
pub(in crate::handlers::crud) async fn execute_find_many_rows(
    state: &EngineState,
    model: &ModelIr,
    query_args: QueryArgs,
    tx_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    if let Some((cache_key, params)) = find_many_cache_request(state, model, &query_args) {
        if let Some(plan) = state.plan_cache().get_find_many(&cache_key) {
            let sql = Sql {
                text: plan.sql_text.clone(),
                params,
            };
            return normalize_rows_with_hints(
                state.execute_query_on(&sql, "Query", tx_id).await?,
                &plan.row_hints,
            );
        }

        let plan = build_find_many_plan(state, model, query_args)?;
        state.plan_cache().insert_find_many(
            cache_key,
            Arc::new(CachedReadPlan {
                sql_text: plan.sql.text.clone(),
                row_hints: plan.row_hints.clone(),
            }),
        );
        return normalize_rows_with_hints(
            state.execute_query_on(&plan.sql, "Query", tx_id).await?,
            &plan.row_hints,
        );
    }

    let plan = build_find_many_plan(state, model, query_args)?;
    let mut rows = normalize_rows_with_hints(
        state.execute_query_on(&plan.sql, "Query", tx_id).await?,
        &plan.row_hints,
    )?;

    if let Some(distinct) = plan.distinct.as_ref() {
        rows = distinct.apply(rows);
    }

    if plan.backward {
        rows.reverse();
    }

    hydrate_rows_with_includes(state, model, rows, &plan.include, tx_id).await
}

async fn execute_find_many_params(
    state: &EngineState,
    params: FindManyParams,
) -> Result<Vec<Row>, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;

    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let relation_map = state.relation_map_for_model(model)?;
    let query_args = QueryArgs::parse_with_context(
        params.args,
        relation_map,
        metadata.field_types(),
        SchemaContext::with_state(state),
    )?;

    execute_find_many_rows(state, model, query_args, tx_id.as_deref()).await
}

pub(in crate::handlers) async fn execute_find_many_typed(
    state: &EngineState,
    model_name: &str,
    args: &nautilus_core::FindManyArgs,
    transaction_id: Option<&str>,
) -> Result<Vec<Row>, ProtocolError> {
    let model = get_model_or_error(state, model_name)?;
    let metadata = state.model_metadata(model);
    let query_args = QueryArgs::from_find_many_args(args, metadata.field_types())?;

    execute_find_many_rows(state, model, query_args, transaction_id).await
}

/// Handle `query.findMany`.
///
/// Builds a SELECT for the requested model, applying optional `where`, `orderBy`,
/// `take`, `skip`, `cursor`, `distinct`, `select`, and `include` arguments.
/// Relation includes are hydrated after the parent rows load so child ordering
/// and pagination execute on the related query before JSON serialization.
/// Returns `QueryResult { data: [...] }`. Supports transactional execution via `transactionId`.
///
/// When the client sets `chunkSize` and the dispatcher provided a response
/// channel, the reply is chunked: a request [`stream::is_streamable`] accepts
/// emits its chunks as rows arrive, the rest are chunked after the fact.
pub(in crate::handlers) async fn handle_find_many(
    state: &EngineState,
    request: RpcRequest,
    sender: Option<mpsc::Sender<RpcResponse>>,
) -> Result<Box<RawValue>, ProtocolError> {
    let params: FindManyParams = parse_params(&request, "findMany")?;

    find_many_with_params(state, params, request.id, sender).await
}

/// Typed `findMany` entry point shared by [`handle_find_many`] and
/// [`handle_find_first`], so callers that already hold a [`FindManyParams`]
/// skip the JSON round-trip through a synthetic [`RpcRequest`].
async fn find_many_with_params(
    state: &EngineState,
    params: FindManyParams,
    request_id: Option<RpcId>,
    sender: Option<mpsc::Sender<RpcResponse>>,
) -> Result<Box<RawValue>, ProtocolError> {
    if params.chunk_size == Some(0) {
        return Err(ProtocolError::InvalidParams(
            "chunkSize must be at least 1".to_string(),
        ));
    }
    let chunk_size = params.chunk_size;
    check_protocol_version(params.protocol_version)?;
    let tx_id = params.transaction_id;
    let model = get_model_or_error(state, &params.model)?;
    let metadata = state.model_metadata(model);
    let relation_map = state.relation_map_for_model(model)?;
    let query_args = QueryArgs::parse_with_context(
        params.args,
        relation_map,
        metadata.field_types(),
        SchemaContext::with_state(state),
    )?;

    let streamable =
        chunk_size.is_some() && sender.is_some() && stream::is_streamable(state, &query_args);

    if streamable {
        let plan = build_find_many_plan(state, model, query_args)?;
        return stream::stream_find_many_chunked(
            state,
            plan,
            tx_id.as_deref(),
            chunk_size.expect("checked above"),
            request_id,
            sender.expect("checked above"),
        )
        .await;
    }

    let rows = execute_find_many_rows(state, model, query_args, tx_id.as_deref()).await?;

    if let (Some(size), Some(channel)) = (chunk_size, sender) {
        if let Some(last) = stream::emit_buffered_chunks(&rows, size, request_id, channel).await? {
            return Ok(last);
        }
    }

    wrap_data_result(&rows, "findMany result")
}

pub(in crate::handlers) async fn handle_find_many_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: FindManyParams = parse_params(&request, "findMany")?;
    execute_find_many_params(state, params).await
}

/// Handle `query.findFirst` and delegate to [`find_many_with_params`] with `take=1`.
pub(in crate::handlers) async fn handle_find_first(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<RawValue>, ProtocolError> {
    let params: FindFirstParams = parse_params(&request, "findFirst")?;

    let find_many_params = FindManyParams {
        protocol_version: params.protocol_version,
        model: params.model,
        args: params
            .args
            .map(|mut value| {
                if let serde_json::Value::Object(ref mut map) = value {
                    map.insert("take".into(), serde_json::json!(1));
                }
                value
            })
            .or_else(|| Some(serde_json::json!({ "take": 1 }))),
        transaction_id: params.transaction_id,
        chunk_size: None,
    };

    find_many_with_params(state, find_many_params, request.id, None).await
}

/// Handle `query.findFirstOrThrow`.
pub(in crate::handlers) async fn handle_find_first_or_throw(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<RawValue>, ProtocolError> {
    let raw = handle_find_first(state, request).await?;
    let parsed: serde_json::Value = serde_json::from_str(raw.get())
        .map_err(|e| ProtocolError::Internal(format!("Failed to parse result: {}", e)))?;
    let is_empty = parsed
        .get("data")
        .and_then(|value| value.as_array())
        .is_none_or(|array| array.is_empty());
    if is_empty {
        return Err(ProtocolError::RecordNotFound(
            "findFirstOrThrow: no record found matching the given filter".to_string(),
        ));
    }
    Ok(raw)
}
