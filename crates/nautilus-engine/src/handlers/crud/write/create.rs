//! `query.create`: one row, plus whatever the nested writes in its `data`
//! hang off it.
//!
//! A create with no relation entries is a single `INSERT`. One with them runs
//! inside a transaction instead, because the children need the parent's key
//! and the whole thing has to roll back together.

use nautilus_connector::Row;
use nautilus_core::{Insert, InsertCapacity};
use nautilus_protocol::{CreateParams, ProtocolError, RpcRequest};
use nautilus_schema::ir::ModelIr;
use serde_json::Value as JsonValue;

use super::input::{ensure_known_data_keys, insert_columns};
use super::read_back::inserted_row_filter;
use crate::conversion::check_protocol_version;
use crate::handlers::crud::common::{
    execute_mutation_result, wrap_count_result, wrap_mutation_result, MutationResultData,
};
use crate::handlers::crud::nested;
use crate::handlers::crud::read::find_rows_by_expr;
use crate::handlers::{get_writable_model_or_error, parse_params};
use crate::state::EngineState;

/// Insert one row of `model`, reading it back afterwards on a backend without
/// `RETURNING`.
async fn insert_row(
    state: &EngineState,
    model: &ModelIr,
    data: &JsonValue,
    return_data: bool,
    tx_id: Option<&str>,
) -> Result<MutationResultData, ProtocolError> {
    let metadata = state.model_metadata(model);

    let data_obj = data
        .as_object()
        .ok_or_else(|| ProtocolError::InvalidParams("data must be an object".to_string()))?;
    ensure_known_data_keys(model, data_obj)?;

    let (columns, values) = insert_columns(state, model, data_obj)?;

    let returns_inline = return_data && state.dialect.supports_returning();

    let mut builder = Insert::into_table(crate::metadata::model_table(model))
        .with_capacity(InsertCapacity {
            columns: columns.len(),
            rows: 1,
            returning: usize::from(returns_inline) * metadata.scalar_markers().len(),
        })
        .columns(columns)
        .values(values);
    if returns_inline {
        builder = builder.returning(metadata.scalar_markers().to_vec());
    }

    let insert = builder
        .build()
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to build insert: {}", e)))?;

    let sql = state
        .dialect
        .render_insert_owned(insert)
        .map_err(|e| ProtocolError::QueryPlanning(format!("Failed to render SQL: {}", e)))?;

    if returns_inline || !return_data {
        return execute_mutation_result(
            state,
            &sql,
            "Insert",
            tx_id,
            metadata.scalar_hints(),
            returns_inline,
        )
        .await;
    }

    let filter = inserted_row_filter(state, model, data_obj)?;

    state
        .in_transaction(tx_id, |tx| async move {
            state.execute_affected_on(&sql, "Insert", Some(&tx)).await?;
            let rows = find_rows_by_expr(state, model, Some(filter), Some(1), Some(&tx)).await?;
            Ok(MutationResultData::Rows(rows))
        })
        .await
}

pub(in crate::handlers::crud) async fn execute_create(
    state: &EngineState,
    params: CreateParams,
) -> Result<MutationResultData, ProtocolError> {
    check_protocol_version(params.protocol_version)?;
    let model = get_writable_model_or_error(state, &params.model)?;
    let plan = nested::split(state, model, &params.data, false)?;

    if plan.is_empty() {
        return insert_row(
            state,
            model,
            &params.data,
            params.return_data,
            params.transaction_id.as_deref(),
        )
        .await;
    }

    let return_data = params.return_data;
    let needs_row = return_data || plan.writes_children();

    state
        .in_transaction(params.transaction_id.as_deref(), |tx| async move {
            let (data, deferred) =
                nested::prepare_parent_data(state, model, &plan, None, &tx).await?;
            let result = insert_row(state, model, &data, needs_row, Some(&tx)).await?;

            if plan.writes_children() {
                let parent = result.first_row().ok_or_else(|| {
                    ProtocolError::Internal(
                        "create could not read back the row its nested writes hang from"
                            .to_string(),
                    )
                })?;
                nested::apply_children(state, model, &plan, parent, &tx).await?;
            }
            deferred.run(state, &tx).await?;

            Ok(if return_data {
                result
            } else {
                MutationResultData::Count(1)
            })
        })
        .await
}

/// Handle `query.create`.
pub(in crate::handlers) async fn handle_create(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Box<serde_json::value::RawValue>, ProtocolError> {
    let params: CreateParams = parse_params(&request, "create")?;

    match execute_create(state, params).await? {
        MutationResultData::Rows(rows) => wrap_mutation_result(&rows, "create result"),
        MutationResultData::Count(count) => wrap_count_result(count, "create result"),
    }
}

pub(in crate::handlers) async fn handle_create_embedded(
    state: &EngineState,
    request: RpcRequest,
) -> Result<Vec<Row>, ProtocolError> {
    let params: CreateParams = parse_params(&request, "create")?;
    execute_create(state, params).await?.into_rows("create")
}

pub(in crate::handlers) async fn handle_create_typed(
    state: &EngineState,
    params: CreateParams,
) -> Result<Vec<Row>, ProtocolError> {
    execute_create(state, params).await?.into_rows("create")
}
