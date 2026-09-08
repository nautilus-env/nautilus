//! `query.update` and `query.updateMany`: assignments over the rows a filter
//! matches.
//!
//! `updateMany` is the same operation with `return_data` pinned off, so the
//! `RETURNING` projection is never emitted and the answer is the affected-row
//! count alone. An update carrying nested writes runs in a transaction and
//! needs a filter matching exactly one row, since the children are linked to
//! one parent key.

use nautilus_connector::Row;
use nautilus_core::{Update, UpdateCapacity};
use nautilus_protocol::{
    check_protocol_version, ProtocolError, RpcRequest, UpdateManyParams, UpdateParams,
};
use nautilus_schema::ir::ModelIr;
use serde_json::Value as JsonValue;

use super::input::{ensure_known_data_keys, update_assignments};
use super::read_back::updated_rows_filter;
use crate::handlers::crud::common::{
    ensure_single_record_filter, execute_mutation_result, parse_optional_model_filter,
    wrap_count_result, wrap_mutation_result, MutationResultData,
};
use crate::handlers::crud::nested;
use crate::handlers::crud::read::{find_rows_by_expr, find_rows_by_filter};
use crate::handlers::{get_writable_model_or_error, parse_params};
use crate::state::EngineState;

/// Apply one set of column assignments, reading the affected rows back
/// afterwards on a backend without `RETURNING`.
async fn update_rows(
    state: &EngineState,
    model: &ModelIr,
    filter: &JsonValue,
    data: &JsonValue,
    return_data: bool,
    tx_id: Option<&str>,
) -> Result<MutationResultData, ProtocolError> {
    let metadata = state.model_metadata(model);

    let qualified_filter = parse_optional_model_filter(
        model,
        filter,
        metadata.field_types(),
        metadata.logical_to_db(),
    )?;

    let data_obj = data
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data must be an object".to_string()))?;
    ensure_known_data_keys(model, data_obj)?;

    let assignments = update_assignments(state, model, data_obj)?;

    let returns_inline = return_data && state.dialect.supports_returning();

    let mut builder = Update::table(crate::metadata::model_table(model))
        .with_capacity(UpdateCapacity {
            assignments: assignments.len(),
            returning: usize::from(returns_inline) * metadata.scalar_markers().len(),
        })
        .assignments(assignments);

    if let Some(filter) = qualified_filter.clone() {
        builder = builder.filter(filter);
    }

    if returns_inline {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let update = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build update: {}", e)))?;

    let sql = state
        .dialect
        .render_update_owned(update)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if returns_inline || !return_data {
        return execute_mutation_result(
            state,
            &sql,
            "Update",
            tx_id,
            metadata.scalar_hints(),
            returns_inline,
        )
        .await;
    }

    state
        .in_transaction(tx_id, |tx| async move {
            let before = find_rows_by_expr(state, model, qualified_filter, None, Some(&tx)).await?;
            state.execute_affected_on(&sql, "Update", Some(&tx)).await?;

            let Some(key_filter) = updated_rows_filter(state, model, &before, data_obj)? else {
                return Ok(MutationResultData::Rows(Vec::new()));
            };
            let rows = find_rows_by_expr(state, model, Some(key_filter), None, Some(&tx)).await?;
            Ok(MutationResultData::Rows(rows))
        })
        .await
}

/// Load the one row a nested write hangs off.
///
/// Nested operations link related rows to a specific parent, so the filter has
/// to name exactly one: a filter matching several offers no single key to point
/// children at.
async fn single_nested_target(
    state: &EngineState,
    model: &ModelIr,
    filter: &JsonValue,
    tx: &str,
) -> Result<Row, ProtocolError> {
    let mut rows = find_rows_by_filter(state, model, filter, 2, Some(tx)).await?;

    if rows.len() > 1 {
        return Err(ProtocolError::InvalidFilter(format!(
            "An update on '{}' that writes relations needs a filter matching exactly one row, and this one matches several",
            model.logical_name
        )));
    }

    rows.pop().ok_or_else(|| {
        ProtocolError::RecordNotFound(format!(
            "update on '{}' matched no record to write relations against",
            model.logical_name
        ))
    })
}

pub(in crate::handlers::crud) async fn execute_update(
    state: &EngineState,
    params: UpdateParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let model = get_writable_model_or_error(state, &params.model)?;
    let plan = nested::split(state, model, &params.data, true)?;

    if plan.is_empty() {
        return update_rows(
            state,
            model,
            &params.filter,
            &params.data,
            params.return_data,
            params.transaction_id.as_deref(),
        )
        .await;
    }

    let return_data = params.return_data;

    state
        .in_transaction(params.transaction_id.as_deref(), |tx| async move {
            let current = single_nested_target(state, model, &params.filter, &tx).await?;
            let (data, deferred) =
                nested::prepare_parent_data(state, model, &plan, Some(&current), &tx).await?;

            let writes_columns = data.as_object().is_some_and(|obj| !obj.is_empty());
            let updated = if writes_columns {
                Some(update_rows(state, model, &params.filter, &data, true, Some(&tx)).await?)
            } else {
                None
            };

            let parent = updated
                .as_ref()
                .and_then(MutationResultData::first_row)
                .unwrap_or(&current);
            nested::apply_children(state, model, &plan, parent, &tx).await?;
            deferred.run(state, &tx).await?;

            if !return_data {
                return Ok(MutationResultData::Count(1));
            }
            Ok(updated.unwrap_or_else(|| MutationResultData::Rows(vec![current])))
        })
        .await
}

/// Handle `query.update`.
pub(in crate::handlers) async fn handle_update(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: UpdateParams = parse_params(&request, "update")?;
    ensure_single_record_filter("update", &params.filter)?;

    match execute_update(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "update result"),
        MutationResultData::Count(count) => wrap_count_result(count, "update result"),
    }
}

pub(in crate::handlers) async fn handle_update_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: UpdateParams = parse_params(&request, "update")?;
    ensure_single_record_filter("update", &params.filter)?;
    execute_update(state, params).await?.into_rows("update")
}

pub(in crate::handlers) async fn handle_update_typed(
    state: &EngineState,
    params: UpdateParams,
) -> Result<Vec<Row>, ProtocolError> {
    ensure_single_record_filter("update", &params.filter)?;
    execute_update(state, params).await?.into_rows("update")
}

/// Handle `query.updateMany`.
///
/// Reuses the `query.update` statement builder with `return_data` pinned off,
/// so the model's `RETURNING` projection is never emitted and the result is the
/// affected-row count alone.
pub(in crate::handlers) async fn handle_update_many(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: UpdateManyParams = parse_params(&request, "updateMany")?;

    wrap_count_result(
        execute_update_many(state, params).await?,
        "updateMany result",
    )
}

pub(in crate::handlers) async fn handle_update_many_typed(
    state: &EngineState,
    params: UpdateManyParams,
) -> Result<usize, ProtocolError> {
    execute_update_many(state, params).await
}

async fn execute_update_many(
    state: &EngineState,
    params: UpdateManyParams,
) -> Result<usize, ProtocolError> {
    Ok(execute_update(
        state,
        UpdateParams {
            protocol_version: params.protocol_version,
            model: params.model,
            filter: params.filter,
            data: params.data,
            transaction_id: params.transaction_id,
            return_data: false,
        },
    )
    .await?
    .into_count())
}
